from __future__ import annotations

import errno
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager, redirect_stdout
from datetime import datetime, timedelta
from pathlib import Path
from unittest.mock import patch

import albert_mvp.core as core_module
from albert_mvp.cli import main
from albert_mvp.core import AlbertError, AlbertMission, LaunchBlockedError
from albert_mvp.retirement import RetirementSnapshotError, RetirementSnapshotStore
from albert_mvp.server import serve
from albert_mvp.workspace import WorkspaceSnapshotService

ISSUE_BODY = """Status: ready-for-agent
Type: AFK
Risk: High
Suggested agent: fake-local
Assigned agent: fake-local

## Parent

PRD.md

## What to build

Preserve one Retirement Unit.

## Acceptance criteria

- [ ] Preservation is verified before retirement.

## Blocked by

None - can start immediately
"""


class RetirementPreservationTest(unittest.TestCase):
    def setUp(self) -> None:
        self.process_isolation = patch(
            "albert_mvp.core._process_isolated_argv",
            side_effect=lambda argv: argv,
        )
        self.process_isolation.start()
        self.addCleanup(self.process_isolation.stop)
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.target_repo = self.root / "target"
        self.target_repo.mkdir()
        self._git("init")
        self._git("config", "user.name", "Retirement Test")
        self._git("config", "user.email", "retirement@example.invalid")
        (self.target_repo / "tracked.txt").write_text("baseline\n", encoding="utf-8")
        (self.target_repo / "unstaged.txt").write_text("baseline\n", encoding="utf-8")
        self._git("add", ".")
        self._git("commit", "-m", "baseline")

        self.tracker = self.root / "tracker"
        (self.tracker / "issues").mkdir(parents=True)
        (self.tracker / "PRD.md").write_text("# Retirement PRD\n", encoding="utf-8")
        (self.tracker / "issues" / "01-retirement.md").write_text(
            ISSUE_BODY,
            encoding="utf-8",
        )
        self.agent_config = self.root / "agents.json"
        self.agent_config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "fake-local",
                            "role": "local-agent",
                            "provider": "test-harness",
                            "runner": "fake",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        self.runtime = self.root / "runtime"

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _git(
        self, *arguments: str, cwd: Path | None = None
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", "-C", str(cwd or self.target_repo), *arguments],
            check=True,
            capture_output=True,
            text=True,
        )

    @staticmethod
    def _stop_process(process: subprocess.Popen[str]) -> None:
        try:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
        finally:
            if process.stdout is not None:
                process.stdout.close()
            if process.stderr is not None:
                process.stderr.close()

    def load_mission(
        self,
        *,
        quiescence: tuple[str, str] | None = ("absent", "absent"),
        retention_grace_seconds: int = 72 * 60 * 60,
        snapshot_storage_retention_seconds: int = 30 * 24 * 60 * 60,
        snapshot_storage_budget_bytes: int = 5 * 1024 * 1024 * 1024,
        perform_startup_effects: bool = True,
    ) -> AlbertMission:
        options = {}
        if quiescence is not None:
            options["retirement_quiescence_probe"] = lambda _boundary: quiescence
        return AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="mission-retirement",
            agent_config_path=self.agent_config,
            retention_grace_seconds=retention_grace_seconds,
            snapshot_storage_retention_seconds=snapshot_storage_retention_seconds,
            snapshot_storage_budget_bytes=snapshot_storage_budget_bytes,
            **options,
        ).load(perform_startup_effects=perform_startup_effects)

    def completed_session(self, mission: AlbertMission):
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        launched = mission.launch_issue("ISS-01")
        return mission.run_session(launched.session_id)

    def add_issue(self, sequence: int) -> str:
        issue_id = f"ISS-{sequence:02d}"
        (self.tracker / "issues" / f"{sequence:02d}-retirement.md").write_text(
            ISSUE_BODY,
            encoding="utf-8",
        )
        return issue_id

    def completed_issue(self, mission: AlbertMission, issue_id: str):
        mission.assign_issue(issue_id, "fake-local")
        mission.approve_issue(issue_id)
        launched = mission.launch_issue(issue_id)
        return mission.run_session(launched.session_id)

    def retirement_blocked_snapshot(self, mission: AlbertMission, completed):
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create one blocked snapshot-backed export.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        return mission._refresh_persisted_session(completed.session_id)

    def persist_legacy_export_intent(
        self,
        mission: AlbertMission,
        blocked,
        *,
        destination: Path,
        correlation_id: str,
        export_kind: str,
        manifest_sha256: str,
    ) -> dict[str, object]:
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        raw_session = runtime["sessions"][blocked.session_id]
        claim_revision = blocked.revision + 1
        raw_session["revision"] = claim_revision
        intent = {
            "claim_revision": claim_revision,
            "correlation_id": correlation_id,
            "destination": str(destination.resolve(strict=False)),
            "expected_revision": blocked.revision,
            "export_kind": export_kind,
            "manifest_sha256": manifest_sha256,
        }
        raw_session["retirement"]["export_intent"] = intent
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        return intent

    def write_legacy_export_marker(
        self,
        mission: AlbertMission,
        blocked,
        *,
        destination: Path,
        correlation_id: str,
        export_kind: str,
        manifest_sha256: str,
    ) -> Path:
        marker_path = destination / "retirement-export.json"
        marker_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "mission_id": mission.mission_id,
                    "session_id": blocked.session_id,
                    "correlation_id": correlation_id,
                    "expected_revision": blocked.revision,
                    "manifest_sha256": manifest_sha256,
                    "export_kind": export_kind,
                    "exported_at": "2026-08-11T12:00:00+00:00",
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        return marker_path

    def test_snapshot_payload_defaults_and_pinning_are_durable(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-for-storage-policy",
        )

        record = preserved.retirement["snapshot"]
        created = datetime.fromisoformat(record["created_at"])
        expires = datetime.fromisoformat(record["expires_at"])
        self.assertEqual(expires - created, timedelta(days=30))
        self.assertFalse(record["pinned"])
        self.assertEqual(record["payload_disposition"], "retained")
        self.assertEqual(record["mission_id"], mission.mission_id)
        self.assertEqual(record["session_id"], completed.session_id)
        self.assertEqual(record["terminal_status"], completed.status)

        pinned = mission.set_retirement_snapshot_pin(
            completed.session_id,
            pinned=True,
            expected_revision=preserved.revision,
            correlation_id="pin-storage-payload",
        )

        self.assertTrue(pinned.retirement["snapshot"]["pinned"])
        reloaded = self.load_mission().sessions[completed.session_id]
        self.assertTrue(reloaded.retirement["snapshot"]["pinned"])
        self.assertEqual(
            reloaded.retirement["snapshot"]["manifest_sha256"],
            record["manifest_sha256"],
        )

    def test_legacy_verified_snapshot_can_be_inspected_and_pinned(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-before-storage-schema",
        )
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        legacy_snapshot = runtime["sessions"][preserved.session_id]["retirement"][
            "snapshot"
        ]
        for field_name in (
            "mission_id",
            "session_id",
            "terminal_status",
            "created_at",
            "expires_at",
            "pinned",
            "payload_disposition",
            "reclaimed_at",
            "reclamation_reason",
        ):
            legacy_snapshot.pop(field_name)
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )

        compatible = self.load_mission()
        inspection = compatible.retirement_storage_inspection()
        self.assertEqual(inspection["counts"]["retained_payloads"], 1)
        self.assertEqual(inspection["counts"]["pinned_payloads"], 0)
        pinned = compatible.set_retirement_snapshot_pin(
            completed.session_id,
            pinned=True,
            expected_revision=preserved.revision,
            correlation_id="pin-legacy-storage-payload",
        )

        self.assertTrue(pinned.retirement["snapshot"]["pinned"])
        reloaded = self.load_mission().sessions[completed.session_id]
        self.assertTrue(reloaded.retirement["snapshot"]["pinned"])
        self.assertEqual(reloaded.retirement["snapshot"]["mission_id"], mission.mission_id)
        self.assertTrue(reloaded.retirement["snapshot"]["expires_at"])

    def test_snapshot_pin_exact_replay_preserves_newer_pin_state(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-for-pin-replay",
        )
        pinned = mission.set_retirement_snapshot_pin(
            completed.session_id,
            pinned=True,
            expected_revision=preserved.revision,
            correlation_id="pin-replay-boundary",
        )
        unpinned = mission.set_retirement_snapshot_pin(
            completed.session_id,
            pinned=False,
            expected_revision=pinned.revision,
            correlation_id="newer-unpin-boundary",
        )

        replayed = mission.set_retirement_snapshot_pin(
            completed.session_id,
            pinned=True,
            expected_revision=preserved.revision,
            correlation_id="pin-replay-boundary",
        )

        self.assertEqual(replayed.revision, unpinned.revision)
        self.assertFalse(replayed.retirement["snapshot"]["pinned"])

    def test_unpin_exact_replay_never_reclaims_newly_expired_payload(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-for-unpin-replay",
        )
        pinned = mission.set_retirement_snapshot_pin(
            completed.session_id,
            pinned=True,
            expected_revision=preserved.revision,
            correlation_id="pin-before-unpin-replay",
        )
        unpinned = mission.set_retirement_snapshot_pin(
            completed.session_id,
            pinned=False,
            expected_revision=pinned.revision,
            correlation_id="unpin-replay-boundary",
        )
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        snapshot = runtime["sessions"][completed.session_id]["retirement"][
            "snapshot"
        ]
        snapshot["created_at"] = (
            datetime.now().astimezone() - timedelta(days=2)
        ).isoformat()
        snapshot["expires_at"] = (
            datetime.now().astimezone() - timedelta(seconds=1)
        ).isoformat()
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        before = mission.runtime_path.read_bytes()

        replayed = mission.set_retirement_snapshot_pin(
            completed.session_id,
            pinned=False,
            expected_revision=pinned.revision,
            correlation_id="unpin-replay-boundary",
        )

        self.assertEqual(replayed.revision, unpinned.revision)
        self.assertEqual(mission.runtime_path.read_bytes(), before)
        self.assertTrue(Path(snapshot["payload_path"]).is_dir())

    def test_expired_unpinned_payloads_reclaim_oldest_first_and_keep_records(self) -> None:
        second_issue = self.add_issue(2)
        third_issue = self.add_issue(3)
        mission = self.load_mission()
        first = self.completed_issue(mission, "ISS-01")
        mission.record_frontier_review(
            first.session_id,
            "Approved",
            reason="Retire the first payload for storage testing.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=first.revision,
        )
        second = self.completed_issue(mission, second_issue)
        mission.record_frontier_review(
            second.session_id,
            "Approved",
            reason="Retire the second payload for storage testing.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=second.revision,
        )
        first_record = mission._refresh_persisted_session(first.session_id).retirement[
            "snapshot"
        ]
        second_record = mission._refresh_persisted_session(second.session_id).retirement[
            "snapshot"
        ]
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        first_snapshot = runtime["sessions"][first.session_id]["retirement"][
            "snapshot"
        ]
        first_snapshot["created_at"] = (
            datetime.now().astimezone() - timedelta(days=2)
        ).isoformat()
        first_snapshot["expires_at"] = (
            datetime.now().astimezone() - timedelta(seconds=1)
        ).isoformat()
        second_snapshot = runtime["sessions"][second.session_id]["retirement"][
            "snapshot"
        ]
        second_snapshot["created_at"] = (
            datetime.now().astimezone() - timedelta(days=1)
        ).isoformat()
        second_snapshot["expires_at"] = (
            datetime.now().astimezone() - timedelta(seconds=1)
        ).isoformat()
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        budget = 32 * 1024 * 1024 + max(
            first_record["snapshot_bytes"], second_record["snapshot_bytes"]
        )

        constrained = self.load_mission(
            snapshot_storage_budget_bytes=budget,
            perform_startup_effects=False,
        )
        constrained.assign_issue(third_issue, "fake-local")
        constrained.approve_issue(third_issue)
        constrained.launch_issue(third_issue)

        reclaimed_first = constrained._refresh_persisted_session(
            first.session_id
        ).retirement["snapshot"]
        retained_second = constrained._refresh_persisted_session(
            second.session_id
        ).retirement["snapshot"]
        self.assertEqual(reclaimed_first["payload_disposition"], "reclaimed")
        self.assertTrue(reclaimed_first["reclaimed_at"])
        self.assertFalse(Path(first_record["payload_path"]).exists())
        self.assertEqual(retained_second["payload_disposition"], "retained")
        self.assertTrue(Path(second_record["payload_path"]).is_dir())
        self.assertEqual(
            reclaimed_first["manifest_sha256"], first_record["manifest_sha256"]
        )
        self.assertEqual(reclaimed_first["worktree_identity"], first.worktree_identity)
        inspection = constrained.retirement_storage_inspection()
        self.assertEqual(inspection["reclamation"]["count"], 1)
        self.assertEqual(
            inspection["reclamation"]["recent"][0]["session_id"],
            first.session_id,
        )

    def test_expired_grace_and_retirement_blocked_payloads_remain_protected(
        self,
    ) -> None:
        second_issue = self.add_issue(2)
        third_issue = self.add_issue(3)
        mission = self.load_mission()
        grace_source = self.completed_issue(mission, "ISS-01")
        mission.record_frontier_review(
            grace_source.session_id,
            "Approved",
            reason="Create the first retained payload before protecting it in grace.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=grace_source.revision,
        )
        blocked_source = self.completed_issue(mission, second_issue)
        mission.record_frontier_review(
            blocked_source.session_id,
            "Approved",
            reason="Create the second retained payload before blocking retirement.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=blocked_source.revision,
        )
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        now = datetime.now().astimezone()
        protected = (
            (grace_source.session_id, "grace"),
            (blocked_source.session_id, "retirement-blocked"),
        )
        payload_paths: list[Path] = []
        for session_id, phase in protected:
            retirement = runtime["sessions"][session_id]["retirement"]
            snapshot = retirement["snapshot"]
            snapshot["created_at"] = (now - timedelta(days=2)).isoformat()
            snapshot["expires_at"] = (now - timedelta(seconds=1)).isoformat()
            retirement["phase"] = phase
            if phase == "grace":
                retirement["grace_started_at"] = now.isoformat()
                retirement["grace_expires_at"] = (
                    now + timedelta(days=1)
                ).isoformat()
            else:
                retirement["retirement_attempts"] = 3
                retirement["blocked_reason"] = "simulated exact removal blocker"
            payload_paths.append(Path(snapshot["payload_path"]))
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )

        constrained = self.load_mission(
            snapshot_storage_budget_bytes=32 * 1024 * 1024,
            perform_startup_effects=False,
        )
        constrained.assign_issue(third_issue, "fake-local")
        constrained.approve_issue(third_issue)

        with self.assertRaisesRegex(
            LaunchBlockedError,
            "Storage Budget is exhausted",
        ):
            constrained.launch_issue(third_issue)

        for (session_id, phase), payload_path in zip(protected, payload_paths):
            with self.subTest(phase=phase):
                retained = constrained._refresh_persisted_session(session_id)
                self.assertEqual(retained.retirement["phase"], phase)
                self.assertEqual(
                    retained.retirement["snapshot"]["payload_disposition"],
                    "retained",
                )
                self.assertTrue(payload_path.is_dir())
        inspection = constrained.retirement_storage_inspection()
        self.assertEqual(inspection["counts"]["expired_eligible_payloads"], 0)
        self.assertEqual(
            inspection["blockers"][0]["code"],
            "snapshot-storage-exhausted",
        )

    def test_pinned_capacity_exhaustion_blocks_new_units_and_raises_attention(
        self,
    ) -> None:
        second_issue = self.add_issue(2)
        mission = self.load_mission()
        first = self.completed_issue(mission, "ISS-01")
        mission.record_frontier_review(
            first.session_id,
            "Approved",
            reason="Retire the payload before pinning it.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=first.revision,
        )
        retired = mission._refresh_persisted_session(first.session_id)
        pinned = mission.set_retirement_snapshot_pin(
            first.session_id,
            pinned=True,
            expected_revision=retired.revision,
            correlation_id="pin-protected-payload",
        )
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        pinned_snapshot = runtime["sessions"][first.session_id]["retirement"][
            "snapshot"
        ]
        pinned_snapshot["created_at"] = (
            datetime.now().astimezone() - timedelta(days=2)
        ).isoformat()
        pinned_snapshot["expires_at"] = (
            datetime.now().astimezone() - timedelta(seconds=1)
        ).isoformat()
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        constrained = self.load_mission(
            snapshot_storage_retention_seconds=0,
            snapshot_storage_budget_bytes=32 * 1024 * 1024,
        )
        constrained.assign_issue(second_issue, "fake-local")
        constrained.approve_issue(second_issue)

        with self.assertRaisesRegex(LaunchBlockedError, "Storage Budget is exhausted"):
            constrained.launch_issue(second_issue)

        inspection = constrained.retirement_storage_inspection()
        self.assertEqual(inspection["counts"]["pinned_payloads"], 1)
        self.assertEqual(
            inspection["blockers"][0]["code"], "snapshot-storage-exhausted"
        )
        self.assertEqual(
            constrained._refresh_persisted_session(first.session_id).retirement[
                "snapshot"
            ]["manifest_sha256"],
            pinned.retirement["snapshot"]["manifest_sha256"],
        )
        mission_summary = WorkspaceSnapshotService(constrained).snapshot().missions[0]
        storage_attention = next(
            item
            for item in mission_summary.attention
            if item.kind == "retirement-storage"
        )
        self.assertIn("Storage Budget is exhausted", storage_attention.label)

        unpinned = constrained.set_retirement_snapshot_pin(
            first.session_id,
            pinned=False,
            expected_revision=pinned.revision,
            correlation_id="unpin-after-storage-exhaustion",
        )
        self.assertFalse(unpinned.retirement["snapshot"]["pinned"])
        resolved = constrained.retirement_storage_inspection()
        self.assertEqual(resolved["blockers"], [])
        self.assertEqual(resolved["counts"]["retained_payloads"], 1)
        self.assertTrue(Path(pinned_snapshot["payload_path"]).is_dir())

        launched = constrained.launch_issue(second_issue)
        self.assertTrue(launched.preservation_budget["bound"])
        reclaimed = constrained._refresh_persisted_session(first.session_id)
        self.assertEqual(
            reclaimed.retirement["snapshot"]["payload_disposition"],
            "reclaimed",
        )

    def test_expired_unpinned_payload_reclaims_during_mission_startup(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        mission.record_frontier_review(
            completed.session_id,
            "Approved",
            reason="Retire one payload before its retention deadline.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=completed.revision,
        )
        retired = mission._refresh_persisted_session(completed.session_id)
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        expired_snapshot = runtime["sessions"][completed.session_id]["retirement"][
            "snapshot"
        ]
        expired_snapshot["created_at"] = (
            datetime.now().astimezone() - timedelta(days=2)
        ).isoformat()
        expired_snapshot["expires_at"] = (
            datetime.now().astimezone() - timedelta(seconds=1)
        ).isoformat()
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )

        reloaded = self.load_mission()
        compact = reloaded.sessions[completed.session_id].retirement["snapshot"]
        self.assertEqual(compact["payload_disposition"], "reclaimed")
        self.assertFalse(Path(retired.retirement["snapshot"]["payload_path"]).exists())

    def test_storage_inspection_is_read_only_for_newly_expired_payloads(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        mission.record_frontier_review(
            completed.session_id,
            "Approved",
            reason="Retire one payload before checking the read-only storage view.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=completed.revision,
        )
        retired = mission._refresh_persisted_session(completed.session_id)
        payload_path = Path(retired.retirement["snapshot"]["payload_path"])
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        runtime["sessions"][completed.session_id]["retirement"]["snapshot"]["created_at"] = (
            datetime.now().astimezone() - timedelta(days=2)
        ).isoformat()
        runtime["sessions"][completed.session_id]["retirement"]["snapshot"]["expires_at"] = (
            datetime.now().astimezone() - timedelta(seconds=1)
        ).isoformat()
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        before = mission.runtime_path.read_bytes()

        inspection = mission.retirement_storage_inspection()

        self.assertEqual(mission.runtime_path.read_bytes(), before)
        self.assertTrue(payload_path.is_dir())
        self.assertEqual(inspection["counts"]["expired_eligible_payloads"], 1)
        self.assertEqual(inspection["counts"]["retained_payloads"], 1)

        common = [
            "retirement-storage",
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            mission.mission_id,
            "--agent-config",
            str(self.agent_config),
        ]
        cli_output = io.StringIO()
        with redirect_stdout(cli_output):
            self.assertEqual(main(common), 0)
        self.assertEqual(mission.runtime_path.read_bytes(), before)
        self.assertTrue(payload_path.is_dir())
        self.assertEqual(
            json.loads(cli_output.getvalue())["counts"][
                "expired_eligible_payloads"
            ],
            1,
        )

        server_output = io.StringIO()
        serve(
            io.StringIO(
                json.dumps(
                    {"id": "read-only-storage", "argv": common},
                )
                + "\n"
            ),
            server_output,
        )
        self.assertTrue(json.loads(server_output.getvalue())["success"])
        self.assertEqual(mission.runtime_path.read_bytes(), before)
        self.assertTrue(payload_path.is_dir())

    def test_storage_inspection_of_fresh_mission_does_not_create_runtime_state(
        self,
    ) -> None:
        fresh_runtime = self.root / "fresh-read-only-runtime"
        output = io.StringIO()

        with redirect_stdout(output):
            self.assertEqual(
                main(
                    [
                        "retirement-storage",
                        "--target-repo",
                        str(self.target_repo),
                        "--tracker-dir",
                        str(self.tracker),
                        "--runtime-root",
                        str(fresh_runtime),
                        "--mission-id",
                        "fresh-storage-inspection",
                        "--agent-config",
                        str(self.agent_config),
                    ]
                ),
                0,
            )

        inspection = json.loads(output.getvalue())
        self.assertEqual(inspection["counts"]["records"], 0)
        self.assertEqual(inspection["usage"]["committed_bytes"], 0)
        self.assertFalse(any(fresh_runtime.rglob("runtime.json")))

    def test_snapshot_storage_schema_rejects_malformed_durable_records(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-for-schema-validation",
        )
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        runtime["sessions"][preserved.session_id]["retirement"]["snapshot"][
            "pinned"
        ] = "yes"
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(AlbertError, "Snapshot Payload record is invalid"):
            self.load_mission()

    def test_retirement_storage_schema_rejects_malformed_aggregate_state(
        self,
    ) -> None:
        mission = self.load_mission()
        baseline = json.loads(mission.runtime_path.read_text(encoding="utf-8"))

        for corruption in (
            "boolean-schema",
            "malformed-reclamation",
            "string-attention-active",
        ):
            with self.subTest(corruption=corruption):
                runtime = json.loads(json.dumps(baseline))
                storage = runtime["retirement_storage"]
                if corruption == "boolean-schema":
                    storage["schema_version"] = True
                elif corruption == "malformed-reclamation":
                    storage["reclamation_count"] = 1
                    storage["recent_reclamations"] = [{}]
                else:
                    storage["attention"] = {
                        "active": "yes",
                        "code": "snapshot-storage-exhausted",
                        "message": "must not fabricate an active blocker",
                        "required_bytes": 1,
                        "committed_bytes": 1,
                        "budget_bytes": 1,
                        "recorded_at": datetime.now().astimezone().isoformat(),
                    }
                mission.runtime_path.write_text(
                    json.dumps(runtime, indent=2, sort_keys=True),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(
                    AlbertError,
                    "Snapshot storage state is invalid",
                ):
                    self.load_mission()

    def test_retirement_action_receipts_fail_closed_when_their_shape_is_malformed(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-before-malformed-action-receipt",
        )
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        runtime["sessions"][preserved.session_id]["retirement"][
            "action_receipts"
        ]["reused-correlation"] = {}
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(
            AlbertError,
            "Retirement Unit action receipt is invalid",
        ):
            self.load_mission()

    def test_legacy_six_field_export_intent_loads_and_replays_without_overwrite(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        second_issue = self.add_issue(2)
        mission = self.load_mission()
        first_completed = self.completed_session(mission)
        second_completed = self.completed_issue(mission, second_issue)
        first_blocked = self.retirement_blocked_snapshot(
            mission,
            first_completed,
        )
        second_blocked = self.retirement_blocked_snapshot(
            mission,
            second_completed,
        )
        first_destination = self.root / "legacy-absent-export"
        second_destination = self.root / "legacy-existing-export"
        sentinel = second_destination / "foreign-sentinel.txt"
        second_destination.mkdir()
        sentinel.write_text(
            "legacy destination must never be replaced\n",
            encoding="utf-8",
        )
        requests = (
            (
                first_completed.session_id,
                first_blocked.revision,
                first_destination,
                "replay-legacy-absent-export",
            ),
            (
                second_completed.session_id,
                second_blocked.revision,
                second_destination,
                "reject-legacy-existing-export",
            ),
        )
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        legacy_fields = {
            "claim_revision",
            "correlation_id",
            "destination",
            "expected_revision",
            "export_kind",
            "manifest_sha256",
        }
        for session_id, expected_revision, destination, correlation_id in requests:
            raw_session = runtime["sessions"][session_id]
            raw_session["revision"] = expected_revision + 1
            snapshot = raw_session["retirement"]["snapshot"]
            raw_session["retirement"]["export_intent"] = {
                "claim_revision": expected_revision + 1,
                "correlation_id": correlation_id,
                "destination": str(destination.resolve(strict=False)),
                "expected_revision": expected_revision,
                "export_kind": "snapshot-payload",
                "manifest_sha256": snapshot["manifest_sha256"],
            }
            self.assertEqual(
                set(raw_session["retirement"]["export_intent"]),
                legacy_fields,
            )
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )

        compatible = self.load_mission()
        self.assertEqual(
            compatible.sessions[first_completed.session_id].retirement[
                "export_intent"
            ]["correlation_id"],
            "replay-legacy-absent-export",
        )
        self.assertEqual(
            compatible.sessions[second_completed.session_id].retirement[
                "export_intent"
            ]["correlation_id"],
            "reject-legacy-existing-export",
        )

        receipt = compatible.export_retirement_unit(
            first_completed.session_id,
            destination=first_destination,
            expected_revision=first_blocked.revision,
            correlation_id="replay-legacy-absent-export",
        )

        self.assertEqual(receipt["action"], "export")
        self.assertEqual(
            receipt["destination"],
            str(first_destination.resolve(strict=True) / "repository"),
        )
        self.assertEqual(
            (first_destination / "repository" / "tracked.txt").read_text(
                encoding="utf-8"
            ),
            "baseline\n",
        )
        with self.assertRaisesRegex(
            AlbertError,
            "destination|legacy|publish|owner|boundary|exist|content",
        ):
            self.load_mission().export_retirement_unit(
                second_completed.session_id,
                destination=second_destination,
                expected_revision=second_blocked.revision,
                correlation_id="reject-legacy-existing-export",
            )

        self.assertEqual(
            sentinel.read_text(encoding="utf-8"),
            "legacy destination must never be replaced\n",
        )
        self.assertEqual(tuple(second_destination.iterdir()), (sentinel,))
        persisted = self.load_mission().sessions[second_completed.session_id]
        self.assertNotIn(
            "reject-legacy-existing-export",
            persisted.retirement.get("action_receipts", {}),
        )

    def test_complete_legacy_snapshot_export_is_adopted_without_marker_rewrite(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        snapshot = blocked.retirement["snapshot"]
        manifest_sha256 = snapshot["manifest_sha256"]
        destination = self.root / "complete-legacy-snapshot-export"
        destination.mkdir()
        store = mission._retirement_snapshot_store(
            blocked,
            snapshot["session_revision"],
        )
        store.materialize(snapshot, destination)
        correlation_id = "adopt-complete-legacy-snapshot-export"
        marker_path = self.write_legacy_export_marker(
            mission,
            blocked,
            destination=destination,
            correlation_id=correlation_id,
            export_kind="snapshot-payload",
            manifest_sha256=manifest_sha256,
        )
        self.persist_legacy_export_intent(
            mission,
            blocked,
            destination=destination,
            correlation_id=correlation_id,
            export_kind="snapshot-payload",
            manifest_sha256=manifest_sha256,
        )
        os.utime(marker_path, ns=(1_600_000_000_000_000_000,) * 2)
        marker_status = marker_path.stat(follow_symlinks=False)
        marker_before = (
            marker_status.st_dev,
            marker_status.st_ino,
            marker_status.st_size,
            marker_status.st_mtime_ns,
            marker_path.read_bytes(),
        )

        receipt = self.load_mission().export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id=correlation_id,
        )

        marker_status = marker_path.stat(follow_symlinks=False)
        self.assertEqual(
            (
                marker_status.st_dev,
                marker_status.st_ino,
                marker_status.st_size,
                marker_status.st_mtime_ns,
                marker_path.read_bytes(),
            ),
            marker_before,
        )
        self.assertEqual(receipt["manifest_sha256"], manifest_sha256)
        self.assertEqual(
            (destination / "repository" / "tracked.txt").read_text(
                encoding="utf-8"
            ),
            "baseline\n",
        )
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertEqual(persisted.retirement["export_intent"], {})
        self.assertEqual(
            persisted.retirement["action_receipts"][correlation_id],
            receipt,
        )

    def test_complete_legacy_retained_export_finalizes_after_source_removal(
        self,
    ) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        (completed.worktree_path / "legacy-direct.txt").write_text(
            "legacy direct retained content\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-legacy-direct-export",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        exclude_git_metadata = blocked.worktree_identity.startswith("managed-git:")
        retained_manifest = RetirementSnapshotStore.retained_worktree_manifest(
            blocked.worktree_path,
            exclude_git_metadata=exclude_git_metadata,
        )
        legacy_projection = (
            RetirementSnapshotStore.legacy_retained_worktree_manifest_projection(
                retained_manifest,
                exclude_git_metadata=exclude_git_metadata,
            )
        )
        manifest_sha256 = legacy_projection["tree_sha256"]
        self.assertNotEqual(
            manifest_sha256,
            retained_manifest["materialized_tree_sha256"],
        )
        destination = self.root / "complete-legacy-direct-export"
        destination.mkdir()
        RetirementSnapshotStore.materialize_retained_worktree(
            blocked.worktree_path,
            destination,
            retained_manifest,
            exclude_git_metadata=exclude_git_metadata,
        )
        correlation_id = "finalize-complete-legacy-direct-export"
        self.write_legacy_export_marker(
            mission,
            blocked,
            destination=destination,
            correlation_id=correlation_id,
            export_kind="retained-worktree",
            manifest_sha256=manifest_sha256,
        )
        self.persist_legacy_export_intent(
            mission,
            blocked,
            destination=destination,
            correlation_id=correlation_id,
            export_kind="retained-worktree",
            manifest_sha256=manifest_sha256,
        )
        shutil.rmtree(blocked.worktree_path)

        receipt = self.load_mission().export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id=correlation_id,
        )

        self.assertEqual(receipt["manifest_sha256"], manifest_sha256)
        self.assertEqual(
            (destination / "repository" / "legacy-direct.txt").read_text(
                encoding="utf-8"
            ),
            "legacy direct retained content\n",
        )
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertEqual(persisted.retirement["export_intent"], {})
        self.assertIn(correlation_id, persisted.retirement["action_receipts"])

    def test_markerless_legacy_export_is_untouched_without_owner_or_receipt(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        snapshot = blocked.retirement["snapshot"]
        destination = self.root / "markerless-legacy-export"
        destination.mkdir()
        store = mission._retirement_snapshot_store(
            blocked,
            snapshot["session_revision"],
        )
        store.materialize(snapshot, destination)
        correlation_id = "reject-markerless-legacy-export"
        intent = self.persist_legacy_export_intent(
            mission,
            blocked,
            destination=destination,
            correlation_id=correlation_id,
            export_kind="snapshot-payload",
            manifest_sha256=snapshot["manifest_sha256"],
        )

        def destination_bytes() -> dict[str, bytes]:
            return {
                str(path.relative_to(destination)): path.read_bytes()
                for path in destination.rglob("*")
                if path.is_file() and not path.is_symlink()
            }

        entries_before = tuple(
            sorted(str(path.relative_to(destination)) for path in destination.rglob("*"))
        )
        bytes_before = destination_bytes()
        exporter = self.load_mission()

        with self.assertRaisesRegex(AlbertError, "incomplete|changed"):
            exporter.export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id=correlation_id,
            )

        self.assertEqual(
            tuple(
                sorted(
                    str(path.relative_to(destination))
                    for path in destination.rglob("*")
                )
            ),
            entries_before,
        )
        self.assertEqual(destination_bytes(), bytes_before)
        self.assertFalse((destination / "retirement-export.json").exists())
        self.assertEqual(exporter._retirement_export_owner_directories(), [])
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertEqual(persisted.retirement["export_intent"], intent)
        self.assertNotIn(
            correlation_id,
            persisted.retirement.get("action_receipts", {}),
        )

    def test_reserved_export_stage_collision_replays_with_second_attempt(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "reserved-stage-collision-export"
        correlation_id = "replay-reserved-stage-collision"
        write_payload = mission._write_runtime_payload
        interrupted = False

        def interrupt_after_reserved_intent(data: dict[str, object]) -> None:
            nonlocal interrupted
            write_payload(data)
            raw_session = data.get("sessions", {}).get(completed.session_id, {})
            intent = raw_session.get("retirement", {}).get("export_intent", {})
            if (
                not interrupted
                and intent.get("stage_state") == "reserved"
                and intent.get("stage_attempt") == 1
            ):
                interrupted = True
                raise KeyboardInterrupt("crash before reserved stage mkdir")

        with patch.object(
            mission,
            "_write_runtime_payload",
            side_effect=interrupt_after_reserved_intent,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "reserved stage mkdir"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id=correlation_id,
                )

        persisted = self.load_mission().sessions[completed.session_id]
        first_intent = persisted.retirement["export_intent"]
        self.assertEqual(first_intent["stage_state"], "reserved")
        self.assertEqual(first_intent["stage_attempt"], 1)
        first_stage = destination.parent / first_intent["stage_name"]
        self.assertFalse(first_stage.exists())
        first_stage.mkdir(mode=0o700)
        sentinel = first_stage / "foreign-sentinel.txt"
        sentinel.write_text(
            "foreign deterministic stage owner must survive\n",
            encoding="utf-8",
        )
        observed_attempts: list[int] = []
        recovered = self.load_mission()
        recovered_write = recovered._write_runtime_payload

        def observe_retry_intent(data: dict[str, object]) -> None:
            raw_session = data.get("sessions", {}).get(completed.session_id, {})
            intent = raw_session.get("retirement", {}).get("export_intent", {})
            if isinstance(intent.get("stage_attempt"), int):
                observed_attempts.append(intent["stage_attempt"])
            recovered_write(data)

        with patch.object(
            recovered,
            "_write_runtime_payload",
            side_effect=observe_retry_intent,
        ):
            receipt = recovered.export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id=correlation_id,
            )

        self.assertIn(2, observed_attempts)
        self.assertEqual(
            receipt["destination"],
            str(destination.resolve(strict=True) / "repository"),
        )
        marker = json.loads(
            (destination / "retirement-export.json").read_text(encoding="utf-8")
        )
        self.assertNotEqual(marker["stage_name"], first_intent["stage_name"])
        self.assertEqual(
            sentinel.read_text(encoding="utf-8"),
            "foreign deterministic stage owner must survive\n",
        )
        self.assertEqual(
            (destination / "repository" / "tracked.txt").read_text(
                encoding="utf-8"
            ),
            "baseline\n",
        )

    def test_export_owner_capacity_rejects_before_second_public_effect(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        second_issue = self.add_issue(2)
        mission = self.load_mission()
        first_completed = self.completed_session(mission)
        second_completed = self.completed_issue(mission, second_issue)
        first_blocked = self.retirement_blocked_snapshot(
            mission,
            first_completed,
        )
        second_blocked = self.retirement_blocked_snapshot(
            mission,
            second_completed,
        )
        first_destination = self.root / "owner-capacity-first-export"
        second_destination = self.root / "owner-capacity-second-export"
        first_correlation = "complete-owner-at-capacity"
        second_correlation = "reject-owner-over-capacity"
        owner_root = mission.runtime_root / ".retirement-export-locks"

        with patch.object(core_module, "_RETIREMENT_EXPORT_OWNER_COUNT_LIMIT", 1):
            first_receipt = mission.export_retirement_unit(
                first_completed.session_id,
                destination=first_destination,
                expected_revision=first_blocked.revision,
                correlation_id=first_correlation,
            )
            owner_names = tuple(
                sorted(
                    entry.name
                    for entry in owner_root.iterdir()
                    if entry.name.endswith(".owner")
                )
            )
            self.assertEqual(len(owner_names), 1)

            with self.assertRaisesRegex(
                AlbertError,
                "owner registry capacity|capacity is exhausted",
            ):
                self.load_mission().export_retirement_unit(
                    second_completed.session_id,
                    destination=second_destination,
                    expected_revision=second_blocked.revision,
                    correlation_id=second_correlation,
                )

            replayed = self.load_mission().export_retirement_unit(
                first_completed.session_id,
                destination=first_destination,
                expected_revision=first_blocked.revision,
                correlation_id=first_correlation,
            )

        self.assertEqual(replayed, first_receipt)
        self.assertEqual(
            tuple(
                sorted(
                    entry.name
                    for entry in owner_root.iterdir()
                    if entry.name.endswith(".owner")
                )
            ),
            owner_names,
        )
        self.assertFalse(second_destination.exists())
        second_persisted = self.load_mission().sessions[
            second_completed.session_id
        ]
        self.assertEqual(
            second_persisted.revision,
            second_blocked.revision,
        )
        self.assertEqual(
            second_persisted.retirement.get("export_intent") or {},
            {},
        )
        self.assertNotIn(
            second_correlation,
            second_persisted.retirement.get("action_receipts", {}),
        )

    def test_export_publish_rejects_inner_payload_substitution(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "inner-payload-substitution-export"
        correlation_id = "reject-inner-payload-substitution"
        rename_noreplace = core_module._rename_directory_noreplace_at
        claimed_payloads: list[Path] = []
        foreign_payloads: list[Path] = []
        substituted = False

        def substitute_inner_payload_at_publish(
            source_parent_fd: int,
            source_name: str,
            destination_parent_fd: int,
            destination_name: str,
        ) -> None:
            nonlocal substituted
            if not substituted:
                self.assertEqual(source_name, "payload")
                self.assertEqual(destination_name, destination.name)
                anchor = mission._bound_retirement_export_directory_path(
                    source_parent_fd
                )
                claimed_name = anchor.name + ".claimed-payload"
                os.rename(
                    source_name,
                    claimed_name,
                    src_dir_fd=source_parent_fd,
                    dst_dir_fd=destination_parent_fd,
                )
                os.mkdir(source_name, mode=0o700, dir_fd=source_parent_fd)
                foreign_fd = mission._open_retirement_export_directory(
                    source_name,
                    dir_fd=source_parent_fd,
                )
                try:
                    sentinel_fd = os.open(
                        "foreign-sentinel.txt",
                        os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                        0o600,
                        dir_fd=foreign_fd,
                    )
                    try:
                        os.write(
                            sentinel_fd,
                            b"foreign payload must never become public\n",
                        )
                    finally:
                        os.close(sentinel_fd)
                finally:
                    os.close(foreign_fd)
                claimed_payloads.append(destination.parent / claimed_name)
                foreign_payloads.append(anchor / source_name)
                substituted = True
            rename_noreplace(
                source_parent_fd,
                source_name,
                destination_parent_fd,
                destination_name,
            )

        with patch.object(
            core_module,
            "_rename_directory_noreplace_at",
            side_effect=substitute_inner_payload_at_publish,
        ):
            with self.assertRaisesRegex(
                AlbertError,
                "publication|staging|source|boundary|changed",
            ):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id=correlation_id,
                )

        self.assertEqual(len(claimed_payloads), 1)
        self.assertEqual(len(foreign_payloads), 1)
        self.assertFalse(
            destination.exists(),
            "a substituted foreign payload became the public export",
        )
        self.assertEqual(
            (
                foreign_payloads[0] / "foreign-sentinel.txt"
            ).read_text(encoding="utf-8"),
            "foreign payload must never become public\n",
        )
        self.assertEqual(
            (
                claimed_payloads[0] / "repository" / "tracked.txt"
            ).read_text(encoding="utf-8"),
            "baseline\n",
        )
        self.assertTrue(
            (claimed_payloads[0] / "retirement-export.json").is_file()
        )
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertNotIn(
            correlation_id,
            persisted.retirement.get("action_receipts", {}),
        )

    def test_retirement_export_intent_fails_closed_on_runtime_load(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "malformed-export-intent"

        with patch.object(
            RetirementSnapshotStore,
            "materialize_into_directory",
            autospec=True,
            side_effect=KeyboardInterrupt("interrupt after export intent"),
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "export intent"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="malformed-export-intent",
                )

        baseline = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        for corruption in (
            "unexpected-field",
            "boolean-attempt",
            "zero-parent-inode",
            "unsafe-stage-name",
            "invalid-lock-digest",
            "invalid-claimed-at",
        ):
            with self.subTest(corruption=corruption):
                runtime = json.loads(json.dumps(baseline))
                intent = runtime["sessions"][completed.session_id]["retirement"][
                    "export_intent"
                ]
                if corruption == "unexpected-field":
                    intent["unexpected"] = "unbounded"
                elif corruption == "boolean-attempt":
                    intent["stage_attempt"] = True
                elif corruption == "zero-parent-inode":
                    intent["parent_inode"] = 0
                elif corruption == "unsafe-stage-name":
                    intent["stage_name"] = "../foreign-stage"
                elif corruption == "invalid-lock-digest":
                    intent["destination_lock_sha256"] = "not-a-digest"
                else:
                    intent["claimed_at"] = "not-a-timestamp"
                mission.runtime_path.write_text(
                    json.dumps(runtime, indent=2, sort_keys=True),
                    encoding="utf-8",
                )

                with self.assertRaisesRegex(
                    AlbertError,
                    "Retirement export intent is invalid",
                ):
                    self.load_mission()

        mission.runtime_path.write_text(
            json.dumps(baseline, indent=2, sort_keys=True),
            encoding="utf-8",
        )

    def test_retained_worktree_discard_manifest_fails_closed_on_runtime_load(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        blocked_mission = self.load_mission(
            quiescence=("absent", "live-exact")
        )
        completed = self.completed_session(blocked_mission)
        (completed.worktree_path / "retained-link").symlink_to("missing-target")
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            blocked_mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-malformed-discard-manifest",
            )
        blocked = blocked_mission._refresh_persisted_session(
            completed.session_id
        )
        quiesced = self.load_mission()
        with patch.object(
            quiesced,
            "_discard_unpreserved_retained_worktree",
            side_effect=KeyboardInterrupt("interrupt after discard intent"),
        ):
            with self.assertRaisesRegex(
                KeyboardInterrupt,
                "interrupt after discard intent",
            ):
                quiesced.discard_retained_worktree(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="malformed-discard-manifest",
                    confirmation=completed.session_id,
                    reason="Persist the exact direct-discard authority for validation.",
                )

        baseline = json.loads(
            quiesced.runtime_path.read_text(encoding="utf-8")
        )

        def refresh_tree_digest(runtime: dict[str, object]) -> None:
            intent = runtime["sessions"][completed.session_id]["retirement"][
                "discard_intent"
            ]
            manifest = intent["tree_manifest"]
            rebuilt = RetirementSnapshotStore._build_retained_worktree_manifest(
                root_mode=manifest["root_mode"],
                entries=manifest["entries"],
            )
            manifest["tree_sha256"] = rebuilt["tree_sha256"]
            manifest["materialized_tree_sha256"] = rebuilt[
                "materialized_tree_sha256"
            ]
            intent["tree_sha256"] = rebuilt["tree_sha256"]

        for corruption in (
            "boolean-schema-version",
            "float-schema-version",
            "unexpected-top-level-field",
            "noncanonical-relative-path",
            "unencodable-relative-path",
            "unexpected-entry-field",
            "nul-symlink-target",
        ):
            with self.subTest(corruption=corruption):
                runtime = json.loads(json.dumps(baseline))
                intent = runtime["sessions"][completed.session_id]["retirement"][
                    "discard_intent"
                ]
                manifest = intent["tree_manifest"]
                if corruption == "boolean-schema-version":
                    manifest["schema_version"] = True
                elif corruption == "float-schema-version":
                    manifest["schema_version"] = 2.0
                elif corruption == "unexpected-top-level-field":
                    manifest["unbounded_extension"] = "not authorized"
                else:
                    entry_key = next(iter(manifest["entries"]))
                    if corruption == "noncanonical-relative-path":
                        entry = manifest["entries"].pop(entry_key)
                        manifest["entries"][f"./{entry_key}"] = entry
                    elif corruption == "unencodable-relative-path":
                        entry = manifest["entries"].pop(entry_key)
                        manifest["entries"]["bad-\ud800"] = entry
                    elif corruption == "nul-symlink-target":
                        symlink_record = next(
                            record
                            for record in manifest["entries"].values()
                            if record.get("kind") == "symlink"
                        )
                        symlink_record["target"] = "00"
                    else:
                        manifest["entries"][entry_key]["unexpected"] = True
                    refresh_tree_digest(runtime)
                quiesced.runtime_path.write_text(
                    json.dumps(runtime, indent=2, sort_keys=True),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(
                    AlbertError,
                    "Retained Worktree Discard intent is invalid",
                ):
                    self.load_mission()

        quiesced.runtime_path.write_text(
            json.dumps(baseline, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        with patch(
            "albert_mvp.retirement._RETAINED_WORKTREE_RECOVERY_METADATA_BYTES_LIMIT",
            1,
        ):
            with self.assertRaisesRegex(
                AlbertError,
                "Retained Worktree Discard intent is invalid",
            ):
                self.load_mission()

    def test_retained_worktree_manifest_uses_exact_canonical_byte_limit(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        manifest = RetirementSnapshotStore.retained_worktree_manifest(
            completed.worktree_path,
            exclude_git_metadata=True,
        )
        canonical_bytes = len(
            json.dumps(
                manifest,
                ensure_ascii=True,
                sort_keys=True,
                separators=(",", ":"),
            ).encode("ascii")
        )

        with patch(
            "albert_mvp.retirement._RETAINED_WORKTREE_RECOVERY_METADATA_BYTES_LIMIT",
            canonical_bytes,
        ):
            self.assertEqual(
                RetirementSnapshotStore.validated_retained_worktree_manifest(
                    manifest
                ),
                manifest,
            )
            self.assertEqual(
                RetirementSnapshotStore.retained_worktree_manifest(
                    completed.worktree_path,
                    exclude_git_metadata=True,
                ),
                manifest,
            )
        with patch(
            "albert_mvp.retirement._RETAINED_WORKTREE_RECOVERY_METADATA_BYTES_LIMIT",
            canonical_bytes - 1,
        ):
            with self.assertRaisesRegex(
                RetirementSnapshotError,
                "exceeds 32 MiB",
            ):
                RetirementSnapshotStore.validated_retained_worktree_manifest(
                    manifest
                )
            with self.assertRaisesRegex(
                RetirementSnapshotError,
                "exceeds 32 MiB",
            ):
                RetirementSnapshotStore.retained_worktree_manifest(
                    completed.worktree_path,
                    exclude_git_metadata=True,
                )

    def test_retirement_action_receipts_are_bound_to_the_owning_session(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-before-owner-bound-receipt",
        )
        baseline = json.loads(mission.runtime_path.read_text(encoding="utf-8"))

        impossible_revision = json.loads(json.dumps(baseline))
        impossible_revision["sessions"][preserved.session_id]["retirement"]["action_receipts"]["impossible-result"] = {
            "action": "retry",
            "mission_id": mission.mission_id,
            "session_id": preserved.session_id,
            "expected_revision": preserved.revision,
            "result_revision": preserved.revision + 2,
            "result_phase": "retired",
            "recorded_at": datetime.now().astimezone().isoformat(),
        }
        mission.runtime_path.write_text(
            json.dumps(impossible_revision, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(
            AlbertError,
            "Retirement Unit action receipt is invalid",
        ):
            self.load_mission()

        wrong_confirmation = json.loads(json.dumps(baseline))
        wrong_confirmation["sessions"][preserved.session_id]["retirement"]["action_receipts"]["wrong-confirmation"] = {
            "action": "discard",
            "mission_id": mission.mission_id,
            "session_id": preserved.session_id,
            "expected_revision": completed.revision,
            "result_revision": preserved.revision,
            "confirmation": "some-other-session",
            "reason": "This receipt must not authorize a different session.",
            "recorded_at": datetime.now().astimezone().isoformat(),
        }
        mission.runtime_path.write_text(
            json.dumps(wrong_confirmation, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(
            AlbertError,
            "Retirement Unit action receipt is invalid",
        ):
            self.load_mission()

        mission.runtime_path.write_text(
            json.dumps(baseline, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        pinned = mission.set_retirement_snapshot_pin(
            preserved.session_id,
            pinned=True,
            expected_revision=preserved.revision,
            correlation_id="owner-bound-pin-receipt",
        )
        owner_bound = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        receipt = owner_bound["sessions"][pinned.session_id]["retirement"][
            "action_receipts"
        ]["owner-bound-pin-receipt"]
        receipt["session_id"] = "some-other-session"
        mission.runtime_path.write_text(
            json.dumps(owner_bound, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(
            AlbertError,
            "Retirement Unit action receipt is invalid",
        ):
            self.load_mission()

    def test_snapshot_storage_schema_rejects_payload_authority_drift(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-for-storage-authority-validation",
        )
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        snapshot = runtime["sessions"][preserved.session_id]["retirement"]["snapshot"]
        outside_payload = self.root / "outside-retirement-payload"
        snapshot["payload_path"] = str(outside_payload)
        snapshot["manifest_path"] = str(outside_payload / "manifest.json")
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )

        with self.assertRaisesRegex(AlbertError, "record authority is invalid"):
            self.load_mission()

    def test_reclamation_recovers_after_payload_deletion_before_record_commit(
        self,
    ) -> None:
        second_issue = self.add_issue(2)
        third_issue = self.add_issue(3)
        mission = self.load_mission()
        first = self.completed_issue(mission, "ISS-01")
        mission.record_frontier_review(
            first.session_id,
            "Approved",
            reason="Retire the first payload for crash recovery.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=first.revision,
        )
        second = self.completed_issue(mission, second_issue)
        mission.record_frontier_review(
            second.session_id,
            "Approved",
            reason="Retire the second payload for crash recovery.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=second.revision,
        )
        first_record = mission._refresh_persisted_session(first.session_id).retirement[
            "snapshot"
        ]
        second_record = mission._refresh_persisted_session(second.session_id).retirement[
            "snapshot"
        ]
        constrained = self.load_mission(
            snapshot_storage_budget_bytes=32 * 1024 * 1024
            + max(first_record["snapshot_bytes"], second_record["snapshot_bytes"]),
        )
        constrained.assign_issue(third_issue, "fake-local")
        constrained.approve_issue(third_issue)
        runtime = json.loads(constrained.runtime_path.read_text(encoding="utf-8"))
        first_snapshot = runtime["sessions"][first.session_id]["retirement"][
            "snapshot"
        ]
        first_snapshot["created_at"] = (
            datetime.now().astimezone() - timedelta(days=2)
        ).isoformat()
        first_snapshot["expires_at"] = (
            datetime.now().astimezone() - timedelta(seconds=1)
        ).isoformat()
        constrained.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        reclaim_payload = RetirementSnapshotStore.reclaim_verified_payload
        interrupted = False

        def interrupt_after_payload_deletion(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            *,
            expected_root_device: int,
            expected_root_inode: int,
        ) -> None:
            nonlocal interrupted
            reclaim_payload(
                store,
                record,
                expected_root_device=expected_root_device,
                expected_root_inode=expected_root_inode,
            )
            if (
                store.payload_root == Path(first_record["payload_path"])
                and not interrupted
            ):
                interrupted = True
                raise KeyboardInterrupt("crash after payload deletion")

        with patch.object(
            RetirementSnapshotStore,
            "reclaim_verified_payload",
            autospec=True,
            side_effect=interrupt_after_payload_deletion,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "payload deletion"):
                constrained.launch_issue(third_issue)

        self.assertFalse(Path(first_record["payload_path"]).exists())
        crash_runtime = json.loads(
            constrained.runtime_path.read_text(encoding="utf-8")
        )
        legacy_intent = crash_runtime["retirement_storage"][
            "reclamation_intents"
        ][first.session_id]
        legacy_intent.pop("root_device")
        legacy_intent.pop("root_inode")
        constrained.runtime_path.write_text(
            json.dumps(crash_runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        recovered = self.load_mission(
            snapshot_storage_budget_bytes=32 * 1024 * 1024
            + max(first_record["snapshot_bytes"], second_record["snapshot_bytes"]),
        )
        launched = recovered.launch_issue(third_issue)
        self.assertEqual(launched.issue_id, third_issue)
        compact = recovered._refresh_persisted_session(first.session_id).retirement[
            "snapshot"
        ]
        self.assertEqual(compact["payload_disposition"], "reclaimed")
        self.assertEqual(compact["manifest_sha256"], first_record["manifest_sha256"])

    def test_reclamation_preserves_payload_root_replaced_after_verification(
        self,
    ) -> None:
        second_issue = self.add_issue(2)
        mission = self.load_mission()
        completed = self.completed_issue(mission, "ISS-01")
        mission.record_frontier_review(
            completed.session_id,
            "Approved",
            reason="Retire one payload before testing a reclamation root swap.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=completed.revision,
        )
        retired = mission._refresh_persisted_session(completed.session_id)
        payload_root = Path(retired.retirement["snapshot"]["payload_path"])
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        snapshot = runtime["sessions"][completed.session_id]["retirement"][
            "snapshot"
        ]
        snapshot["created_at"] = (
            datetime.now().astimezone() - timedelta(days=2)
        ).isoformat()
        snapshot["expires_at"] = (
            datetime.now().astimezone() - timedelta(seconds=1)
        ).isoformat()
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        constrained = self.load_mission(
            snapshot_storage_budget_bytes=32 * 1024 * 1024,
            perform_startup_effects=False,
        )
        constrained.assign_issue(second_issue, "fake-local")
        constrained.approve_issue(second_issue)
        verify_snapshot = RetirementSnapshotStore.verify
        parked_payload = self.root / "verified-payload-parked-before-reclamation"
        replacement = payload_root / "foreign-replacement.txt"
        swapped = False

        def replace_root_after_verification(
            store: RetirementSnapshotStore,
            record: dict[str, object],
        ) -> bool:
            nonlocal swapped
            verified = verify_snapshot(store, record)
            if store.payload_root == payload_root and not swapped:
                swapped = True
                store.payload_root.replace(parked_payload)
                store.payload_root.mkdir()
                replacement.write_text(
                    "replacement payload bytes must survive\n",
                    encoding="utf-8",
                )
            return verified

        with patch.object(
            RetirementSnapshotStore,
            "verify",
            autospec=True,
            side_effect=replace_root_after_verification,
        ):
            with self.assertRaisesRegex(
                LaunchBlockedError,
                "Storage Budget is exhausted",
            ):
                constrained.launch_issue(second_issue)

        self.assertTrue(swapped)
        self.assertEqual(
            replacement.read_text(encoding="utf-8"),
            "replacement payload bytes must survive\n",
        )
        self.assertTrue(parked_payload.is_dir())
        compact = constrained._refresh_persisted_session(
            completed.session_id
        ).retirement["snapshot"]
        self.assertEqual(compact["payload_disposition"], "retained")

    def test_preservation_budget_is_reserved_before_execution_and_released_only_after_verification(
        self,
    ) -> None:
        mission = self.load_mission()
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")

        launched = mission.launch_issue("ISS-01")

        self.assertEqual(launched.retirement["phase"], "active")
        self.assertEqual(launched.preservation_budget["state"], "reserved")
        self.assertTrue(launched.preservation_budget["bound"])
        self.assertGreater(launched.preservation_budget["reserved_bytes"], 0)
        reloaded = self.load_mission().sessions[launched.session_id]
        self.assertTrue(reloaded.preservation_budget["bound"])

        completed = mission.run_session(launched.session_id)
        boundary = completed.retirement["runner_boundary"]
        self.assertEqual(
            boundary["owner_release_operation_id"],
            completed.runner_operation_id,
        )
        self.assertTrue(boundary["owner_released_at"])
        self.assertFalse(Path(boundary["owner_lease_path"]).exists())
        self.assertEqual(
            mission._probe_retirement_quiescence(boundary),
            ("absent", "absent"),
        )
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-budget",
        )

        self.assertEqual(preserved.retirement["phase"], "preserved")
        self.assertEqual(preserved.preservation_budget["state"], "verified")
        self.assertFalse(preserved.preservation_budget["bound"])
        snapshot = preserved.retirement["snapshot"]
        self.assertEqual(
            snapshot["snapshot_bytes"],
            snapshot["payload_bytes"] + snapshot["manifest_bytes"],
        )
        self.assertLessEqual(
            snapshot["snapshot_bytes"],
            preserved.preservation_budget["reserved_bytes"],
        )
        self.assertTrue(mission.verify_retirement_snapshot(preserved.session_id))

        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        receipt = runtime["sessions"][completed.session_id]["retirement"][
            "preservation_receipt"
        ]
        exact_receipt_digest = receipt["manifest_sha256"]
        receipt["manifest_sha256"] = "corrupt"
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(AlbertError, "exact result"):
            self.load_mission()
        receipt["manifest_sha256"] = exact_receipt_digest
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )

    def test_one_expected_revision_lock_allows_only_one_retirement_lifecycle_winner(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        expected_revision = completed.revision

        def preserve() -> str:
            try:
                mission.preserve_retirement_unit(
                    completed.session_id,
                    expected_revision=expected_revision,
                    correlation_id="preserve-race",
                )
            except AlbertError:
                return "rejected"
            return "preserved"

        def review() -> str:
            try:
                mission.record_frontier_review(
                    completed.session_id,
                    "Approved",
                    reason="Evidence is sufficient.",
                    allowed_session_statuses={"evidence-ready"},
                    expected_revision=expected_revision,
                )
            except AlbertError:
                return "rejected"
            return "reviewed"

        with ThreadPoolExecutor(max_workers=2) as executor:
            futures = [executor.submit(preserve), executor.submit(review)]
            outcomes = {future.result() for future in futures}

        self.assertEqual(len(outcomes & {"preserved", "reviewed"}), 1)
        current = self.load_mission().sessions[completed.session_id]
        if current.retirement["phase"] == "preserved":
            replayed = mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=expected_revision,
                correlation_id="preserve-race",
            )
            self.assertEqual(replayed.revision, current.revision)
        else:
            with self.assertRaisesRegex(AlbertError, "lifecycle revision"):
                mission.preserve_retirement_unit(
                    completed.session_id,
                    expected_revision=expected_revision,
                    correlation_id="preserve-race",
                )
        with self.assertRaisesRegex(AlbertError, "lifecycle revision"):
            mission.run_session(
                completed.session_id,
                expected_revision=expected_revision,
            )
        with self.assertRaisesRegex(AlbertError, "lifecycle revision"):
            mission.cancel_session(
                completed.session_id,
                reason="Do not race preservation.",
                expected_revision=expected_revision,
            )
        with self.assertRaisesRegex(AlbertError, "lifecycle revision"):
            mission.launch_repair(
                completed.session_id,
                manual_retry_reason="Do not race preservation.",
                expected_revision=expected_revision,
            )
        self.assertGreater(current.revision, expected_revision)

    def test_concurrent_exact_preservation_recovery_publishes_once(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        first = self.load_mission()
        second = self.load_mission()
        capture = RetirementSnapshotStore.capture
        active = 0
        maximum_active = 0
        capture_calls = 0
        counter_lock = threading.Lock()

        def observed_capture(store):
            nonlocal active, maximum_active, capture_calls
            with counter_lock:
                active += 1
                capture_calls += 1
                maximum_active = max(maximum_active, active)
            time.sleep(0.05)
            try:
                return capture(store)
            finally:
                with counter_lock:
                    active -= 1

        def preserve(candidate):
            return candidate.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="concurrent-exact-preservation",
            )

        with patch.object(
            RetirementSnapshotStore,
            "capture",
            autospec=True,
            side_effect=observed_capture,
        ):
            with ThreadPoolExecutor(max_workers=2) as executor:
                results = list(executor.map(preserve, (first, second)))

        self.assertEqual(
            [result.retirement["phase"] for result in results],
            ["preserved"] * 2,
        )
        self.assertEqual(capture_calls, 1)
        self.assertEqual(maximum_active, 1)

    def test_worktree_identity_requires_stored_managed_canonical_and_git_registration_agreement(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        self.assertEqual(
            mission.current_worktree_identity(completed.session_id),
            completed.worktree_identity,
        )

        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        runtime["sessions"][completed.session_id]["worktree_path"] = str(
            self.target_repo
        )
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        tampered = self.load_mission()

        self.assertEqual(
            tampered.current_worktree_identity(completed.session_id),
            "",
        )
        with self.assertRaisesRegex(AlbertError, "Worktree Identity"):
            tampered.preserve_retirement_unit(
                completed.session_id,
                expected_revision=tampered.sessions[completed.session_id].revision,
                correlation_id="preserve-identity",
            )

    def test_terminal_status_cannot_replace_independent_runner_quiescence_proof(
        self,
    ) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)

        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-quiescence",
            )

        blocked = self.load_mission().sessions[completed.session_id]
        self.assertEqual(blocked.retirement["phase"], "preservation-blocked")
        self.assertTrue(blocked.preservation_budget["bound"])
        self.assertFalse(blocked.retirement.get("snapshot"))

    def test_missing_legacy_runner_boundary_cannot_be_inferred_as_never_started(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        raw_session = runtime["sessions"][completed.session_id]
        raw_session["retirement"]["runner_boundary"] = {}
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        legacy = self.load_mission(quiescence=None, perform_startup_effects=False)
        legacy_session = legacy.sessions[completed.session_id]

        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            legacy.preserve_retirement_unit(
                completed.session_id,
                expected_revision=legacy_session.revision,
                correlation_id="preserve-missing-legacy-runner-boundary",
            )

        blocked = self.load_mission(
            perform_startup_effects=False
        ).sessions[completed.session_id]
        self.assertEqual(blocked.retirement["phase"], "preservation-blocked")
        self.assertEqual(blocked.retirement["runner_boundary"], {})
        self.assertTrue(blocked.preservation_budget["bound"])
        self.assertFalse(blocked.retirement.get("snapshot"))

    def test_late_lifecycle_race_quarantines_publication_and_blocks_unit(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        capture = RetirementSnapshotStore.capture

        def capture_after_external_revision(store: RetirementSnapshotStore):
            snapshot = capture(store)
            runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
            runtime["sessions"][completed.session_id]["revision"] += 1
            mission.runtime_path.write_text(
                json.dumps(runtime, indent=2, sort_keys=True),
                encoding="utf-8",
            )
            return snapshot

        with (
            patch.object(
                RetirementSnapshotStore,
                "capture",
                autospec=True,
                side_effect=capture_after_external_revision,
            ),
            self.assertRaisesRegex(LaunchBlockedError, "lifecycle boundary"),
        ):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-late-race",
            )

        blocked = self.load_mission().sessions[completed.session_id]
        self.assertEqual(blocked.retirement["phase"], "preservation-blocked")
        self.assertTrue(blocked.preservation_budget["bound"])
        self.assertFalse(
            (
                mission.runtime_dir / "retirement" / "payloads" / completed.session_id
            ).exists()
        )
        self.assertEqual(
            len(list((mission.runtime_dir / "retirement" / "quarantine").iterdir())),
            1,
        )

    def test_startup_recovers_published_preserving_phase_idempotently(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        capture = RetirementSnapshotStore.capture

        def crash_after_publication(store: RetirementSnapshotStore):
            capture(store)
            raise KeyboardInterrupt("simulated process loss before phase commit")

        with (
            patch.object(
                RetirementSnapshotStore,
                "capture",
                autospec=True,
                side_effect=crash_after_publication,
            ),
            self.assertRaisesRegex(KeyboardInterrupt, "phase commit"),
        ):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-crash-cut",
            )

        interrupted = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(interrupted.retirement["phase"], "preserving")
        self.assertTrue(
            (mission.runtime_dir / "retirement" / "payloads" / completed.session_id).is_dir()
        )

        recovered = self.load_mission().sessions[completed.session_id]
        self.assertEqual(recovered.retirement["phase"], "preserved")
        self.assertEqual(
            recovered.retirement["preservation_receipt"]["correlation_id"],
            "preserve-crash-cut",
        )
        self.assertTrue(mission.verify_retirement_snapshot(completed.session_id))

    def test_snapshot_preserves_git_state_evidence_and_clean_room_reconstruction(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        (completed.worktree_path / "tracked.txt").write_text(
            "staged change\n", encoding="utf-8"
        )
        self._git("add", "tracked.txt", cwd=completed.worktree_path)
        (completed.worktree_path / "unstaged.txt").write_text(
            "unstaged change\n", encoding="utf-8"
        )
        (completed.worktree_path / "untracked.txt").write_text(
            "untracked change\n", encoding="utf-8"
        )

        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-git-state",
        )
        snapshot = preserved.retirement["snapshot"]
        manifest_path = Path(snapshot["manifest_path"])
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

        self.assertEqual(manifest["authority"]["mission_id"], mission.mission_id)
        self.assertEqual(manifest["authority"]["session_id"], completed.session_id)
        self.assertEqual(
            manifest["identity"]["worktree_identity"],
            completed.worktree_identity,
        )
        self.assertEqual(manifest["git_state"]["kind"], "git-worktree")
        self.assertGreater(manifest["git_state"]["status_bytes"], 0)
        self.assertEqual(set(manifest["evidence"]), set(completed.artifacts))
        self.assertTrue(manifest["verification"]["manifest_readback"])
        self.assertTrue(manifest["verification"]["clean_room_reconstruction"])
        self.assertTrue(mission.verify_retirement_snapshot(completed.session_id))

        moved_repository = self.root / "source-repository-moved-after-snapshot"
        self.target_repo.rename(moved_repository)
        self.assertTrue(mission.verify_retirement_snapshot(completed.session_id))

        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        runtime["sessions"][completed.session_id]["retirement"]["snapshot"][
            "schema_version"
        ] = 2
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(AlbertError, "unit boundary"):
            mission.verify_retirement_snapshot(completed.session_id)

        runtime["sessions"][completed.session_id]["retirement"]["snapshot"][
            "schema_version"
        ] = 1
        runtime["sessions"][completed.session_id]["retirement"]["snapshot"][
            "session_revision"
        ] += 1
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(AlbertError, "authority"):
            mission.verify_retirement_snapshot(completed.session_id)

        staged_patch = manifest_path.parent / "git" / "staged.patch"
        staged_patch.write_text("tampered\n", encoding="utf-8")
        with self.assertRaisesRegex(AlbertError, "integrity"):
            mission.verify_retirement_snapshot(completed.session_id)

    def test_managed_directory_cannot_bypass_expected_git_registration(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        (completed.worktree_path / ".git").rename(
            self.root / "detached-worktree-git-metadata"
        )

        with self.assertRaisesRegex(AlbertError, "Worktree Identity"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-unregistered",
            )
        blocked = self.load_mission().sessions[completed.session_id]
        self.assertEqual(blocked.retirement["phase"], "preservation-blocked")
        self.assertTrue(blocked.preservation_budget["bound"])

    def test_snapshot_preserves_untracked_symlinks_and_executable_modes(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        executable = completed.worktree_path / "run-check.sh"
        executable.write_text("#!/bin/sh\nprintf 'ready\\n'\n", encoding="utf-8")
        executable.chmod(0o755)
        link = completed.worktree_path / "tracked-link"
        os.symlink(b"tracked-\xff", os.fsencode(link))

        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-untracked-types",
        )
        manifest = json.loads(
            Path(preserved.retirement["snapshot"]["manifest_path"]).read_text(
                encoding="utf-8"
            )
        )
        untracked = manifest["git_state"]["untracked_entries"]

        self.assertEqual(
            untracked["run-check.sh"],
            {"kind": "file", "mode": 0o755},
        )
        self.assertEqual(
            untracked["tracked-link"],
            {"kind": "symlink"},
        )
        self.assertTrue(mission.verify_retirement_snapshot(completed.session_id))

    def test_accepted_evidence_is_preserved_and_retired_without_waiting_for_merge(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        worktree_path = completed.worktree_path.resolve()

        mission.record_frontier_review(
            completed.session_id,
            "Approved",
            reason="The verified evidence satisfies the Issue Slice.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=completed.revision,
        )

        retired = self.load_mission().sessions[completed.session_id]
        self.assertEqual(retired.retirement["phase"], "retired")
        self.assertFalse(worktree_path.exists())
        registered_paths = {
            Path(line.removeprefix("worktree ")).resolve(strict=False)
            for line in self._git("worktree", "list", "--porcelain").stdout.splitlines()
            if line.startswith("worktree ")
        }
        self.assertNotIn(worktree_path, registered_paths)
        self.assertEqual(retired.retirement["removal_kind"], "git-worktree")
        self.assertEqual(retired.retirement["retirement_attempts"], 1)
        self.assertTrue(retired.retirement["retired_at"])
        self.assertTrue(mission.verify_retirement_snapshot(completed.session_id))

    def test_git_retirement_refuses_changes_after_verified_preservation(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        tracked = completed.worktree_path / "tracked.txt"
        tracked.write_text("preserved tracked state\n", encoding="utf-8")
        prepare = RetirementSnapshotStore.prepare_git_non_force_removal

        def mutate_after_preservation(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            *,
            worktree_path: Path | None = None,
        ) -> None:
            tracked.parent.mkdir(parents=True, exist_ok=True)
            tracked.write_text("late human edit\n", encoding="utf-8")
            prepare(store, record, worktree_path=worktree_path)

        with patch.object(
            RetirementSnapshotStore,
            "prepare_git_non_force_removal",
            autospec=True,
            side_effect=mutate_after_preservation,
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="The earlier evidence was accepted.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )

        retained = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(retained.retirement["phase"], "retirement-blocked")
        self.assertEqual(retained.retirement["retirement_attempts"], 1)
        self.assertIn(
            "changed",
            retained.retirement["blocked_reason"],
        )
        self.assertTrue(retained.worktree_path.is_dir())
        self.assertEqual(tracked.read_text(encoding="utf-8"), "late human edit\n")

    def test_ignored_files_are_preserved_before_git_retirement(self) -> None:
        (self.target_repo / ".gitignore").write_text("ignored.log\n", encoding="utf-8")
        self._git("add", ".gitignore")
        self._git("commit", "-m", "ignore generated log")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        ignored = completed.worktree_path / "ignored.log"
        ignored.write_text("valuable ignored material\n", encoding="utf-8")

        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-ignored-material",
        )
        manifest = json.loads(
            Path(preserved.retirement["snapshot"]["manifest_path"]).read_text(
                encoding="utf-8"
            )
        )

        self.assertIn("ignored.log", manifest["git_state"]["untracked_paths"])
        self.assertTrue(mission.verify_retirement_snapshot(completed.session_id))

    def test_git_cleanup_preparation_is_idempotent_after_a_crash_cut(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        (completed.worktree_path / "tracked.txt").write_text(
            "preserved tracked state\n",
            encoding="utf-8",
        )
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-before-cleanup-crash",
        )
        snapshot_revision = preserved.retirement["snapshot"]["session_revision"]
        store = mission._retirement_snapshot_store(preserved, snapshot_revision)

        store.prepare_git_non_force_removal(preserved.retirement["snapshot"])
        store.prepare_git_non_force_removal(preserved.retirement["snapshot"])

        self.assertEqual(
            self._git("status", "--porcelain", cwd=preserved.worktree_path).stdout,
            "",
        )

    def test_git_cleanup_accepts_an_exact_partial_tracked_subset(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        (completed.worktree_path / "tracked.txt").write_text(
            "preserved tracked state\n",
            encoding="utf-8",
        )
        (completed.worktree_path / "unstaged.txt").write_text(
            "preserved second state\n",
            encoding="utf-8",
        )
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-before-partial-tracked-cleanup",
        )
        snapshot_revision = preserved.retirement["snapshot"]["session_revision"]
        store = mission._retirement_snapshot_store(preserved, snapshot_revision)
        self._git(
            "restore",
            "--source=HEAD",
            "--staged",
            "--worktree",
            "--",
            "tracked.txt",
            cwd=preserved.worktree_path,
        )

        store.prepare_git_non_force_removal(preserved.retirement["snapshot"])

        self.assertEqual(
            self._git("status", "--porcelain", cwd=preserved.worktree_path).stdout,
            "",
        )

    def test_git_cleanup_rechecks_bytes_written_at_the_restore_boundary(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        untracked = completed.worktree_path / "FAKE_AGENT_RESULT.md"
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-before-late-cleanup-write",
        )
        snapshot_revision = preserved.retirement["snapshot"]["session_revision"]
        store = mission._retirement_snapshot_store(preserved, snapshot_revision)
        run_git = store._git
        changed = False

        def mutate_at_restore(cwd, arguments, **kwargs):
            nonlocal changed
            if arguments and arguments[0] == "restore" and not changed:
                changed = True
                untracked.write_text("late boundary bytes\n", encoding="utf-8")
            return run_git(cwd, arguments, **kwargs)

        with (
            patch.object(store, "_git", side_effect=mutate_at_restore),
            self.assertRaisesRegex(
                RetirementSnapshotError,
                "changed after verified preservation",
            ),
        ):
            store.prepare_git_non_force_removal(preserved.retirement["snapshot"])

        self.assertEqual(
            untracked.read_text(encoding="utf-8"),
            "late boundary bytes\n",
        )

    def test_git_retirement_preserves_a_tracked_write_at_the_cleanup_boundary(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        tracked = completed.worktree_path / "tracked.txt"
        prepare = RetirementSnapshotStore.prepare_git_non_force_removal

        def mutate_during_prepare(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            *,
            worktree_path: Path | None = None,
        ) -> None:
            tracked.parent.mkdir(parents=True, exist_ok=True)
            tracked.write_text("late unpreserved bytes\n", encoding="utf-8")
            prepare(store, record, worktree_path=worktree_path)

        with patch.object(
            RetirementSnapshotStore,
            "prepare_git_non_force_removal",
            autospec=True,
            side_effect=mutate_during_prepare,
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Exercise a tracked cleanup-boundary write.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )

        reviewed = self.load_mission().sessions[completed.session_id]
        self.assertNotEqual(reviewed.retirement["phase"], "retired")
        self.assertEqual(tracked.read_text(encoding="utf-8"), "late unpreserved bytes\n")

    def test_absent_git_path_with_registration_recovers_by_exact_removal(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        moved_effect = self.root / "interrupted-worktree-effect"

        def remove_path_before_registration(_session, _removal_kind):
            shutil.move(completed.worktree_path, moved_effect)
            raise KeyboardInterrupt("crash before Git registration removal")

        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=remove_path_before_registration,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "registration removal"):
                mission.record_frontier_review(
                    completed.session_id,
                    "Approved",
                    reason="Exercise absent-path registration recovery.",
                    allowed_session_statuses={"evidence-ready"},
                    expected_revision=completed.revision,
                )

        recovered = mission.reconcile_retirement_unit(completed.session_id)
        self.assertEqual(recovered.retirement["phase"], "retired")
        self.assertEqual(recovered.retirement["retirement_attempts"], 2)
        registered = self._git("worktree", "list", "--porcelain").stdout
        self.assertNotIn(str(completed.worktree_path), registered)

    def test_unavailable_registration_inspection_becomes_durably_blocked(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        moved_effect = self.root / "registration-inspection-unavailable"

        def remove_path_before_registration(_session, _removal_kind):
            shutil.move(completed.worktree_path, moved_effect)
            raise KeyboardInterrupt("crash before registration inspection")

        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=remove_path_before_registration,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "registration inspection"):
                mission.record_frontier_review(
                    completed.session_id,
                    "Approved",
                    reason="Exercise unavailable registration inspection.",
                    allowed_session_statuses={"evidence-ready"},
                    expected_revision=completed.revision,
                )

        with patch.object(
            mission,
            "_git_worktree_registration_present",
            side_effect=AlbertError("registration unavailable"),
        ):
            blocked = mission.reconcile_retirement_unit(completed.session_id)

        self.assertEqual(blocked.retirement["phase"], "retirement-blocked")
        self.assertIn("registration unavailable", blocked.retirement["blocked_reason"])

    def test_failed_work_enters_passive_default_retention_grace(self) -> None:
        failing_runner = self.root / "failing-runner.py"
        failing_runner.write_text("raise SystemExit(7)\n", encoding="utf-8")
        command = f"python3 {failing_runner}"
        self.agent_config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "failing-local",
                            "role": "local-agent",
                            "provider": "local",
                            "runner": "command",
                            "command": command,
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        mission = self.load_mission()
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "failing-local")
        mission.approve_issue("ISS-01")
        launched = mission.launch_issue("ISS-01")

        result = mission.run_session(launched.session_id)
        self.assertEqual(result.status, "failed")

        failed = self.load_mission().sessions[launched.session_id]
        self.assertEqual(failed.status, "failed")
        self.assertEqual(failed.retirement["phase"], "grace")
        self.assertTrue(failed.worktree_path.is_dir())
        grace_started = datetime.fromisoformat(failed.retirement["grace_started_at"])
        grace_expires = datetime.fromisoformat(failed.retirement["grace_expires_at"])
        self.assertEqual((grace_expires - grace_started).total_seconds(), 72 * 60 * 60)
        self.assertFalse(
            any(
                session.task_packet.get("repair_context")
                for session in self.load_mission().sessions.values()
            )
        )

    def test_human_review_remains_nonterminal_and_retains_its_worktree(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)

        mission.record_frontier_review(
            completed.session_id,
            "Needs human review",
            reason="A person must inspect the evidence.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=completed.revision,
        )

        retained = self.load_mission().sessions[completed.session_id]
        self.assertEqual(retained.retirement["phase"], "active")
        self.assertTrue(retained.worktree_path.is_dir())
        self.assertEqual(
            self.load_mission().issues["ISS-01"].review_state,
            "needs-human-review",
        )

    def test_rejected_review_is_preserved_into_passive_grace(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)

        mission.record_frontier_review(
            completed.session_id,
            "Rejected",
            reason="The result is terminal but remains inspectable during grace.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=completed.revision,
        )

        rejected = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(rejected.retirement["phase"], "grace")
        self.assertTrue(rejected.worktree_path.is_dir())
        self.assertEqual(self.load_mission().issues["ISS-01"].review_state, "rejected")

    def test_completed_cancellation_retires_after_runner_quiescence(self) -> None:
        command = (
            f'{sys.executable} -c "import pathlib,time; '
            "pathlib.Path('started-before-cancel.txt').write_text('started'); "
            'time.sleep(30)"'
        )
        self.agent_config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "blocking-local",
                            "role": "local-agent",
                            "provider": "local",
                            "runner": "command",
                            "command": command,
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        mission = self.load_mission(quiescence=None)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "blocking-local")
        mission.approve_issue("ISS-01")
        launched = mission.launch_issue("ISS-01")
        runner = self.load_mission()
        sandbox = patch(
            "albert_mvp.core.sandboxed_process_argv",
            side_effect=lambda argv, **_kwargs: (argv, True),
        )
        sandbox.start()
        self.addCleanup(sandbox.stop)
        # This fixture intentionally substitutes raw argv for the prepared
        # Bubblewrap command so the test can coordinate cancellation with the
        # child directly. Keep the validator exception local to that fixture.
        provider_validation = patch(
            "albert_mvp.execution.PythonExecutionProvider.validate_request",
            autospec=True,
            side_effect=lambda _provider, request: request.validate(),
        )
        provider_validation.start()
        self.addCleanup(provider_validation.stop)
        outcomes: list[str] = []

        def run() -> None:
            try:
                outcomes.append(
                    runner.run_session(launched.session_id).retirement["phase"]
                )
            except Exception as exc:  # surfaced by the assertion below
                outcomes.append(f"error:{type(exc).__name__}:{exc}")

        thread = threading.Thread(target=run)
        thread.start()
        deadline = time.monotonic() + 5
        while (
            time.monotonic() < deadline
            and not (launched.worktree_path / "started-before-cancel.txt").exists()
        ):
            time.sleep(0.02)
        if not (launched.worktree_path / "started-before-cancel.txt").exists():
            latest = self.load_mission().sessions[launched.session_id]
            stderr = (
                Path(latest.artifacts["stderr"]).read_text(encoding="utf-8")
                if latest.artifacts.get("stderr")
                else ""
            )
            self.fail(
                "blocking runner never reached its observable process boundary: "
                f"{outcomes}; stderr={stderr!r}"
            )
        canceller = self.load_mission()
        active = canceller.sessions[launched.session_id]
        self.assertIsNotNone(active.runner_process_pid)

        canceller.cancel_session(
            launched.session_id,
            reason="Cancellation retirement contract test.",
            expected_revision=active.revision,
        )
        waiting = self.load_mission(quiescence=("live-exact", "live-exact"))
        self.assertEqual(
            waiting.sessions[launched.session_id].retirement["phase"],
            "active",
        )
        thread.join(timeout=10)

        self.assertFalse(thread.is_alive())
        self.assertEqual(outcomes, ["retired"])
        retired = self.load_mission().sessions[launched.session_id]
        self.assertEqual(retired.status, "cancelled")
        self.assertEqual(retired.retirement["phase"], "retired")
        self.assertFalse(retired.worktree_path.exists())

    def test_queued_cancellation_retires_a_verified_absent_worktree(self) -> None:
        mission = self.load_mission()
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")
        self.assertFalse(queued.worktree_path.exists())

        cancelled = mission.cancel_session(
            queued.session_id,
            reason="Cancel before worktree materialization.",
            expected_revision=queued.revision,
        )
        retired = self.load_mission().sessions[cancelled.session_id]

        self.assertEqual(retired.retirement["phase"], "retired")
        self.assertEqual(retired.retirement["removal_kind"], "managed-absence")
        self.assertTrue(retired.worktree_identity.startswith("managed-absence:"))
        self.assertTrue(mission.verify_retirement_snapshot(retired.session_id))

    def test_queued_cancellation_proves_quiescence_without_an_injected_probe(
        self,
    ) -> None:
        mission = self.load_mission(quiescence=None)
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")

        retired = mission.cancel_session(
            queued.session_id,
            reason="Cancel before any runner operation exists.",
            expected_revision=queued.revision,
        )

        self.assertEqual(retired.retirement["phase"], "retired")
        boundary = retired.retirement["runner_boundary"]
        self.assertTrue(boundary["runner_operation_id"].startswith("never-started:"))
        self.assertEqual(
            mission._probe_retirement_quiescence(boundary),
            ("absent", "absent"),
        )

    def test_managed_absence_removes_an_exact_lingering_git_registration(self) -> None:
        mission = self.load_mission()
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")
        self._git("worktree", "add", "--detach", str(queued.worktree_path), "HEAD")
        moved = self.root / "externally-moved-queued-worktree"
        shutil.move(queued.worktree_path, moved)

        mission.cancel_session(
            queued.session_id,
            reason="Cancel with a stale exact Git registration.",
            expected_revision=queued.revision,
        )
        retired = mission.reconcile_retirement_unit(queued.session_id)

        self.assertEqual(
            retired.retirement["phase"],
            "retired",
            retired.retirement,
        )
        self.assertEqual(retired.retirement["removal_kind"], "git-registration")
        registered = self._git("worktree", "list", "--porcelain").stdout
        self.assertNotIn(str(queued.worktree_path), registered)
        self.assertTrue(moved.is_dir())

    def test_pre_worktree_failure_enters_grace_with_verified_absence(self) -> None:
        mission = self.load_mission()
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        queued = mission.launch_issue("ISS-01")

        with (
            patch.object(
                mission,
                "_ensure_session_worktree",
                side_effect=AlbertError("injected pre-worktree failure"),
            ),
            self.assertRaisesRegex(AlbertError, "pre-worktree failure"),
        ):
            mission.run_session(queued.session_id)

        grace = mission._refresh_persisted_session(queued.session_id)
        self.assertEqual(grace.status, "failed")
        self.assertEqual(grace.retirement["phase"], "grace")
        self.assertTrue(grace.worktree_identity.startswith("managed-absence:"))
        self.assertFalse(grace.worktree_path.exists())
        self.assertTrue(mission.verify_retirement_snapshot(grace.session_id))

    def test_retirement_retries_are_bounded_and_exhaust_into_retirement_blocked(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)

        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("injected exact removal failure"),
        ) as removal:
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Evidence is accepted before bounded cleanup.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            first = mission._refresh_persisted_session(completed.session_id)
            self.assertEqual(first.retirement["phase"], "retiring")
            self.assertEqual(first.retirement["retirement_attempts"], 2)

            third = mission.reconcile_retirement_unit(completed.session_id)
            self.assertEqual(third.retirement["phase"], "retirement-blocked")
            self.assertEqual(third.retirement["retirement_attempts"], 3)
            self.assertEqual(removal.call_count, 3)

            replayed = mission.reconcile_retirement_unit(completed.session_id)
            self.assertEqual(replayed.retirement["phase"], "retirement-blocked")
            self.assertEqual(removal.call_count, 3)

        self.assertTrue(completed.worktree_path.is_dir())

    def test_crash_after_third_attempt_never_executes_a_fourth_effect(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)

        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=[
                AlbertError("attempt one failed"),
                AlbertError("attempt two failed"),
                KeyboardInterrupt("attempt three crashed"),
            ],
        ) as removal:
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Exercise the exact retry ceiling.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            with self.assertRaisesRegex(KeyboardInterrupt, "attempt three crashed"):
                mission.reconcile_retirement_unit(completed.session_id)
            crashed = mission._refresh_persisted_session(completed.session_id)
            self.assertEqual(crashed.retirement["retirement_attempts"], 3)

            blocked = mission.reconcile_retirement_unit(completed.session_id)
            self.assertEqual(blocked.retirement["phase"], "retirement-blocked")
            self.assertEqual(removal.call_count, 3)

    def test_corrupt_snapshot_blocks_retirement_without_blocking_startup(self) -> None:
        mission = self.load_mission(retention_grace_seconds=0)
        completed = self.completed_session(mission)
        completed.status = "failed"
        completed = mission._persist_session_update(
            completed,
            expected_statuses={"evidence-ready"},
        )
        grace = mission.reconcile_retirement_unit(completed.session_id)
        patch_path = Path(grace.retirement["snapshot"]["payload_path"]) / "git" / "unstaged.patch"
        patch_path.write_text("corrupt\n", encoding="utf-8")

        reloaded = self.load_mission(retention_grace_seconds=0)
        blocked = reloaded.sessions[completed.session_id]
        self.assertEqual(blocked.retirement["phase"], "retirement-blocked")
        self.assertIn("verification failed", blocked.retirement["blocked_reason"])

    def test_concurrent_reconciliation_executes_one_physical_removal(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        completed.status = "cancelled"
        completed = mission._persist_session_update(completed)
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-before-concurrent-retirement",
        )
        first = mission
        with patch.object(AlbertMission, "_reconcile_retirement_units"):
            second = self.load_mission()
        removal = AlbertMission._remove_retirement_worktree
        calls: list[str] = []

        def record_removal(candidate, session, removal_kind):
            calls.append(session.session_id)
            return removal(candidate, session, removal_kind)

        with patch.object(
            AlbertMission,
            "_remove_retirement_worktree",
            autospec=True,
            side_effect=record_removal,
        ):
            with ThreadPoolExecutor(max_workers=2) as executor:
                results = list(
                    executor.map(
                        lambda candidate: candidate.reconcile_retirement_unit(
                            completed.session_id
                        ),
                        (first, second),
                    )
                )

        self.assertEqual([result.retirement["phase"] for result in results], ["retired"] * 2)
        self.assertEqual(calls, [completed.session_id])

    def test_restart_after_removal_effect_finalizes_without_repeating_deletion(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)

        with patch.object(
            mission,
            "_verify_retirement_removal",
            side_effect=AlbertError("simulated crash after removal effect"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Evidence is accepted before crash-cut cleanup.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )

        interrupted = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(interrupted.retirement["phase"], "retiring")
        self.assertEqual(interrupted.retirement["retirement_attempts"], 1)
        self.assertFalse(completed.worktree_path.exists())

        recovered = self.load_mission().sessions[completed.session_id]
        self.assertEqual(recovered.retirement["phase"], "retired")
        self.assertEqual(recovered.retirement["retirement_attempts"], 1)

    def test_restart_resumes_a_git_worktree_isolated_before_cleanup(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        prepare = RetirementSnapshotStore.prepare_git_non_force_removal
        crashed = False

        def crash_once_after_isolation(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            *,
            worktree_path: Path | None = None,
        ) -> None:
            nonlocal crashed
            if not crashed:
                crashed = True
                self.assertIsNotNone(worktree_path)
                self.assertTrue(worktree_path.is_dir())
                self.assertFalse(completed.worktree_path.exists())
                raise KeyboardInterrupt("crash after Git worktree isolation")
            prepare(store, record, worktree_path=worktree_path)

        with patch.object(
            RetirementSnapshotStore,
            "prepare_git_non_force_removal",
            autospec=True,
            side_effect=crash_once_after_isolation,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "worktree isolation"):
                mission.record_frontier_review(
                    completed.session_id,
                    "Approved",
                    reason="Exercise isolated Git restart recovery.",
                    allowed_session_statuses={"evidence-ready"},
                    expected_revision=completed.revision,
                )

        interrupted = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(interrupted.retirement["phase"], "retiring")
        self.assertEqual(interrupted.retirement["retirement_attempts"], 1)
        recovered = mission.reconcile_retirement_unit(completed.session_id)
        self.assertEqual(recovered.retirement["phase"], "retired")
        self.assertEqual(recovered.retirement["retirement_attempts"], 2)

    def test_restart_resumes_partial_git_removal_with_deleted_tracked_content(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        prepare = RetirementSnapshotStore.prepare_git_non_force_removal
        crashed = False

        def crash_after_git_marker_removal(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            *,
            worktree_path: Path | None = None,
        ) -> None:
            nonlocal crashed
            prepare(store, record, worktree_path=worktree_path)
            if not crashed:
                crashed = True
                self.assertIsNotNone(worktree_path)
                (worktree_path / ".git").unlink()
                (worktree_path / "tracked.txt").unlink()
                raise KeyboardInterrupt("crash during Git worktree removal")

        with patch.object(
            RetirementSnapshotStore,
            "prepare_git_non_force_removal",
            autospec=True,
            side_effect=crash_after_git_marker_removal,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "Git worktree removal"):
                mission.record_frontier_review(
                    completed.session_id,
                    "Approved",
                    reason="Exercise partial Git removal restart recovery.",
                    allowed_session_statuses={"evidence-ready"},
                    expected_revision=completed.revision,
                )

        interrupted = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(interrupted.retirement["phase"], "retiring")
        recovered = mission.reconcile_retirement_unit(completed.session_id)
        self.assertEqual(
            recovered.retirement["phase"],
            "retired",
            recovered.retirement,
        )
        self.assertEqual(recovered.retirement["retirement_attempts"], 2)

    def test_restart_repairs_a_split_git_worktree_move_backpointer(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        crashed = False

        def crash_once_after_isolation(
            _store: RetirementSnapshotStore,
            _record: dict[str, object],
            *,
            worktree_path: Path | None = None,
        ) -> None:
            nonlocal crashed
            if not crashed:
                crashed = True
                raise KeyboardInterrupt("crash during Git worktree move")

        with patch.object(
            RetirementSnapshotStore,
            "prepare_git_non_force_removal",
            autospec=True,
            side_effect=crash_once_after_isolation,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "Git worktree move"):
                mission.record_frontier_review(
                    completed.session_id,
                    "Approved",
                    reason="Exercise a split Git move restart.",
                    allowed_session_statuses={"evidence-ready"},
                    expected_revision=completed.revision,
                )

        effect_path = mission._retirement_removal_effect_path(completed.session_id)
        marker = (effect_path / ".git").read_text(encoding="utf-8").strip()
        admin_path = Path(marker.removeprefix("gitdir: ")).resolve(strict=True)
        (admin_path / "gitdir").write_text(
            str(completed.worktree_path / ".git") + "\n",
            encoding="utf-8",
        )
        self.assertTrue(mission._git_worktree_registration_present(completed))
        self.assertFalse(mission._git_worktree_registration_present_at(effect_path))

        recovered = mission.reconcile_retirement_unit(completed.session_id)

        self.assertEqual(recovered.retirement["phase"], "retired")
        self.assertEqual(recovered.retirement["retirement_attempts"], 2)

    def test_restart_resumes_a_managed_directory_isolated_before_cleanup(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        prepare = RetirementSnapshotStore.prepare_managed_directory_removal
        crashed = False

        def crash_once_after_isolation(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            *,
            worktree_path: Path | None = None,
        ) -> None:
            nonlocal crashed
            if not crashed:
                crashed = True
                self.assertIsNotNone(worktree_path)
                self.assertTrue(worktree_path.is_dir())
                self.assertFalse(completed.worktree_path.exists())
                raise KeyboardInterrupt("crash after directory isolation")
            prepare(store, record, worktree_path=worktree_path)

        with patch.object(
            RetirementSnapshotStore,
            "prepare_managed_directory_removal",
            autospec=True,
            side_effect=crash_once_after_isolation,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "directory isolation"):
                mission.record_frontier_review(
                    completed.session_id,
                    "Approved",
                    reason="Exercise isolated directory restart recovery.",
                    allowed_session_statuses={"evidence-ready"},
                    expected_revision=completed.revision,
                )

        interrupted = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(interrupted.retirement["phase"], "retiring")
        self.assertEqual(interrupted.retirement["retirement_attempts"], 1)
        recovered = mission.reconcile_retirement_unit(completed.session_id)
        self.assertEqual(recovered.retirement["phase"], "retired")
        self.assertEqual(recovered.retirement["retirement_attempts"], 2)

    def test_expired_configurable_grace_retires_on_startup_reconciliation(self) -> None:
        failing_runner = self.root / "grace-expiry-runner.py"
        failing_runner.write_text("raise SystemExit(9)\n", encoding="utf-8")
        command = f"python3 {failing_runner}"
        self.agent_config.write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "grace-local",
                            "role": "local-agent",
                            "provider": "local",
                            "runner": "command",
                            "command": command,
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        mission = self.load_mission(retention_grace_seconds=0)
        mission.record_command_approval(command, "auto-allowed")
        mission.assign_issue("ISS-01", "grace-local")
        mission.approve_issue("ISS-01")
        launched = mission.launch_issue("ISS-01")
        mission.run_session(launched.session_id)

        grace = mission._refresh_persisted_session(launched.session_id)
        self.assertEqual(grace.retirement["phase"], "grace")

        retired = self.load_mission(retention_grace_seconds=0).sessions[
            launched.session_id
        ]
        self.assertEqual(retired.retirement["phase"], "retired")
        self.assertFalse(retired.worktree_path.exists())

    def test_repair_reconstructs_verified_snapshot_as_a_separate_retirement_unit(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        (completed.worktree_path / "prior-only.txt").write_text(
            "verified predecessor material\n",
            encoding="utf-8",
        )
        mission.record_frontier_review(
            completed.session_id,
            "Needs repair",
            reason="The prior result needs one separately authorized repair.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=completed.revision,
        )

        repair = mission.launch_repair(completed.session_id)

        retired_prior = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(retired_prior.retirement["phase"], "retired")
        self.assertFalse(retired_prior.worktree_path.exists())
        self.assertEqual(repair.status, "queued")
        self.assertEqual(repair.retirement["phase"], "active")
        self.assertTrue(repair.preservation_budget["bound"])
        self.assertEqual(
            repair.task_packet["repair_context"]["retirement_snapshot_sha256"],
            retired_prior.retirement["snapshot"]["manifest_sha256"],
        )

        repaired = mission.run_session(repair.session_id)
        self.assertEqual(
            (repaired.worktree_path / "prior-only.txt").read_text(encoding="utf-8"),
            "verified predecessor material\n",
        )
        self.assertEqual(
            repaired.repository_snapshot["repair_overlay"]["source"],
            "verified-retirement-snapshot",
        )

    def test_non_git_retirement_stays_inside_the_exact_managed_directory(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        managed_path = completed.worktree_path.resolve()
        self.assertTrue(completed.worktree_identity.startswith("managed-directory:"))

        mission.record_frontier_review(
            completed.session_id,
            "Approved",
            reason="Directory work is accepted for exact managed removal.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=completed.revision,
        )

        retired = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(retired.retirement["phase"], "retired")
        self.assertEqual(retired.retirement["removal_kind"], "managed-directory")
        self.assertFalse(managed_path.exists())
        self.assertTrue(self.target_repo.is_dir())
        self.assertTrue((self.target_repo / "tracked.txt").is_file())

    def test_non_git_retirement_blocks_when_the_directory_changes_after_preservation(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        prepare = RetirementSnapshotStore.prepare_managed_directory_removal

        def mutate_then_prepare(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            *,
            worktree_path: Path | None = None,
        ) -> None:
            completed.worktree_path.mkdir(parents=True, exist_ok=True)
            (completed.worktree_path / "late-change.txt").write_text(
                "not preserved\n",
                encoding="utf-8",
            )
            prepare(store, record, worktree_path=worktree_path)

        with patch.object(
            RetirementSnapshotStore,
            "prepare_managed_directory_removal",
            autospec=True,
            side_effect=mutate_then_prepare,
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="The preserved directory result is accepted.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )

        retained = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(retained.retirement["phase"], "retirement-blocked")
        self.assertTrue(retained.worktree_path.is_dir())
        self.assertEqual(retained.retirement["retirement_attempts"], 1)

    def test_retirement_blocked_inspect_export_and_retry_are_actionable(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create one actionable blocked retirement.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)

        blocked = mission._refresh_persisted_session(completed.session_id)
        inspection = mission.inspect_retirement_unit(completed.session_id)
        self.assertEqual(inspection["phase"], "retirement-blocked")
        self.assertEqual(
            inspection["actions"],
            {"retry": True, "inspect": True, "export": True, "discard": True},
        )

        destination = self.root / "retirement-export"
        exported = mission.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="export-blocked-retirement",
        )
        self.assertEqual(
            exported["destination"],
            str(destination.resolve() / "repository"),
        )
        self.assertEqual(
            (destination / "repository" / "tracked.txt").read_text(encoding="utf-8"),
            "baseline\n",
        )

        after_export = mission._refresh_persisted_session(completed.session_id)
        retried = mission.retry_retirement_unit(
            completed.session_id,
            expected_revision=after_export.revision,
            correlation_id="retry-blocked-retirement",
        )
        self.assertEqual(retried.retirement["phase"], "retired")
        self.assertFalse(completed.worktree_path.exists())

    def test_snapshot_payload_export_rejects_app_private_destination(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create one blocked snapshot-backed export.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        destinations = (
            (
                Path(blocked.retirement["snapshot"]["payload_path"]) / "user-export",
                "reject-private-snapshot-export",
                "app-private runtime storage",
            ),
            (
                completed.worktree_path / "user-export",
                "reject-source-snapshot-export",
                "outside the Retained Worktree",
            ),
            (
                mission.runtime_root / "another-project" / "user-export",
                "reject-sibling-runtime-snapshot-export",
                "app-private runtime storage",
            ),
        )
        for destination, correlation_id, error in destinations:
            with self.subTest(destination=destination):
                with self.assertRaisesRegex(AlbertError, error):
                    mission.export_retirement_unit(
                        completed.session_id,
                        destination=destination,
                        expected_revision=blocked.revision,
                        correlation_id=correlation_id,
                    )
                self.assertFalse(destination.exists())
        persisted = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(persisted.retirement.get("export_intent") or {}, {})

    @unittest.skipUnless(
        sys.platform == "darwin",
        "Darwin case-insensitive path alias regression",
    )
    def test_snapshot_payload_export_rejects_case_alias_of_runtime_root(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create one blocked snapshot-backed export.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        canonical_runtime = mission.runtime_root
        runtime_value = canonical_runtime.as_posix()
        if "/private/" not in runtime_value:
            self.skipTest("No alternate-case Darwin path prefix is available.")
        alias_runtime = Path(runtime_value.replace("/private/", "/PRIVATE/", 1))
        if not alias_runtime.exists() or not alias_runtime.samefile(canonical_runtime):
            self.skipTest("The test volume is case-sensitive.")
        destination = alias_runtime / "case-alias-export"

        with self.assertRaisesRegex(AlbertError, "app-private runtime storage"):
            mission.export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id="reject-case-alias-runtime-export",
            )

        self.assertFalse(destination.exists())
        source_value = completed.worktree_path.as_posix()
        alias_source = Path(source_value.replace("/private/", "/PRIVATE/", 1))
        if alias_source.exists() and alias_source.samefile(completed.worktree_path):
            source_destination = alias_source / "case-alias-export"
            with self.assertRaisesRegex(AlbertError, "outside the Retained Worktree"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=source_destination,
                    expected_revision=blocked.revision,
                    correlation_id="reject-case-alias-source-export",
                )
            self.assertFalse(source_destination.exists())
        persisted = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(persisted.retirement.get("export_intent") or {}, {})

    def test_failed_export_never_deletes_foreign_destination_data(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create one blocked snapshot-backed export.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        destination = self.root / "failed-export-with-foreign-data"
        staging_roots: list[Path] = []

        def fail_after_foreign_write(
            _store: RetirementSnapshotStore,
            _record: dict[str, object],
            destination_fd: int,
        ) -> None:
            destination_root = mission._bound_retirement_export_directory_path(
                destination_fd
            )
            staging_roots.append(destination_root)
            sentinel_fd = os.open(
                "foreign-sentinel.txt",
                os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                0o600,
                dir_fd=destination_fd,
            )
            try:
                os.write(
                    sentinel_fd,
                    b"foreign data must survive export failure\n",
                )
            finally:
                os.close(sentinel_fd)
            self.assertEqual(
                destination_root.parent.parent,
                destination.parent.resolve(),
            )
            self.assertNotEqual(destination_root, destination.resolve())
            raise RetirementSnapshotError("simulated export verification failure")

        with patch.object(
            RetirementSnapshotStore,
            "materialize_into_directory",
            autospec=True,
            side_effect=fail_after_foreign_write,
        ):
            with self.assertRaisesRegex(AlbertError, "retirement export failed"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="failed-export-preserves-foreign-data",
                )

        self.assertEqual(len(staging_roots), 1)
        sentinel = staging_roots[0] / "foreign-sentinel.txt"
        self.assertEqual(
            sentinel.read_text(encoding="utf-8"),
            "foreign data must survive export failure\n",
        )
        self.assertFalse(destination.exists())

    def test_retirement_export_rejects_parent_swapped_into_protected_tree(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        second_issue = self.add_issue(2)
        mission = self.load_mission()
        completed_sessions = (
            self.completed_session(mission),
            self.completed_issue(mission, second_issue),
        )
        blocked_sessions = tuple(
            self.retirement_blocked_snapshot(mission, completed)
            for completed in completed_sessions
        )
        protected_targets = (
            ("runtime", mission.runtime_root),
            ("source", completed_sessions[1].worktree_path),
        )

        for index, ((label, protected_target), completed, blocked) in enumerate(
            zip(protected_targets, completed_sessions, blocked_sessions),
            start=1,
        ):
            with self.subTest(protected_tree=label):
                parent = self.root / f"safe-export-parent-{index}"
                parent.mkdir()
                parked_parent = self.root / f"parked-export-parent-{index}"
                destination = parent / f"escaped-{label}-export"
                original_write = mission._write_runtime_payload
                swapped = False

                def swap_parent_after_intent(data: dict[str, object]) -> None:
                    nonlocal swapped
                    original_write(data)
                    raw_session = data.get("sessions", {}).get(
                        completed.session_id,
                        {},
                    )
                    intent = raw_session.get("retirement", {}).get(
                        "export_intent",
                        {},
                    )
                    if intent and not swapped:
                        parent.rename(parked_parent)
                        parent.symlink_to(protected_target, target_is_directory=True)
                        swapped = True

                with patch.object(
                    mission,
                    "_write_runtime_payload",
                    side_effect=swap_parent_after_intent,
                ):
                    with self.assertRaisesRegex(
                        AlbertError,
                        "destination|boundary|runtime|Retained Worktree",
                    ):
                        mission.export_retirement_unit(
                            completed.session_id,
                            destination=destination,
                            expected_revision=blocked.revision,
                            correlation_id=f"reject-parent-swap-{label}",
                        )

                self.assertTrue(swapped)
                self.assertFalse(
                    (protected_target / destination.name).exists()
                )

    def test_retirement_export_publish_uses_bound_parent_after_late_swap(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        parent = self.root / "late-swap-export-parent"
        parent.mkdir()
        parked_parent = self.root / "late-swap-export-parent-parked"
        destination = parent / "late-swapped-export"
        publish = mission._publish_retirement_export_noreplace
        swapped = False

        def swap_parent_at_publish(
            source_parent_fd: int,
            source_name: str,
            destination_parent_fd: int,
            destination_name: str,
            *identity: int,
        ) -> None:
            nonlocal swapped
            parent.rename(parked_parent)
            parent.symlink_to(mission.runtime_root, target_is_directory=True)
            swapped = True
            publish(
                source_parent_fd,
                source_name,
                destination_parent_fd,
                destination_name,
                *identity,
            )

        with patch.object(
            mission,
            "_publish_retirement_export_noreplace",
            side_effect=swap_parent_at_publish,
        ):
            with self.assertRaisesRegex(
                AlbertError,
                "destination|parent|boundary|failed",
            ):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="reject-late-parent-swap",
                )

        self.assertTrue(swapped)
        self.assertFalse((mission.runtime_root / destination.name).exists())
        self.assertEqual(
            (
                parked_parent / destination.name / "repository" / "tracked.txt"
            ).read_text(encoding="utf-8"),
            "baseline\n",
        )
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertNotIn(
            "reject-late-parent-swap",
            persisted.retirement.get("action_receipts", {}),
        )

    def test_parent_swap_at_materializer_never_writes_into_protected_runtime(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        parent = self.root / "materializer-swap-parent"
        parent.mkdir()
        parked_parent = self.root / "materializer-swap-parent-parked"
        destination = parent / "materializer-swap-export"
        materialize = RetirementSnapshotStore.materialize_into_directory
        protected_payloads: list[Path] = []
        original_payloads: list[Path] = []

        def swap_parent_at_materializer(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            destination_fd: int,
        ) -> None:
            runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
            intent = runtime["sessions"][completed.session_id]["retirement"][
                "export_intent"
            ]
            stage_name = intent["stage_name"]
            parent.rename(parked_parent)
            parent.symlink_to(mission.runtime_root, target_is_directory=True)
            protected_anchor = mission.runtime_root / stage_name
            protected_anchor.mkdir(mode=0o700)
            protected_payload = protected_anchor / "payload"
            protected_payload.mkdir(mode=0o700)
            protected_payloads.append(protected_payload)
            original_payloads.append(
                parked_parent / stage_name / "payload"
            )
            materialize(store, record, destination_fd)

        with patch.object(
            RetirementSnapshotStore,
            "materialize_into_directory",
            autospec=True,
            side_effect=swap_parent_at_materializer,
        ):
            with self.assertRaisesRegex(
                AlbertError,
                "destination|parent|boundary|runtime|failed",
            ):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="reject-materializer-parent-swap",
                )

        self.assertEqual(len(protected_payloads), 1)
        self.assertTrue(parent.is_symlink())
        self.assertEqual(
            tuple(protected_payloads[0].iterdir()),
            (),
            "export wrote repository or marker bytes through a swapped parent",
        )
        self.assertTrue((original_payloads[0] / "repository").is_dir())
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertNotIn(
            "reject-materializer-parent-swap",
            persisted.retirement.get("action_receipts", {}),
        )

    def test_retirement_export_never_overwrites_foreign_marker(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "foreign-marker-race-export"
        marker_path = destination / "retirement-export.json"
        foreign_marker = '{"owner":"foreign-concurrent-writer"}\n'
        original_materialize = (
            RetirementSnapshotStore.materialize_into_directory
        )
        inserted_foreign_marker = False

        def create_foreign_marker_before_publish(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            destination_fd: int,
        ) -> None:
            nonlocal inserted_foreign_marker
            original_materialize(
                store,
                record,
                destination_fd,
            )
            marker_path.parent.mkdir(parents=True, exist_ok=True)
            marker_path.write_text(foreign_marker, encoding="utf-8")
            inserted_foreign_marker = True

        with patch.object(
            RetirementSnapshotStore,
            "materialize_into_directory",
            autospec=True,
            side_effect=create_foreign_marker_before_publish,
        ):
            with self.assertRaisesRegex(
                AlbertError,
                "marker|destination|claim|publish",
            ):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="foreign-marker-must-win",
                )

        self.assertTrue(inserted_foreign_marker)
        self.assertEqual(marker_path.read_text(encoding="utf-8"), foreign_marker)
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertNotIn(
            "foreign-marker-must-win",
            persisted.retirement.get("action_receipts", {}),
        )

    def test_retirement_export_never_replaces_foreign_empty_directory(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "foreign-empty-directory-race-export"
        original_materialize = (
            RetirementSnapshotStore.materialize_into_directory
        )
        inserted_foreign_directory = False

        def create_foreign_directory_before_publish(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            destination_fd: int,
        ) -> None:
            nonlocal inserted_foreign_directory
            original_materialize(
                store,
                record,
                destination_fd,
            )
            destination.mkdir()
            inserted_foreign_directory = True

        with patch.object(
            RetirementSnapshotStore,
            "materialize_into_directory",
            autospec=True,
            side_effect=create_foreign_directory_before_publish,
        ):
            with self.assertRaisesRegex(
                AlbertError,
                "destination|exists|publisher|publish",
            ):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="foreign-empty-directory-must-win",
                )

        self.assertTrue(inserted_foreign_directory)
        self.assertTrue(destination.is_dir())
        self.assertEqual(tuple(destination.iterdir()), ())
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertNotIn(
            "foreign-empty-directory-must-win",
            persisted.retirement.get("action_receipts", {}),
        )

    def test_retirement_export_destination_has_one_session_owner(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        second_issue = self.add_issue(2)
        mission = self.load_mission()
        first_completed = self.completed_session(mission)
        second_completed = self.completed_issue(mission, second_issue)
        first_blocked = self.retirement_blocked_snapshot(
            mission,
            first_completed,
        )
        second_blocked = self.retirement_blocked_snapshot(
            mission,
            second_completed,
        )
        first_mission = self.load_mission()
        second_mission = self.load_mission()
        destination = self.root / "single-owner-export"
        marker_path = destination / "retirement-export.json"
        first_claimed = threading.Event()
        release_first = threading.Event()
        original_runtime_lock = first_mission._runtime_lock
        held_initial_claim = False

        @contextmanager
        def hold_after_first_runtime_claim(*, exclusive: bool):
            nonlocal held_initial_claim
            with original_runtime_lock(exclusive=exclusive):
                yield
            if not held_initial_claim:
                held_initial_claim = True
                first_claimed.set()
                if not release_first.wait(timeout=5):
                    raise AssertionError("timed out waiting for the competing export")

        with patch.object(
            first_mission,
            "_runtime_lock",
            side_effect=hold_after_first_runtime_claim,
        ):
            with ThreadPoolExecutor(max_workers=1) as executor:
                first = executor.submit(
                    first_mission.export_retirement_unit,
                    first_completed.session_id,
                    destination=destination,
                    expected_revision=first_blocked.revision,
                    correlation_id="first-destination-owner",
                )
                self.assertTrue(first_claimed.wait(timeout=5))
                try:
                    with self.assertRaisesRegex(
                        AlbertError,
                        "destination|claim|owner|already",
                    ):
                        second_mission.export_retirement_unit(
                            second_completed.session_id,
                            destination=destination,
                            expected_revision=second_blocked.revision,
                            correlation_id="second-destination-owner",
                        )
                finally:
                    release_first.set()
                first_receipt = first.result(timeout=5)

        self.assertEqual(
            first_receipt["session_id"],
            first_completed.session_id,
        )
        marker = json.loads(marker_path.read_text(encoding="utf-8"))
        self.assertEqual(marker["session_id"], first_completed.session_id)
        persisted = self.load_mission().sessions[second_completed.session_id]
        self.assertNotIn(
            "second-destination-owner",
            persisted.retirement.get("action_receipts", {}),
        )
        self.assertEqual(persisted.retirement.get("export_intent") or {}, {})

    def test_persisted_export_intent_owns_normalized_destination_after_lock_release(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        second_issue = self.add_issue(2)
        mission = self.load_mission()
        first_completed = self.completed_session(mission)
        second_completed = self.completed_issue(mission, second_issue)
        first_blocked = self.retirement_blocked_snapshot(mission, first_completed)
        second_blocked = self.retirement_blocked_snapshot(mission, second_completed)
        destination = self.root / "persisted-owner-export"
        materialization_identities: list[tuple[int, int]] = []

        def interrupt_after_durable_claim(
            _store: RetirementSnapshotStore,
            _record: dict[str, object],
            destination_fd: int,
        ) -> None:
            status = os.fstat(destination_fd)
            materialization_identities.append((status.st_dev, status.st_ino))
            raise KeyboardInterrupt("crash after durable destination claim")

        first_exporter = self.load_mission()
        with patch.object(
            RetirementSnapshotStore,
            "materialize_into_directory",
            autospec=True,
            side_effect=interrupt_after_durable_claim,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "durable destination"):
                first_exporter.export_retirement_unit(
                    first_completed.session_id,
                    destination=destination,
                    expected_revision=first_blocked.revision,
                    correlation_id="persist-first-destination-owner",
                )

        self.assertEqual(len(materialization_identities), 1)
        self.assertFalse(destination.exists())
        first_persisted = self.load_mission().sessions[first_completed.session_id]
        self.assertEqual(
            first_persisted.retirement["export_intent"]["destination"],
            str(destination.resolve()),
        )
        normalized_alias = (
            destination.parent
            / "unused-normalization-component"
            / ".."
            / destination.name
        )

        with self.assertRaisesRegex(
            AlbertError,
            "destination|claim|owner|intent|already",
        ):
            self.load_mission().export_retirement_unit(
                second_completed.session_id,
                destination=normalized_alias,
                expected_revision=second_blocked.revision,
                correlation_id="persist-second-destination-owner",
            )

        second_persisted = self.load_mission().sessions[second_completed.session_id]
        self.assertEqual(second_persisted.retirement.get("export_intent") or {}, {})
        self.assertNotIn(
            "persist-second-destination-owner",
            second_persisted.retirement.get("action_receipts", {}),
        )
        self.assertFalse(destination.exists())

    def test_completed_export_receipt_keeps_destination_ownership_after_removal(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        second_issue = self.add_issue(2)
        mission = self.load_mission()
        first_completed = self.completed_session(mission)
        second_completed = self.completed_issue(mission, second_issue)
        first_blocked = self.retirement_blocked_snapshot(mission, first_completed)
        second_blocked = self.retirement_blocked_snapshot(mission, second_completed)
        destination = self.root / "receipt-owned-export"
        receipt = mission.export_retirement_unit(
            first_completed.session_id,
            destination=destination,
            expected_revision=first_blocked.revision,
            correlation_id="complete-first-destination-owner",
        )
        self.assertEqual(
            receipt["destination"],
            str(destination.resolve(strict=False) / "repository"),
        )
        parked_destination = self.root / "receipt-owned-export-parked"
        destination.rename(parked_destination)
        normalized_alias = (
            destination.parent / "receipt-alias" / ".." / destination.name
        )

        with self.assertRaisesRegex(
            AlbertError,
            "destination|claim|owner|receipt|already",
        ):
            self.load_mission().export_retirement_unit(
                second_completed.session_id,
                destination=normalized_alias,
                expected_revision=second_blocked.revision,
                correlation_id="reject-owner-after-destination-move",
            )
        self.assertTrue(parked_destination.is_dir())

        shutil.rmtree(parked_destination)
        with self.assertRaisesRegex(
            AlbertError,
            "destination|claim|owner|receipt|already",
        ):
            self.load_mission().export_retirement_unit(
                second_completed.session_id,
                destination=normalized_alias,
                expected_revision=second_blocked.revision,
                correlation_id="reject-owner-after-destination-delete",
            )

        second_persisted = self.load_mission().sessions[second_completed.session_id]
        self.assertEqual(second_persisted.retirement.get("export_intent") or {}, {})
        self.assertNotIn(
            "reject-owner-after-destination-move",
            second_persisted.retirement.get("action_receipts", {}),
        )
        self.assertNotIn(
            "reject-owner-after-destination-delete",
            second_persisted.retirement.get("action_receipts", {}),
        )
        self.assertFalse(destination.exists())

    def test_crash_after_export_owner_before_session_intent_recovers_exact_request(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "owner-before-intent-export"
        correlation_id = "owner-before-intent-export"
        write_payload = mission._write_runtime_payload
        owner_names_at_crash: list[tuple[str, ...]] = []

        def interrupt_before_export_intent(data: dict[str, object]) -> None:
            raw_session = data.get("sessions", {}).get(completed.session_id, {})
            intent = raw_session.get("retirement", {}).get("export_intent", {})
            if intent and not owner_names_at_crash:
                owner_names = tuple(
                    mission._retirement_export_owner_directories()
                )
                self.assertEqual(len(owner_names), 1)
                owner_path = (
                    mission.runtime_root
                    / ".retirement-export-locks"
                    / owner_names[0]
                )
                self.assertEqual(tuple(owner_path.iterdir()), ())
                self.assertEqual(
                    tuple(
                        destination.parent.glob(
                            ".alfredo-retirement-export.*.stage"
                        )
                    ),
                    (),
                )
                owner_names_at_crash.append(owner_names)
                raise KeyboardInterrupt(
                    "crash after destination owner before session intent"
                )
            write_payload(data)

        with patch.object(
            mission,
            "_write_runtime_payload",
            side_effect=interrupt_before_export_intent,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "before session intent"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id=correlation_id,
                )

        self.assertEqual(len(owner_names_at_crash), 1)
        owner_names = owner_names_at_crash[0]
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertEqual(persisted.revision, blocked.revision)
        self.assertEqual(persisted.retirement.get("export_intent") or {}, {})
        self.assertFalse(destination.exists())

        with self.assertRaisesRegex(
            LaunchBlockedError,
            "action in progress|export owner",
        ):
            self.load_mission().set_retirement_snapshot_pin(
                completed.session_id,
                pinned=True,
                expected_revision=blocked.revision,
                correlation_id="pin-must-not-bypass-unfinished-export-owner",
            )

        different_destination = self.root / "different-owner-before-intent-export"
        with self.assertRaisesRegex(
            LaunchBlockedError,
            "different durable export owner|action in progress",
        ):
            self.load_mission().export_retirement_unit(
                completed.session_id,
                destination=different_destination,
                expected_revision=blocked.revision,
                correlation_id="different-owner-before-intent-export",
            )

        still_blocked = self.load_mission().sessions[completed.session_id]
        self.assertEqual(still_blocked.revision, blocked.revision)
        self.assertEqual(
            still_blocked.retirement["snapshot"]["pinned"],
            blocked.retirement["snapshot"]["pinned"],
        )
        self.assertEqual(still_blocked.retirement.get("export_intent") or {}, {})
        self.assertEqual(
            tuple(self.load_mission()._retirement_export_owner_directories()),
            owner_names,
        )
        self.assertFalse(different_destination.exists())

        receipt = self.load_mission().export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id=correlation_id,
        )

        self.assertEqual(receipt["result_revision"], blocked.revision + 2)
        self.assertEqual(
            receipt["destination"],
            str(destination.resolve(strict=True) / "repository"),
        )
        self.assertEqual(
            tuple(self.load_mission()._retirement_export_owner_directories()),
            owner_names,
        )
        anchors = tuple(
            destination.parent.glob(".alfredo-retirement-export.*.stage")
        )
        self.assertEqual(len(anchors), 1)
        self.assertEqual(tuple(anchors[0].iterdir()), ())

    def test_retirement_export_retry_recovers_partial_repository_crash(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "partial-repository-crash-export"
        staging_roots: list[Path] = []

        def interrupt_partial_materialization(
            _store: RetirementSnapshotStore,
            _record: dict[str, object],
            destination_fd: int,
        ) -> None:
            destination_root = mission._bound_retirement_export_directory_path(
                destination_fd
            )
            staging_roots.append(destination_root)
            os.mkdir("repository", mode=0o700, dir_fd=destination_fd)
            repository_fd = mission._open_retirement_export_directory(
                "repository",
                dir_fd=destination_fd,
            )
            try:
                partial_fd = os.open(
                    "partial.txt",
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                    0o600,
                    dir_fd=repository_fd,
                )
                try:
                    os.write(
                        partial_fd,
                        b"incomplete export from interrupted materialization\n",
                    )
                finally:
                    os.close(partial_fd)
            finally:
                os.close(repository_fd)
            raise KeyboardInterrupt("crash with a partial exported repository")

        with patch.object(
            RetirementSnapshotStore,
            "materialize_into_directory",
            autospec=True,
            side_effect=interrupt_partial_materialization,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "partial exported"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="retry-partial-repository-export",
                )

        self.assertEqual(len(staging_roots), 1)
        partial_file = staging_roots[0] / "repository" / "partial.txt"
        self.assertEqual(
            staging_roots[0].parent.parent,
            destination.parent.resolve(),
        )
        self.assertNotEqual(staging_roots[0], destination.resolve())
        self.assertTrue(partial_file.is_file())
        self.assertFalse(destination.exists())
        recovered = self.load_mission()
        receipt = recovered.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="retry-partial-repository-export",
        )

        self.assertEqual(
            receipt["destination"],
            str(destination.resolve() / "repository"),
        )
        self.assertTrue(partial_file.exists())
        self.assertEqual(
            (destination / "repository" / "tracked.txt").read_text(
                encoding="utf-8"
            ),
            "baseline\n",
        )

    def test_retirement_export_retry_never_deletes_replaced_staging_tree(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "replaced-staging-retry-export"
        staging_roots: list[Path] = []

        def interrupt_after_stage_capture(
            _store: RetirementSnapshotStore,
            _record: dict[str, object],
            destination_fd: int,
        ) -> None:
            destination_root = mission._bound_retirement_export_directory_path(
                destination_fd
            )
            staging_roots.append(destination_root)
            partial_fd = os.open(
                "partial-foreign.txt",
                os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                0o600,
                dir_fd=destination_fd,
            )
            try:
                os.write(partial_fd, b"interrupted original stage\n")
            finally:
                os.close(partial_fd)
            raise KeyboardInterrupt("crash after staging identity was persisted")

        with patch.object(
            RetirementSnapshotStore,
            "materialize_into_directory",
            autospec=True,
            side_effect=interrupt_after_stage_capture,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "staging identity"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="retry-replaced-staging-export",
                )

        self.assertEqual(len(staging_roots), 1)
        recorded_stage = staging_roots[0].parent
        parked_stage = recorded_stage.with_name(recorded_stage.name + ".parked")
        recorded_stage.rename(parked_stage)
        recorded_stage.mkdir(mode=0o700)
        replacement_sentinel = recorded_stage / "replacement-owner.txt"
        replacement_sentinel.write_text(
            "replacement stage must never be deleted\n",
            encoding="utf-8",
        )

        receipt = self.load_mission().export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="retry-replaced-staging-export",
        )

        self.assertEqual(
            receipt["destination"],
            str(destination.resolve() / "repository"),
        )
        self.assertEqual(
            replacement_sentinel.read_text(encoding="utf-8"),
            "replacement stage must never be deleted\n",
        )
        self.assertEqual(
            (parked_stage / "payload" / "partial-foreign.txt").read_text(
                encoding="utf-8"
            ),
            "interrupted original stage\n",
        )

    def test_retirement_export_materializer_borrows_stage_fd_without_closing_it(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "stage-fd-hygiene-export"
        materialize = RetirementSnapshotStore.materialize_into_directory
        borrowed_descriptors: list[int] = []

        def observe_borrowed_descriptor(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            destination_fd: int,
        ) -> None:
            before = os.fstat(destination_fd)
            materialize(store, record, destination_fd)
            after = os.fstat(destination_fd)
            self.assertEqual(
                (after.st_dev, after.st_ino),
                (before.st_dev, before.st_ino),
            )
            borrowed_descriptors.append(destination_fd)

        with patch.object(
            RetirementSnapshotStore,
            "materialize_into_directory",
            autospec=True,
            side_effect=observe_borrowed_descriptor,
        ):
            receipt = mission.export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id="verify-borrowed-materializer-descriptor",
            )

        self.assertEqual(
            receipt["destination"],
            str(destination.resolve(strict=False) / "repository"),
        )
        self.assertEqual(len(borrowed_descriptors), 1)
        with self.assertRaises(OSError) as error:
            os.fstat(borrowed_descriptors[0])
        self.assertEqual(error.exception.errno, errno.EBADF)

    def test_retry_stage_persistence_crash_does_not_accumulate_orphan_stages(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "bounded-retry-stage-export"

        def interrupt_with_invalid_first_stage(
            _store: RetirementSnapshotStore,
            _record: dict[str, object],
            destination_fd: int,
        ) -> None:
            partial_fd = os.open(
                "partial-attempt-one.txt",
                os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                0o600,
                dir_fd=destination_fd,
            )
            try:
                os.write(partial_fd, b"attempt one is incomplete\n")
            finally:
                os.close(partial_fd)
            raise KeyboardInterrupt("crash with invalid attempt-one stage")

        with patch.object(
            RetirementSnapshotStore,
            "materialize_into_directory",
            autospec=True,
            side_effect=interrupt_with_invalid_first_stage,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "attempt-one"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="bounded-retry-stage-owner",
                )

        first_intent = self.load_mission().sessions[completed.session_id].retirement[
            "export_intent"
        ]
        self.assertEqual(first_intent["stage_attempt"], 1)
        stage_pattern = ".alfredo-retirement-export.*.stage"
        initial_stages = set(destination.parent.glob(stage_pattern))
        self.assertEqual(len(initial_stages), 1)

        for crash_number in range(2):
            recovered = self.load_mission()
            write_payload = recovered._write_runtime_payload

            def interrupt_attempt_two_persistence(data: dict[str, object]) -> None:
                raw_session = data.get("sessions", {}).get(
                    completed.session_id,
                    {},
                )
                intent = raw_session.get("retirement", {}).get(
                    "export_intent",
                    {},
                )
                if intent.get("stage_attempt") == 2:
                    raise KeyboardInterrupt(
                        "crash before retry stage identity persistence"
                    )
                write_payload(data)

            with patch.object(
                recovered,
                "_write_runtime_payload",
                side_effect=interrupt_attempt_two_persistence,
            ):
                with self.assertRaisesRegex(
                    KeyboardInterrupt,
                    "retry stage identity",
                ):
                    recovered.export_retirement_unit(
                        completed.session_id,
                        destination=destination,
                        expected_revision=blocked.revision,
                        correlation_id="bounded-retry-stage-owner",
                    )

            persisted = self.load_mission().sessions[completed.session_id]
            self.assertEqual(
                persisted.retirement["export_intent"]["stage_attempt"],
                1,
            )
            self.assertEqual(
                set(destination.parent.glob(stage_pattern)),
                initial_stages,
                f"crash {crash_number + 1} left an unowned retry stage",
            )

    def test_retirement_export_rejects_symlink_to_missing_destination(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "foreign-missing-destination-link"
        missing_target = self.root / "missing-destination-target"
        destination.symlink_to(missing_target, target_is_directory=True)

        with self.assertRaisesRegex(
            AlbertError,
            "destination|symlink|exist|boundary",
        ):
            mission.export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id="reject-missing-target-destination-link",
            )

        self.assertTrue(destination.is_symlink())
        self.assertFalse(missing_target.exists())
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertEqual(persisted.retirement.get("export_intent") or {}, {})
        self.assertNotIn(
            "reject-missing-target-destination-link",
            persisted.retirement.get("action_receipts", {}),
        )

    def test_retained_worktree_export_retry_recovers_truncated_marker_crash(
        self,
    ) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        (completed.worktree_path / "direct-export.txt").write_text(
            "exact direct retained content\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-truncated-export-marker",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        quiesced = self.load_mission()
        destination = self.root / "truncated-marker-crash-export"
        staging_marker_paths: list[Path] = []
        original_write = quiesced._write_retirement_export_marker_exclusive
        interrupted = False

        def interrupt_during_marker_write(
            stage_fd: int,
            path: Path,
            content: str,
        ) -> None:
            nonlocal interrupted
            if not interrupted:
                stage_root = quiesced._bound_retirement_export_directory_path(
                    stage_fd
                )
                marker_path = stage_root / path.name
                staging_marker_paths.append(marker_path)
                marker_fd = os.open(
                    path.name,
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                    0o600,
                    dir_fd=stage_fd,
                )
                try:
                    os.write(marker_fd, b'{"schema_version":')
                finally:
                    os.close(marker_fd)
                interrupted = True
                raise KeyboardInterrupt("crash with a truncated export marker")
            original_write(stage_fd, path, content)

        with patch.object(
            quiesced,
            "_write_retirement_export_marker_exclusive",
            side_effect=interrupt_during_marker_write,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "truncated export marker"):
                quiesced.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="retry-truncated-export-marker",
                )

        self.assertTrue(interrupted)
        self.assertEqual(len(staging_marker_paths), 1)
        marker_path = staging_marker_paths[0]
        self.assertNotEqual(marker_path.parent, destination.resolve())
        self.assertFalse(destination.exists())
        self.assertEqual(
            marker_path.read_text(encoding="utf-8"),
            '{"schema_version":',
        )
        recovered = self.load_mission()
        receipt = recovered.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="retry-truncated-export-marker",
        )

        self.assertEqual(
            receipt["destination"],
            str(destination.resolve() / "repository"),
        )
        self.assertEqual(
            (destination / "repository" / "direct-export.txt").read_text(
                encoding="utf-8"
            ),
            "exact direct retained content\n",
        )

    def test_retirement_export_rechecks_artifacts_before_receipt(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        second_issue = self.add_issue(2)
        mission = self.load_mission()
        completed_sessions = (
            self.completed_session(mission),
            self.completed_issue(mission, second_issue),
        )
        blocked_sessions = tuple(
            self.retirement_blocked_snapshot(mission, completed)
            for completed in completed_sessions
        )

        for index, (mutation, completed, blocked) in enumerate(
            zip(
                ("tamper-repository", "remove-marker"),
                completed_sessions,
                blocked_sessions,
            ),
            start=1,
        ):
            with self.subTest(mutation=mutation):
                exporter = self.load_mission()
                destination = self.root / f"pre-receipt-mutation-export-{index}"
                marker_path = destination / "retirement-export.json"
                original_runtime_lock = exporter._runtime_lock
                mutated = False

                @contextmanager
                def mutate_before_receipt(*, exclusive: bool):
                    nonlocal mutated
                    if marker_path.is_file() and not mutated:
                        if mutation == "tamper-repository":
                            (destination / "repository" / "tracked.txt").write_text(
                                "tampered between marker and receipt\n",
                                encoding="utf-8",
                            )
                        else:
                            marker_path.unlink()
                        mutated = True
                    with original_runtime_lock(exclusive=exclusive):
                        yield

                correlation_id = f"reject-pre-receipt-{mutation}"
                with patch.object(
                    exporter,
                    "_runtime_lock",
                    side_effect=mutate_before_receipt,
                ):
                    with self.assertRaisesRegex(
                        AlbertError,
                        "marker|repository|content|destination|changed",
                    ):
                        exporter.export_retirement_unit(
                            completed.session_id,
                            destination=destination,
                            expected_revision=blocked.revision,
                            correlation_id=correlation_id,
                        )

                self.assertTrue(mutated)
                persisted = self.load_mission().sessions[completed.session_id]
                self.assertNotIn(
                    correlation_id,
                    persisted.retirement.get("action_receipts", {}),
                )

    def test_retirement_export_rechecks_top_level_after_final_repository_verifier(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        exporter = self.load_mission()
        destination = self.root / "foreign-after-final-verifier-export"
        marker_path = destination / "retirement-export.json"
        foreign_entry = destination / "foreign-after-final-verifier.txt"
        verify_repository = (
            RetirementSnapshotStore.verify_materialized_repository_in_directory
        )
        original_runtime_lock = exporter._runtime_lock
        receipt_phase = False
        inserted_foreign_entry = False

        @contextmanager
        def mark_receipt_phase(*, exclusive: bool):
            nonlocal receipt_phase
            final_receipt_lock = marker_path.is_file()
            if final_receipt_lock:
                receipt_phase = True
            try:
                with original_runtime_lock(exclusive=exclusive):
                    yield
            finally:
                if final_receipt_lock:
                    receipt_phase = False

        def insert_after_final_repository_verifier(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            repository_fd: int,
        ) -> bool:
            nonlocal inserted_foreign_entry
            verified = verify_repository(store, record, repository_fd)
            repository = exporter._bound_retirement_export_directory_path(
                repository_fd
            )
            if (
                receipt_phase
                and repository.parent == destination.resolve(strict=True)
            ):
                foreign_entry.write_text(
                    "foreign top-level entry after final repository verification\n",
                    encoding="utf-8",
                )
                inserted_foreign_entry = True
            return verified

        with (
            patch.object(
                exporter,
                "_runtime_lock",
                side_effect=mark_receipt_phase,
            ),
            patch.object(
                RetirementSnapshotStore,
                "verify_materialized_repository_in_directory",
                autospec=True,
                side_effect=insert_after_final_repository_verifier,
            ),
        ):
            with self.assertRaisesRegex(
                AlbertError,
                "destination|boundary|entry|changed",
            ):
                exporter.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="reject-foreign-after-final-verifier",
                )

        self.assertTrue(inserted_foreign_entry)
        self.assertEqual(
            foreign_entry.read_text(encoding="utf-8"),
            "foreign top-level entry after final repository verification\n",
        )
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertNotIn(
            "reject-foreign-after-final-verifier",
            persisted.retirement.get("action_receipts", {}),
        )

    def test_retirement_export_syncs_tree_before_publication_and_receipt(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "durable-ordering-export"
        correlation_id = "durable-ordering-export"
        sync_tree = mission._durably_sync_retirement_export_tree
        publish = mission._publish_retirement_export_noreplace
        write_payload = mission._write_runtime_payload
        events: list[str] = []
        stage_sync_complete = False
        public_sync_complete = False

        def record_tree_sync(root_fd: int) -> None:
            nonlocal stage_sync_complete, public_sync_complete
            sync_tree(root_fd)
            self.assertEqual(
                set(os.listdir(root_fd)),
                {"repository", "retirement-export.json"},
            )
            if destination.exists():
                destination_status = destination.stat(follow_symlinks=False)
                root_status = os.fstat(root_fd)
                self.assertEqual(
                    (root_status.st_dev, root_status.st_ino),
                    (destination_status.st_dev, destination_status.st_ino),
                )
                public_sync_complete = True
                events.append("public-tree-sync")
            else:
                stage_sync_complete = True
                events.append("private-tree-sync")

        def record_publish(
            source_parent_fd: int,
            source_name: str,
            destination_parent_fd: int,
            destination_name: str,
            *identity: int,
        ) -> None:
            self.assertTrue(stage_sync_complete)
            self.assertFalse(destination.exists())
            self.assertEqual(source_name, "payload")
            self.assertEqual(destination_name, destination.name)
            publish(
                source_parent_fd,
                source_name,
                destination_parent_fd,
                destination_name,
                *identity,
            )
            events.append("publish")

        def require_public_sync_before_receipt(data: dict[str, object]) -> None:
            raw_session = data.get("sessions", {}).get(completed.session_id, {})
            receipt = raw_session.get("retirement", {}).get(
                "action_receipts", {}
            ).get(correlation_id)
            if receipt:
                self.assertTrue(public_sync_complete)
                events.append("receipt")
            write_payload(data)

        with (
            patch.object(
                mission,
                "_durably_sync_retirement_export_tree",
                side_effect=record_tree_sync,
            ),
            patch.object(
                mission,
                "_publish_retirement_export_noreplace",
                side_effect=record_publish,
            ),
            patch.object(
                mission,
                "_write_runtime_payload",
                side_effect=require_public_sync_before_receipt,
            ),
        ):
            receipt = mission.export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id=correlation_id,
            )

        self.assertEqual(
            receipt["destination"],
            str(destination.resolve(strict=True) / "repository"),
        )
        self.assertEqual(
            events,
            ["private-tree-sync", "publish", "public-tree-sync", "receipt"],
        )

    def test_retirement_export_rejects_valid_timestamp_marker_substitution(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        blocked = self.retirement_blocked_snapshot(mission, completed)
        destination = self.root / "substituted-marker-timestamp-export"
        correlation_id = "reject-substituted-marker-timestamp"
        write_payload = mission._write_runtime_payload

        def interrupt_before_export_receipt(data: dict[str, object]) -> None:
            raw_session = data.get("sessions", {}).get(completed.session_id, {})
            receipt = raw_session.get("retirement", {}).get(
                "action_receipts", {}
            ).get(correlation_id)
            if receipt:
                raise KeyboardInterrupt("crash before timestamp-bound receipt")
            write_payload(data)

        with patch.object(
            mission,
            "_write_runtime_payload",
            side_effect=interrupt_before_export_receipt,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "timestamp-bound receipt"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id=correlation_id,
                )

        marker_path = destination / "retirement-export.json"
        marker = json.loads(marker_path.read_text(encoding="utf-8"))
        original_exported_at = marker["exported_at"]
        marker["exported_at"] = (
            datetime.fromisoformat(original_exported_at) + timedelta(days=1)
        ).isoformat()
        marker_path.write_text(
            json.dumps(marker, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

        with self.assertRaisesRegex(
            AlbertError,
            "marker|binding|changed|invalid",
        ):
            self.load_mission().export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id=correlation_id,
            )

        self.assertNotEqual(marker["exported_at"], original_exported_at)
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertNotIn(
            correlation_id,
            persisted.retirement.get("action_receipts", {}),
        )

    def test_retained_source_replacement_after_publication_prevents_export_receipt(
        self,
    ) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        (completed.worktree_path / "retained-source.txt").write_text(
            "original retained source\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-source-publication-swap",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        exporter = self.load_mission()
        destination = self.root / "source-replaced-before-receipt-export"
        marker_path = destination / "retirement-export.json"
        parked_source = completed.worktree_path.with_name(
            completed.worktree_path.name + ".parked"
        )
        replacement_sentinel = completed.worktree_path / "replacement-owner.txt"
        original_runtime_lock = exporter._runtime_lock
        replaced_source = False

        @contextmanager
        def replace_source_before_receipt(*, exclusive: bool):
            nonlocal replaced_source
            if marker_path.is_file() and not replaced_source:
                completed.worktree_path.rename(parked_source)
                completed.worktree_path.mkdir(mode=0o700)
                replacement_sentinel.write_text(
                    "replacement source must not authorize a receipt\n",
                    encoding="utf-8",
                )
                replaced_source = True
            with original_runtime_lock(exclusive=exclusive):
                yield

        with patch.object(
            exporter,
            "_runtime_lock",
            side_effect=replace_source_before_receipt,
        ):
            with self.assertRaisesRegex(
                AlbertError,
                "source|identity|boundary|changed",
            ):
                exporter.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="reject-source-replacement-before-receipt",
                )

        self.assertTrue(replaced_source)
        self.assertTrue(parked_source.is_dir())
        self.assertEqual(
            replacement_sentinel.read_text(encoding="utf-8"),
            "replacement source must not authorize a receipt\n",
        )
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertNotIn(
            "reject-source-replacement-before-receipt",
            persisted.retirement.get("action_receipts", {}),
        )

    def test_preservation_blocked_exports_the_exact_retained_worktree(self) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        (completed.worktree_path / "retained-only.txt").write_text(
            "recover this exact retained worktree\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preservation-blocked-actions",
            )

        blocked = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(blocked.retirement["phase"], "preservation-blocked")
        quiesced = self.load_mission()
        inspection = quiesced.inspect_retirement_unit(completed.session_id)
        self.assertEqual(
            inspection["actions"],
            {"retry": True, "inspect": True, "export": True, "discard": True},
        )

        destination = self.root / "preservation-blocked-export"
        write_payload = quiesced._write_runtime_payload

        def interrupt_before_direct_export_receipt(data: dict[str, object]) -> None:
            raw_session = data.get("sessions", {}).get(completed.session_id, {})
            receipt = raw_session.get("retirement", {}).get(
                "action_receipts", {}
            ).get("export-preservation-blocked-worktree")
            if receipt:
                raise KeyboardInterrupt("crash before direct export receipt")
            write_payload(data)

        with patch.object(
            quiesced,
            "_write_runtime_payload",
            side_effect=interrupt_before_direct_export_receipt,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "direct export receipt"):
                quiesced.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="export-preservation-blocked-worktree",
                )

        exported_file = destination / "repository" / "retained-only.txt"
        exported_file.write_text("tampered direct export\n", encoding="utf-8")
        recovered = self.load_mission()
        with self.assertRaisesRegex(AlbertError, "exported repository content"):
            recovered.export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id="export-preservation-blocked-worktree",
            )
        exported_file.write_text(
            "recover this exact retained worktree\n",
            encoding="utf-8",
        )
        receipt = recovered.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="export-preservation-blocked-worktree",
        )

        self.assertEqual(receipt["destination"], str(destination.resolve() / "repository"))
        self.assertEqual(
            (destination / "repository" / "retained-only.txt").read_text(encoding="utf-8"),
            "recover this exact retained worktree\n",
        )

    def test_preservation_blocked_inspection_hides_unavailable_export(self) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-missing-source-inspection",
            )
        shutil.rmtree(completed.worktree_path)

        inspection = self.load_mission().inspect_retirement_unit(
            completed.session_id
        )

        self.assertFalse(inspection["actions"]["export"])
        self.assertTrue(inspection["actions"]["discard"])

    def test_preservation_blocked_inspection_hides_actions_until_quiescent(
        self,
    ) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-live-inspection",
            )

        inspection = mission.inspect_retirement_unit(completed.session_id)

        self.assertEqual(
            inspection["actions"],
            {"retry": True, "inspect": True, "export": False, "discard": False},
        )

        quiesced = self.load_mission().inspect_retirement_unit(
            completed.session_id
        )
        self.assertEqual(
            quiesced["actions"],
            {"retry": True, "inspect": True, "export": True, "discard": True},
        )

    def test_retained_worktree_export_fails_closed_on_unreadable_subtree(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        blocked_mission = self.load_mission(
            quiescence=("absent", "live-exact")
        )
        completed = self.completed_session(blocked_mission)
        restricted = completed.worktree_path / "execute-only"
        restricted.mkdir()
        hidden = restricted / "hidden.txt"
        hidden.write_text("must never be omitted\n", encoding="utf-8")
        restricted.chmod(0o100)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            blocked_mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-unreadable-export",
            )
        blocked = blocked_mission._refresh_persisted_session(
            completed.session_id
        )
        destination = self.root / "unreadable-retained-export"

        try:
            with self.assertRaisesRegex(
                AlbertError,
                "could not inspect",
            ):
                self.load_mission().export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="reject-unreadable-retained-export",
                )
            with self.assertRaisesRegex(
                RetirementSnapshotError,
                "could not inspect",
            ):
                self.load_mission().discard_retained_worktree(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="reject-unreadable-retained-discard",
                    confirmation=completed.session_id,
                    reason="An unreadable subtree must never be omitted from authority.",
                )
        finally:
            if restricted.exists():
                restricted.chmod(0o700)

        self.assertEqual(hidden.read_text(encoding="utf-8"), "must never be omitted\n")
        self.assertFalse(destination.exists())
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertEqual(persisted.retirement.get("discard_intent") or {}, {})

    def test_retained_worktree_export_preserves_read_only_directory(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        blocked_mission = self.load_mission(
            quiescence=("absent", "live-exact")
        )
        completed = self.completed_session(blocked_mission)
        read_only = completed.worktree_path / "read-only"
        read_only.mkdir()
        retained_file = read_only / "retained.txt"
        retained_file.write_text("preserve read-only content\n", encoding="utf-8")
        read_only.chmod(0o555)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            blocked_mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-read-only-export",
            )
        blocked = blocked_mission._refresh_persisted_session(
            completed.session_id
        )
        destination = self.root / "read-only-retained-export"
        exported_directory = destination / "repository" / "read-only"

        try:
            receipt = self.load_mission().export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id="export-read-only-retained-tree",
            )
            self.assertEqual(
                receipt["destination"],
                str(destination.resolve() / "repository"),
            )
            self.assertEqual(
                (exported_directory / "retained.txt").read_text(encoding="utf-8"),
                "preserve read-only content\n",
            )
            self.assertEqual(
                exported_directory.stat(follow_symlinks=False).st_mode & 0o777,
                0o555,
            )
        finally:
            if read_only.exists():
                read_only.chmod(0o700)
            if exported_directory.exists():
                exported_directory.chmod(0o700)

    @unittest.skipUnless(
        sys.platform.startswith("linux"),
        "requires a filesystem that accepts undecodable POSIX byte names",
    )
    def test_retained_worktree_actions_round_trip_non_utf8_names(self) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        filename = b"retained-\xff.bin"
        linkname = b"link-\xfe"
        link_target = b"missing-target-\xfd"
        source_root = os.fsencode(completed.worktree_path)
        source_file = os.path.join(source_root, filename)
        descriptor = os.open(
            source_file,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL,
            0o640,
        )
        try:
            os.write(descriptor, b"non-UTF-8 retained bytes\n")
        finally:
            os.close(descriptor)
        os.symlink(link_target, os.path.join(source_root, linkname))
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-non-utf8-actions",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        destination = self.root / "non-utf8-retained-export"

        self.load_mission().export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="export-non-utf8-retained-tree",
        )

        exported_root = os.fsencode(destination / "repository")
        exported_descriptor = os.open(
            os.path.join(exported_root, filename),
            os.O_RDONLY,
        )
        try:
            self.assertEqual(
                os.read(exported_descriptor, 1024),
                b"non-UTF-8 retained bytes\n",
            )
        finally:
            os.close(exported_descriptor)
        self.assertEqual(
            os.readlink(os.path.join(exported_root, linkname)),
            link_target,
        )
        quiesced = self.load_mission()
        after_export = quiesced._refresh_persisted_session(completed.session_id)
        discarded = quiesced.discard_retained_worktree(
            completed.session_id,
            expected_revision=after_export.revision,
            correlation_id="discard-non-utf8-retained-tree",
            confirmation=completed.session_id,
            reason="Delete the exact byte-named retained tree after export.",
        )

        self.assertEqual(discarded.retirement["phase"], "retired")
        self.assertFalse(completed.worktree_path.exists())

    @unittest.skipIf(os.name == "nt", "backslash is a Windows path separator")
    def test_retained_worktree_actions_round_trip_backslash_name(self) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        retained = completed.worktree_path / "retained\\backslash.txt"
        retained.write_text("legal POSIX backslash name\n", encoding="utf-8")
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-backslash-actions",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        destination = self.root / "backslash-retained-export"
        quiesced = self.load_mission()

        quiesced.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="export-backslash-retained-tree",
        )

        self.assertEqual(
            (destination / "repository" / "retained\\backslash.txt").read_text(
                encoding="utf-8"
            ),
            "legal POSIX backslash name\n",
        )
        after_export = quiesced._refresh_persisted_session(completed.session_id)
        discarded = quiesced.discard_retained_worktree(
            completed.session_id,
            expected_revision=after_export.revision,
            correlation_id="discard-backslash-retained-tree",
            confirmation=completed.session_id,
            reason="Delete the exact retained tree with a legal POSIX name.",
        )
        self.assertEqual(discarded.retirement["phase"], "retired")

    @unittest.skipIf(os.name == "nt", "FIFO entries require a POSIX filesystem")
    def test_retained_worktree_manifest_rejects_fifo_without_blocking(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        fifo = completed.worktree_path / "retained.pipe"
        os.mkfifo(fifo)
        open_file = os.open

        def reject_blocking_fifo_open(path, flags, *args, **kwargs):
            if os.fspath(path) == os.fspath(fifo) and not (
                flags & getattr(os, "O_NONBLOCK", 0)
            ):
                raise AssertionError("FIFO inspection attempted a blocking open")
            return open_file(path, flags, *args, **kwargs)

        with patch(
            "albert_mvp.retirement.os.open",
            side_effect=reject_blocking_fifo_open,
        ):
            with self.assertRaisesRegex(
                RetirementSnapshotError,
                "unsupported entry",
            ):
                RetirementSnapshotStore.retained_worktree_manifest(
                    completed.worktree_path,
                    exclude_git_metadata=True,
                )

    def test_preservation_blocked_retry_runs_fresh_preservation_and_reloads(
        self,
    ) -> None:
        blocked_mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(blocked_mission)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            blocked_mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-explicit-retry",
            )
        blocked = blocked_mission._refresh_persisted_session(completed.session_id)

        quiesced = self.load_mission()
        retried = quiesced.retry_retirement_unit(
            completed.session_id,
            expected_revision=blocked.revision,
            correlation_id="retry-preservation-blocked",
        )

        self.assertEqual(retried.retirement["phase"], "preserved")
        self.assertTrue(retried.retirement["snapshot"]["verified"])
        self.assertEqual(
            retried.retirement["action_receipts"]["retry-preservation-blocked"]["result_phase"],
            "preserved",
        )
        reloaded = self.load_mission().sessions[completed.session_id]
        self.assertEqual(reloaded.retirement["phase"], "preserved")

    def test_preservation_blocked_discard_does_not_require_a_snapshot(self) -> None:
        blocked_mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(blocked_mission)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            blocked_mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-direct-discard",
            )
        blocked = blocked_mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(blocked.retirement["snapshot"], {})

        quiesced = self.load_mission()
        discarded = quiesced.discard_retained_worktree(
            completed.session_id,
            expected_revision=blocked.revision,
            correlation_id="discard-preservation-blocked-worktree",
            confirmation=completed.session_id,
            reason="Mission Commander explicitly discards unpreserved retained work.",
        )

        self.assertEqual(discarded.retirement["phase"], "retired")
        self.assertFalse(completed.worktree_path.exists())

    def test_preservation_blocked_export_uses_exact_path_when_git_identity_is_broken(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        git_pointer = completed.worktree_path / ".git"
        git_pointer.write_text("gitdir: /missing/retained-worktree-admin\n", encoding="utf-8")
        with self.assertRaisesRegex(LaunchBlockedError, "Worktree Identity"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-with-broken-git-pointer",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        destination = self.root / "broken-identity-export"

        receipt = mission.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="export-broken-git-identity",
        )

        self.assertEqual(
            receipt["destination"], str(destination.resolve() / "repository")
        )
        self.assertEqual(
            (destination / "repository" / "tracked.txt").read_text(encoding="utf-8"),
            "baseline\n",
        )
        self.assertFalse((destination / "repository" / ".git").exists())

    def test_preservation_blocked_export_preserves_replacement_dot_git_directory(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        git_pointer = completed.worktree_path / ".git"
        git_pointer.unlink()
        git_pointer.mkdir()
        replacement = git_pointer / "retained-user-data.txt"
        replacement.write_text(
            "a replacement directory is retained data\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(LaunchBlockedError, "Worktree Identity"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-with-replacement-dot-git-directory",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        destination = self.root / "replacement-dot-git-export"

        mission.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="export-replacement-dot-git-directory",
        )

        self.assertEqual(
            (
                destination
                / "repository"
                / ".git"
                / "retained-user-data.txt"
            ).read_text(encoding="utf-8"),
            "a replacement directory is retained data\n",
        )

    def test_preservation_blocked_discard_uses_exact_path_when_git_identity_is_broken(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        git_pointer = completed.worktree_path / ".git"
        git_pointer.write_text("gitdir: /missing/retained-worktree-admin\n", encoding="utf-8")
        with self.assertRaisesRegex(LaunchBlockedError, "Worktree Identity"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-broken-identity-discard",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)

        discarded = mission.discard_retained_worktree(
            completed.session_id,
            expected_revision=blocked.revision,
            correlation_id="discard-broken-git-identity",
            confirmation=completed.session_id,
            reason="Mission Commander explicitly discards the fingerprinted path.",
        )

        self.assertEqual(discarded.retirement["phase"], "retired")
        self.assertFalse(completed.worktree_path.exists())
        self.assertTrue((self.target_repo / "tracked.txt").is_file())
        self.assertFalse(mission._git_worktree_registration_present(completed))

    def test_direct_discard_rejects_dot_git_replacement_after_pointer_claim(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        git_pointer = completed.worktree_path / ".git"
        git_pointer.write_text(
            "gitdir: /missing/retained-worktree-admin\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(LaunchBlockedError, "Worktree Identity"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-dot-git-replacement",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        with patch.object(
            mission,
            "_discard_unpreserved_retained_worktree",
            side_effect=KeyboardInterrupt("interrupt after pointer claim"),
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "pointer claim"):
                mission.discard_retained_worktree(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="discard-before-dot-git-replacement",
                    confirmation=completed.session_id,
                    reason="Bind the exact Git administration pointer before discard.",
                )

        git_pointer.unlink()
        git_pointer.mkdir()
        replacement = git_pointer / "foreign-replacement.txt"
        replacement.write_text("must survive\n", encoding="utf-8")
        recovered = self.load_mission()
        with self.assertRaisesRegex(AlbertError, "content changed"):
            recovered.discard_retained_worktree(
                completed.session_id,
                expected_revision=blocked.revision,
                correlation_id="discard-before-dot-git-replacement",
                confirmation=completed.session_id,
                reason="Bind the exact Git administration pointer before discard.",
            )

        self.assertEqual(replacement.read_text(encoding="utf-8"), "must survive\n")

    def test_preservation_blocked_discard_recovers_partial_directory_deletion(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        extra = completed.worktree_path / "partial-delete.txt"
        extra.write_text("delete only after exact discard\n", encoding="utf-8")
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-partial-direct-discard",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        quiesced = self.load_mission()
        remove_tree = RetirementSnapshotStore.remove_retained_worktree

        def interrupt_after_partial_delete(
            path: Path,
            _manifest: dict[str, object],
            **_identity: int,
        ) -> None:
            (path / "partial-delete.txt").unlink()
            raise OSError("simulated partial direct discard")

        with patch.object(
            RetirementSnapshotStore,
            "remove_retained_worktree",
            side_effect=interrupt_after_partial_delete,
        ):
            with self.assertRaisesRegex(OSError, "partial direct discard"):
                quiesced.discard_retained_worktree(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="partial-direct-discard",
                    confirmation=completed.session_id,
                    reason="Exercise restart after partial exact-tree deletion.",
                )

        effect_path = quiesced._retirement_removal_effect_path(completed.session_id)
        self.assertFalse(completed.worktree_path.exists())
        self.assertTrue(effect_path.is_dir())
        self.assertFalse((effect_path / "partial-delete.txt").exists())

        late_entry = effect_path / "late-replacement.txt"
        late_entry.write_text("not covered by the discard authority\n", encoding="utf-8")
        with self.assertRaisesRegex(
            (AlbertError, RetirementSnapshotError),
            "content changed after authorization",
        ):
            self.load_mission().discard_retained_worktree(
                completed.session_id,
                expected_revision=blocked.revision,
                correlation_id="partial-direct-discard",
                confirmation=completed.session_id,
                reason="Exercise restart after partial exact-tree deletion.",
            )
        self.assertEqual(
            late_entry.read_text(encoding="utf-8"),
            "not covered by the discard authority\n",
        )
        late_entry.unlink()

        with patch.object(
            RetirementSnapshotStore,
            "remove_retained_worktree",
            wraps=remove_tree,
        ):
            recovered = self.load_mission().discard_retained_worktree(
                completed.session_id,
                expected_revision=blocked.revision,
                correlation_id="partial-direct-discard",
                confirmation=completed.session_id,
                reason="Exercise restart after partial exact-tree deletion.",
            )
        self.assertEqual(recovered.retirement["phase"], "retired")
        self.assertFalse(effect_path.exists())

    def test_preservation_blocked_discard_recovers_partial_forced_git_deletion(
        self,
    ) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        extra = completed.worktree_path / "partial-git-delete.txt"
        extra.write_text("delete only after exact Git discard\n", encoding="utf-8")
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-partial-git-discard",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        quiesced = self.load_mission()
        remove_tree = RetirementSnapshotStore.remove_retained_worktree

        def interrupt_forced_git_remove(
            path: Path,
            _manifest: dict[str, object],
            **_identity: int,
        ) -> None:
            (path / "partial-git-delete.txt").unlink()
            raise OSError("simulated partial forced Git discard")

        with patch.object(
            RetirementSnapshotStore,
            "remove_retained_worktree",
            side_effect=interrupt_forced_git_remove,
        ):
            with self.assertRaisesRegex(OSError, "partial forced Git discard"):
                quiesced.discard_retained_worktree(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="partial-forced-git-discard",
                    confirmation=completed.session_id,
                    reason="Exercise restart after partial exact Git deletion.",
                )

        effect_path = quiesced._retirement_removal_effect_path(completed.session_id)
        self.assertFalse(completed.worktree_path.exists())
        self.assertTrue(effect_path.is_dir())
        self.assertTrue(quiesced._git_worktree_registration_present_at(effect_path))

        with patch.object(
            RetirementSnapshotStore,
            "remove_retained_worktree",
            wraps=remove_tree,
        ):
            recovered = self.load_mission().discard_retained_worktree(
                completed.session_id,
                expected_revision=blocked.revision,
                correlation_id="partial-forced-git-discard",
                confirmation=completed.session_id,
                reason="Exercise restart after partial exact Git deletion.",
            )
        self.assertEqual(recovered.retirement["phase"], "retired")
        self.assertFalse(effect_path.exists())
        self.assertFalse(
            quiesced._git_worktree_registration_present_at(effect_path)
        )

    def test_direct_discard_preserves_content_added_after_subset_validation(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-late-direct-discard-write",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        quiesced = self.load_mission()
        prepare = RetirementSnapshotStore.prepare_retained_worktree_removal
        effect_path = quiesced._retirement_removal_effect_path(completed.session_id)
        foreign = effect_path / "foreign-after-validation.txt"

        def inject_after_permission_preparation(
            path: Path,
            manifest: dict[str, object],
        ) -> None:
            prepare(path, manifest)
            foreign.write_text("must survive exact discard\n", encoding="utf-8")

        with patch.object(
            RetirementSnapshotStore,
            "prepare_retained_worktree_removal",
            side_effect=inject_after_permission_preparation,
        ):
            with self.assertRaisesRegex(
                RetirementSnapshotError,
                "unauthorized content",
            ):
                quiesced.discard_retained_worktree(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="discard-with-late-foreign-write",
                    confirmation=completed.session_id,
                    reason="Never delete bytes added after exact subset validation.",
                )

        self.assertEqual(
            foreign.read_text(encoding="utf-8"),
            "must survive exact discard\n",
        )

    def test_direct_discard_preserves_file_replaced_after_entry_validation(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        claimed = completed.worktree_path / "claimed-before-discard.txt"
        claimed.write_text("claimed retained bytes\n", encoding="utf-8")
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-entry-swap-discard",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        quiesced = self.load_mission()
        effect_path = quiesced._retirement_removal_effect_path(completed.session_id)
        effect_claimed = effect_path / claimed.name
        parked_claimed = self.root / "validated-discard-file-parked"
        replacement_text = "replacement bytes must survive discard\n"
        record_path = RetirementSnapshotStore._retained_regular_file_record
        record_at = RetirementSnapshotStore._retained_regular_file_record_at
        prepare = RetirementSnapshotStore.prepare_retained_worktree_removal
        swapped = False
        removal_started = False

        def prepare_then_arm(
            path: Path,
            manifest: dict[str, object],
        ) -> None:
            nonlocal removal_started
            prepare(path, manifest)
            removal_started = True

        def swap_after_record() -> None:
            nonlocal swapped
            if swapped:
                return
            swapped = True
            effect_claimed.replace(parked_claimed)
            effect_claimed.write_text(replacement_text, encoding="utf-8")

        def record_then_swap(path: Path) -> dict[str, object]:
            record = record_path(path)
            if removal_started and Path(path) == effect_claimed:
                swap_after_record()
            return record

        def record_at_then_swap(
            parent_fd: int,
            name: str,
            initial: os.stat_result,
        ) -> dict[str, object]:
            record = record_at(parent_fd, name, initial)
            if removal_started and name == claimed.name and effect_claimed.exists():
                swap_after_record()
            return record

        with patch.object(
            RetirementSnapshotStore,
            "prepare_retained_worktree_removal",
            side_effect=prepare_then_arm,
        ), patch.object(
            RetirementSnapshotStore,
            "_retained_regular_file_record",
            side_effect=record_then_swap,
        ), patch.object(
            RetirementSnapshotStore,
            "_retained_regular_file_record_at",
            side_effect=record_at_then_swap,
        ):
            with self.assertRaisesRegex(
                RetirementSnapshotError,
                "changed|unauthorized",
            ):
                quiesced.discard_retained_worktree(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="discard-with-validated-entry-swap",
                    confirmation=completed.session_id,
                    reason="Never delete a replacement installed after entry proof.",
                )

        self.assertTrue(swapped)
        self.assertEqual(
            effect_claimed.read_text(encoding="utf-8"),
            replacement_text,
        )
        self.assertEqual(
            parked_claimed.read_text(encoding="utf-8"),
            "claimed retained bytes\n",
        )

    def test_retained_worktree_export_rejects_destination_below_its_source(self) -> None:
        blocked_mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(blocked_mission)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            blocked_mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-nested-export",
            )
        blocked = blocked_mission._refresh_persisted_session(completed.session_id)
        destination = completed.worktree_path / "nested-export"

        with self.assertRaisesRegex(AlbertError, "outside the Retained Worktree"):
            self.load_mission().export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id="reject-nested-retained-export",
            )

        self.assertFalse(destination.exists())
        persisted = self.load_mission().sessions[completed.session_id]
        self.assertEqual(persisted.retirement.get("export_intent") or {}, {})

    def test_preservation_entry_limit_still_allows_export_and_discard_recovery(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        recovery_entries = completed.worktree_path / "many-recovery-entries"
        recovery_entries.mkdir()
        for index in range(10_001):
            (recovery_entries / f"entry-{index:05d}").touch()

        with self.assertRaisesRegex(AlbertError, "10000-file preservation limit"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-over-entry-limit",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(blocked.retirement["phase"], "preservation-blocked")

        destination = self.root / "many-entry-recovery-export"
        receipt = mission.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="export-over-preservation-entry-limit",
        )
        self.assertEqual(
            receipt["destination"], str(destination.resolve() / "repository")
        )
        self.assertEqual(
            len(tuple((destination / "repository" / "many-recovery-entries").iterdir())),
            10_001,
        )

        after_export = mission._refresh_persisted_session(completed.session_id)
        discarded = mission.discard_retained_worktree(
            completed.session_id,
            expected_revision=after_export.revision,
            correlation_id="discard-over-preservation-entry-limit",
            confirmation=completed.session_id,
            reason="The large retained tree was exported and explicitly discarded.",
        )
        self.assertEqual(discarded.retirement["phase"], "retired")
        self.assertFalse(completed.worktree_path.exists())

    def test_preservation_blocked_discard_removes_read_only_directory(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        read_only = completed.worktree_path / "read-only"
        read_only.mkdir()
        (read_only / "retained.txt").write_text(
            "authorized read-only retained data\n",
            encoding="utf-8",
        )
        read_only.chmod(0o555)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-read-only-direct-discard",
            )
        blocked = mission._refresh_persisted_session(completed.session_id)
        effect_path = mission._retirement_removal_effect_path(completed.session_id)
        remove_tree = RetirementSnapshotStore.remove_retained_worktree

        try:
            with patch.object(
                RetirementSnapshotStore,
                "remove_retained_worktree",
                side_effect=OSError("interrupt after permission normalization"),
            ):
                with self.assertRaisesRegex(
                    OSError,
                    "permission normalization",
                ):
                    self.load_mission().discard_retained_worktree(
                        completed.session_id,
                        expected_revision=blocked.revision,
                        correlation_id="discard-read-only-retained-tree",
                        confirmation=completed.session_id,
                        reason="Delete the exact read-only tree after explicit authorization.",
                    )
            prepared = effect_path / "read-only"
            self.assertEqual(
                prepared.stat(follow_symlinks=False).st_mode & 0o777,
                0o755,
            )
            with patch.object(
                RetirementSnapshotStore,
                "remove_retained_worktree",
                wraps=remove_tree,
            ):
                discarded = self.load_mission().discard_retained_worktree(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="discard-read-only-retained-tree",
                    confirmation=completed.session_id,
                    reason="Delete the exact read-only tree after explicit authorization.",
                )
        finally:
            for root in (completed.worktree_path, effect_path):
                nested = root / "read-only"
                if nested.exists():
                    nested.chmod(0o700)
                if root.exists():
                    root.chmod(0o700)

        self.assertEqual(discarded.retirement["phase"], "retired")
        self.assertFalse(completed.worktree_path.exists())
        self.assertFalse(effect_path.exists())

    def test_concurrent_headless_admission_serializes_capacity_reservation(
        self,
    ) -> None:
        mission_one = self.load_mission(
            snapshot_storage_budget_bytes=32 * 1024 * 1024,
        )
        mission_two = self.load_mission(
            snapshot_storage_budget_bytes=32 * 1024 * 1024,
        )
        first_admission = threading.Event()
        release_first = threading.Event()
        original_admit = AlbertMission._admit_retirement_unit
        first_identity = id(mission_one)

        def paused_admit(candidate: AlbertMission) -> None:
            original_admit(candidate)
            if id(candidate) == first_identity:
                first_admission.set()
                self.assertTrue(release_first.wait(timeout=5))

        def queue_without_runner(
            candidate: AlbertMission,
            session_id: str,
            *,
            expected_revision: int | None = None,
        ):
            del expected_revision
            return candidate.sessions[session_id]

        with (
            patch.object(
                AlbertMission,
                "_admit_retirement_unit",
                autospec=True,
                side_effect=paused_admit,
            ),
            patch.object(
                AlbertMission,
                "run_session",
                autospec=True,
                side_effect=queue_without_runner,
            ),
            ThreadPoolExecutor(max_workers=2) as executor,
        ):
            first = executor.submit(
                mission_one.launch_headless_work,
                work_kind="run",
                agent_id="fake-local",
                prompt="first bounded headless unit",
            )
            self.assertTrue(first_admission.wait(timeout=5))
            second = executor.submit(
                mission_two.launch_headless_work,
                work_kind="run",
                agent_id="fake-local",
                prompt="second bounded headless unit",
            )
            time.sleep(0.1)
            self.assertFalse(second.done())
            release_first.set()
            first_session = first.result(timeout=5)
            with self.assertRaisesRegex(LaunchBlockedError, "Storage Budget is exhausted"):
                second.result(timeout=5)

        persisted = self.load_mission(
            snapshot_storage_budget_bytes=32 * 1024 * 1024,
        )
        self.assertIn(first_session.session_id, persisted.sessions)
        self.assertEqual(
            sum(
                session.preservation_budget["reserved_bytes"]
                for session in persisted.sessions.values()
                if session.preservation_budget["bound"]
            ),
            32 * 1024 * 1024,
        )

    def test_retirement_export_replays_after_materialization_before_receipt(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create one blocked export crash cut.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        destination = self.root / "crash-export"
        write_payload = mission._write_runtime_payload

        def interrupt_before_export_receipt(data: dict[str, object]) -> None:
            raw_session = data.get("sessions", {}).get(completed.session_id, {})
            receipt = raw_session.get("retirement", {}).get(
                "action_receipts", {}
            ).get("export-crash-cut")
            if receipt:
                raise KeyboardInterrupt("crash before export receipt")
            write_payload(data)

        with patch.object(
            mission,
            "_write_runtime_payload",
            side_effect=interrupt_before_export_receipt,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "export receipt"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="export-crash-cut",
                )

        self.assertEqual(
            (destination / "repository" / "tracked.txt").read_text(encoding="utf-8"),
            "baseline\n",
        )
        (destination / "repository" / "tracked.txt").write_text(
            "tampered after marker\n",
            encoding="utf-8",
        )
        recovered = self.load_mission()
        with self.assertRaisesRegex(AlbertError, "exported repository content"):
            recovered.export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id="export-crash-cut",
            )
        (destination / "repository" / "tracked.txt").write_text(
            "baseline\n",
            encoding="utf-8",
        )
        receipt = recovered.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="export-crash-cut",
        )
        self.assertEqual(
            receipt["destination"], str(destination.resolve() / "repository")
        )

    def test_retirement_export_recovers_private_staging_before_marker(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create one blocked pre-marker export crash cut.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        destination = self.root / "pre-marker-crash-export"
        staging_roots: list[Path] = []

        def interrupt_before_export_marker(
            stage_fd: int,
            _path: Path,
            _content: str,
        ) -> None:
            staging_roots.append(
                mission._bound_retirement_export_directory_path(stage_fd)
            )
            raise KeyboardInterrupt("crash before export marker")

        with patch.object(
            mission,
            "_write_retirement_export_marker_exclusive",
            side_effect=interrupt_before_export_marker,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "export marker"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="export-pre-marker-crash-cut",
                )

        self.assertEqual(len(staging_roots), 1)
        staging_root = staging_roots[0]
        self.assertTrue((staging_root / "repository").is_dir())
        self.assertFalse(destination.exists())
        staged_tracked = staging_root / "repository" / "tracked.txt"
        staged_tracked.write_text(
            "changed after crash\n",
            encoding="utf-8",
        )
        recovered = self.load_mission()
        receipt = recovered.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="export-pre-marker-crash-cut",
        )
        self.assertEqual(
            receipt["destination"], str(destination.resolve() / "repository")
        )
        self.assertEqual(
            staged_tracked.read_text(encoding="utf-8"),
            "changed after crash\n",
        )
        self.assertEqual(
            (destination / "repository" / "tracked.txt").read_text(
                encoding="utf-8"
            ),
            "baseline\n",
        )

    def test_retirement_retry_replays_after_effect_before_receipt(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create one blocked retry crash cut.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        write_payload = mission._write_runtime_payload

        def interrupt_before_retry_receipt(data: dict[str, object]) -> None:
            raw_session = data.get("sessions", {}).get(completed.session_id, {})
            receipt = raw_session.get("retirement", {}).get(
                "action_receipts", {}
            ).get("retry-crash-cut")
            if receipt:
                raise KeyboardInterrupt("crash before retry receipt")
            write_payload(data)

        with patch.object(
            mission,
            "_write_runtime_payload",
            side_effect=interrupt_before_retry_receipt,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "retry receipt"):
                mission.retry_retirement_unit(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="retry-crash-cut",
                )

        recovered = self.load_mission()
        retried = recovered.retry_retirement_unit(
            completed.session_id,
            expected_revision=blocked.revision,
            correlation_id="retry-crash-cut",
        )
        self.assertEqual(retried.retirement["phase"], "retired")
        replayed = recovered.retry_retirement_unit(
            completed.session_id,
            expected_revision=blocked.revision,
            correlation_id="retry-crash-cut",
        )
        self.assertEqual(replayed.revision, retried.revision)

    def test_retained_worktree_discard_requires_quiescence_and_exact_containment(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create a blocked unit for explicit discard.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)

        blocked = mission._refresh_persisted_session(completed.session_id)
        live = self.load_mission(quiescence=("live-exact", "absent"))
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            live.discard_retained_worktree(
                completed.session_id,
                expected_revision=blocked.revision,
                correlation_id="discard-live-worktree",
                confirmation=completed.session_id,
                reason="Mission Commander explicitly accepts irreversible deletion.",
            )
        self.assertTrue(completed.worktree_path.is_dir())

        safe = self.load_mission()
        discarded = safe.discard_retained_worktree(
            completed.session_id,
            expected_revision=blocked.revision,
            correlation_id="discard-quiesced-worktree",
            confirmation=completed.session_id,
            reason="Mission Commander explicitly accepts irreversible deletion.",
        )
        self.assertEqual(discarded.retirement["phase"], "retired")
        self.assertTrue(discarded.retirement["discarded_at"])
        self.assertEqual(
            discarded.retirement["removal_kind"], "retained-worktree-discard"
        )
        self.assertFalse(completed.worktree_path.exists())
        self.assertTrue(self.target_repo.is_dir())
        self.assertTrue((self.target_repo / "tracked.txt").is_file())
        reloaded = self.load_mission().sessions[completed.session_id]
        self.assertEqual(reloaded.retirement["phase"], "retired")
        self.assertEqual(
            reloaded.retirement["removal_kind"], "retained-worktree-discard"
        )

    def test_retained_worktree_discard_rejects_replacement_directory_data(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create a blocked unit before path substitution.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        shutil.rmtree(completed.worktree_path)
        completed.worktree_path.mkdir()
        sentinel = completed.worktree_path / "replacement-data.txt"
        sentinel.write_text("unrelated replacement bytes\n", encoding="utf-8")

        with self.assertRaisesRegex(
            (AlbertError, RetirementSnapshotError),
            "changed|invalid|snapshot|Snapshot|preserved",
        ):
            mission.discard_retained_worktree(
                completed.session_id,
                expected_revision=blocked.revision,
                correlation_id="discard-replacement-directory",
                confirmation=completed.session_id,
                reason="This must not authorize replacement data deletion.",
            )

        self.assertEqual(
            sentinel.read_text(encoding="utf-8"),
            "unrelated replacement bytes\n",
        )

    def test_retained_worktree_discard_serializes_snapshot_pin_mutation(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create a blocked unit for action serialization.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        deletion_started = threading.Event()
        finish_deletion = threading.Event()
        remove_tree = shutil.rmtree

        def paused_remove(path: Path, *args, **kwargs) -> None:
            deletion_started.set()
            self.assertTrue(finish_deletion.wait(timeout=5))
            remove_tree(path, *args, **kwargs)

        with patch("albert_mvp.core.shutil.rmtree", side_effect=paused_remove):
            with ThreadPoolExecutor(max_workers=2) as executor:
                discard = executor.submit(
                    mission.discard_retained_worktree,
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="serialized-discard",
                    confirmation=completed.session_id,
                    reason="Delete only after serializing policy mutation.",
                )
                self.assertTrue(deletion_started.wait(timeout=5))
                pin = executor.submit(
                    mission.set_retirement_snapshot_pin,
                    completed.session_id,
                    pinned=True,
                    expected_revision=blocked.revision + 1,
                    correlation_id="concurrent-pin",
                )
                time.sleep(0.1)
                self.assertFalse(pin.done())
                finish_deletion.set()
                discarded = discard.result(timeout=5)
                with self.assertRaises(LaunchBlockedError):
                    pin.result(timeout=5)

        self.assertEqual(discarded.retirement["phase"], "retired")
        self.assertFalse(completed.worktree_path.exists())

    def test_retained_worktree_discard_serializes_retirement_retry(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create a blocked unit for retry serialization.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        deletion_started = threading.Event()
        finish_deletion = threading.Event()
        remove_tree = shutil.rmtree

        def paused_remove(path: Path, *args, **kwargs) -> None:
            deletion_started.set()
            self.assertTrue(finish_deletion.wait(timeout=5))
            remove_tree(path, *args, **kwargs)

        with patch("albert_mvp.core.shutil.rmtree", side_effect=paused_remove):
            with ThreadPoolExecutor(max_workers=2) as executor:
                discard = executor.submit(
                    mission.discard_retained_worktree,
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="serialized-discard-before-retry",
                    confirmation=completed.session_id,
                    reason="Delete only after serializing every blocked action.",
                )
                self.assertTrue(deletion_started.wait(timeout=5))
                retry = executor.submit(
                    mission.retry_retirement_unit,
                    completed.session_id,
                    expected_revision=blocked.revision + 1,
                    correlation_id="concurrent-retry",
                )
                time.sleep(0.1)
                self.assertFalse(retry.done())
                finish_deletion.set()
                discarded = discard.result(timeout=5)
                with self.assertRaises(LaunchBlockedError):
                    retry.result(timeout=5)

        self.assertEqual(discarded.retirement["phase"], "retired")
        self.assertFalse(completed.worktree_path.exists())
        self.assertEqual(discarded.retirement.get("discard_intent"), {})

    def test_git_retained_worktree_discard_blocks_a_process_cwd(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create a blocked Git unit for handle inspection.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        holder = subprocess.Popen(
            [
                sys.executable,
                "-c",
                "import os,sys,time; os.chdir(sys.argv[1]); "
                "print('ready', flush=True); time.sleep(30)",
                str(completed.worktree_path),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.addCleanup(self._stop_process, holder)
        self.assertEqual(holder.stdout.readline().strip(), "ready")

        with self.assertRaisesRegex(AlbertError, "process|handle"):
            mission.discard_retained_worktree(
                completed.session_id,
                expected_revision=blocked.revision,
                correlation_id="discard-git-live-cwd",
                confirmation=completed.session_id,
                reason="Open handles must still block explicit discard.",
            )

        self.assertTrue(
            completed.worktree_path.is_dir()
            or mission._retirement_removal_effect_path(completed.session_id).is_dir()
        )
        self.assertIsNone(holder.poll())

    def test_darwin_discard_inspects_open_handles_across_nested_mounts(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        blocked_mission = self.load_mission(
            quiescence=("absent", "live-exact")
        )
        completed = self.completed_session(blocked_mission)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            blocked_mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-before-darwin-mounted-handle",
            )
        blocked = blocked_mission._refresh_persisted_session(
            completed.session_id
        )
        quiesced = self.load_mission()
        observed_commands: list[list[str]] = []

        def inspect_like_darwin_lsof(
            command: list[str],
            **_kwargs,
        ) -> subprocess.CompletedProcess[str]:
            observed_commands.append(command)
            expected = [
                "/usr/sbin/lsof",
                "-Fn",
                "-xf",
                "+D",
                str(completed.worktree_path.resolve(strict=True)),
            ]
            if command == expected:
                return subprocess.CompletedProcess(
                    command,
                    0,
                    stdout=f"p123\nn{completed.worktree_path}/mounted/open.txt\n",
                    stderr="",
                )
            return subprocess.CompletedProcess(
                command,
                1,
                stdout="",
                stderr="",
            )

        with (
            patch.object(core_module.sys, "platform", "darwin"),
            patch.object(core_module.shutil, "which", return_value="/usr/sbin/lsof"),
            patch.object(
                core_module,
                "_run_bounded_process",
                side_effect=inspect_like_darwin_lsof,
            ),
        ):
            with self.assertRaisesRegex(AlbertError, "open process handle"):
                quiesced.discard_retained_worktree(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="discard-with-darwin-mounted-handle",
                    confirmation=completed.session_id,
                    reason="Nested mounted handles must remain a discard blocker.",
                )

        self.assertTrue(completed.worktree_path.is_dir())
        self.assertEqual(
            observed_commands,
            [[
                "/usr/sbin/lsof",
                "-Fn",
                "-xf",
                "+D",
                str(completed.worktree_path.resolve(strict=True)),
            ]],
        )

    def test_retained_worktree_discard_never_targets_the_coding_workspace(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create a blocked unit for containment testing.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        runtime = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        runtime["sessions"][completed.session_id]["worktree_path"] = str(
            self.target_repo
        )
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        unsafe = self.load_mission()
        poisoned = unsafe.sessions[completed.session_id]

        with self.assertRaisesRegex(LaunchBlockedError, "Coding Workspace"):
            unsafe.discard_retained_worktree(
                completed.session_id,
                expected_revision=poisoned.revision,
                correlation_id="discard-coding-workspace",
                confirmation=completed.session_id,
                reason="This must still fail closed.",
            )

        self.assertTrue(self.target_repo.is_dir())
        self.assertTrue((self.target_repo / "tracked.txt").is_file())

    def test_retained_worktree_discard_replays_after_effect_before_receipt(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create a blocked unit for discard replay.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        blocked = mission._refresh_persisted_session(completed.session_id)
        write_payload = mission._write_runtime_payload

        def interrupt_before_discard_receipt(data: dict[str, object]) -> None:
            raw_session = data.get("sessions", {}).get(completed.session_id, {})
            retirement = raw_session.get("retirement", {})
            if (
                retirement.get("phase") == "retired"
                and retirement.get("action_receipts", {}).get("discard-crash-cut")
            ):
                raise KeyboardInterrupt("crash before discard receipt")
            write_payload(data)

        with patch.object(
            mission,
            "_write_runtime_payload",
            side_effect=interrupt_before_discard_receipt,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "discard receipt"):
                mission.discard_retained_worktree(
                    completed.session_id,
                    expected_revision=blocked.revision,
                    correlation_id="discard-crash-cut",
                    confirmation=completed.session_id,
                    reason="Mission Commander explicitly accepts irreversible deletion.",
                )

        self.assertFalse(completed.worktree_path.exists())
        recovered = self.load_mission()
        discarded = recovered.discard_retained_worktree(
            completed.session_id,
            expected_revision=blocked.revision,
            correlation_id="discard-crash-cut",
            confirmation=completed.session_id,
            reason="Mission Commander explicitly accepts irreversible deletion.",
        )
        self.assertEqual(discarded.retirement["phase"], "retired")
        self.assertEqual(
            discarded.retirement["action_receipts"]["discard-crash-cut"][
                "expected_revision"
            ],
            blocked.revision,
        )
        replayed = recovered.discard_retained_worktree(
            completed.session_id,
            expected_revision=blocked.revision,
            correlation_id="discard-crash-cut",
            confirmation=completed.session_id,
            reason="Mission Commander explicitly accepts irreversible deletion.",
        )
        self.assertEqual(replayed.revision, discarded.revision)
        self.assertTrue(self.target_repo.is_dir())

    def test_non_git_retirement_preserves_a_write_at_the_removal_boundary(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        prepare = RetirementSnapshotStore.prepare_managed_directory_removal

        def mutate_after_prepare(
            store: RetirementSnapshotStore,
            record: dict[str, object],
            *,
            worktree_path: Path | None = None,
        ) -> None:
            prepare(store, record, worktree_path=worktree_path)
            completed.worktree_path.mkdir(parents=True, exist_ok=True)
            (completed.worktree_path / "late-boundary.txt").write_text(
                "late unpreserved bytes\n",
                encoding="utf-8",
            )

        with patch.object(
            RetirementSnapshotStore,
            "prepare_managed_directory_removal",
            autospec=True,
            side_effect=mutate_after_prepare,
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Exercise a managed-directory removal-boundary write.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )

        reviewed = self.load_mission().sessions[completed.session_id]
        late = completed.worktree_path / "late-boundary.txt"
        self.assertNotEqual(reviewed.retirement["phase"], "retired")
        self.assertEqual(late.read_text(encoding="utf-8"), "late unpreserved bytes\n")

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux procfs contract")
    def test_linux_handle_scan_fails_closed_for_inaccessible_live_process(self) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        original_readlink = os.readlink
        inaccessible_boundary = Path(f"/proc/{os.getpid()}/cwd")

        def readlink_with_isolated_process(path):
            if Path(path) == inaccessible_boundary:
                raise PermissionError(errno.EACCES, "access isolated", str(path))
            return original_readlink(path)

        with (
            patch.object(os, "readlink", side_effect=readlink_with_isolated_process),
            self.assertRaisesRegex(
                AlbertError,
                "Open-handle inspection was unavailable before retirement",
            ),
        ):
            mission._assert_no_open_retirement_handles(completed.worktree_path)

    def test_non_git_retirement_blocks_an_open_handle_write_after_validation(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        prepare = RetirementSnapshotStore.prepare_managed_directory_removal
        tracked = completed.worktree_path / "tracked.txt"

        with tracked.open("r+b") as handle:

            def mutate_through_open_handle(
                store: RetirementSnapshotStore,
                record: dict[str, object],
                *,
                worktree_path: Path | None = None,
            ) -> None:
                prepare(store, record, worktree_path=worktree_path)
                handle.seek(0)
                handle.write(b"late open-handle bytes\n")
                handle.truncate()
                handle.flush()
                os.fsync(handle.fileno())
                self.assertIsNotNone(worktree_path)

            with patch.object(
                RetirementSnapshotStore,
                "prepare_managed_directory_removal",
                autospec=True,
                side_effect=mutate_through_open_handle,
            ):
                mission.record_frontier_review(
                    completed.session_id,
                    "Approved",
                    reason="Exercise an open-handle removal-boundary write.",
                    allowed_session_statuses={"evidence-ready"},
                    expected_revision=completed.revision,
                )

            retained = mission._refresh_persisted_session(completed.session_id)
            effect_path = mission._retirement_removal_effect_path(completed.session_id)
            self.assertNotEqual(retained.retirement["phase"], "retired")
            self.assertEqual(
                (effect_path / "tracked.txt").read_bytes(),
                b"late open-handle bytes\n",
            )

    def test_non_git_retirement_blocks_a_process_cwd_inside_the_effect(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        holder = subprocess.Popen(
            [
                sys.executable,
                "-c",
                "import os,sys,time; os.chdir(sys.argv[1]); "
                "print('ready', flush=True); time.sleep(30)",
                str(completed.worktree_path),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.addCleanup(self._stop_process, holder)
        self.assertEqual(holder.stdout.readline().strip(), "ready")

        mission.record_frontier_review(
            completed.session_id,
            "Approved",
            reason="Exercise a process cwd retirement boundary.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=completed.revision,
        )

        retained = mission._refresh_persisted_session(completed.session_id)
        self.assertNotEqual(retained.retirement["phase"], "retired")
        self.assertIn("process", retained.retirement["blocked_reason"])

    def test_non_git_retirement_blocks_a_writable_shared_mapping(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        holder = subprocess.Popen(
            [
                sys.executable,
                "-c",
                "import mmap,sys,time; f=open(sys.argv[1], 'r+b'); "
                "m=mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_WRITE); f.close(); "
                "print('ready', flush=True); time.sleep(30)",
                str(completed.worktree_path / "tracked.txt"),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.addCleanup(self._stop_process, holder)
        self.assertEqual(holder.stdout.readline().strip(), "ready")

        mission.record_frontier_review(
            completed.session_id,
            "Approved",
            reason="Exercise a writable mapping retirement boundary.",
            allowed_session_statuses={"evidence-ready"},
            expected_revision=completed.revision,
        )

        retained = mission._refresh_persisted_session(completed.session_id)
        self.assertNotEqual(retained.retirement["phase"], "retired")
        self.assertIn("writable shared mapping", retained.retirement["blocked_reason"])

    def test_non_git_cleanup_resumes_after_partial_directory_removal(self) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        mission = self.load_mission()
        completed = self.completed_session(mission)
        preserved = mission.preserve_retirement_unit(
            completed.session_id,
            expected_revision=completed.revision,
            correlation_id="preserve-before-directory-cleanup-crash",
        )
        snapshot_revision = preserved.retirement["snapshot"]["session_revision"]
        store = mission._retirement_snapshot_store(preserved, snapshot_revision)
        (preserved.worktree_path / "tracked.txt").unlink()

        store.prepare_managed_directory_removal(preserved.retirement["snapshot"])

        self.assertTrue(preserved.worktree_path.is_dir())

    def test_cli_nondefault_retention_grace_governs_startup_reconciliation(self) -> None:
        second_issue = self.add_issue(2)
        mission = self.load_mission()
        completed = self.completed_session(mission)
        completed.status = "failed"
        mission._persist_session_update(
            completed,
            expected_statuses={"evidence-ready"},
        )
        output = io.StringIO()
        with (
            patch.object(
                AlbertMission,
                "_probe_retirement_quiescence",
                return_value=("absent", "absent"),
            ),
            redirect_stdout(output),
        ):
            exit_code = main(
                [
                    "assign",
                    second_issue,
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    mission.mission_id,
                    "--agent-config",
                    str(self.agent_config),
                    "--retention-grace-seconds",
                    "17",
                    "--agent",
                    "fake-local",
                    "--notes",
                    "Mutating command must own startup reconciliation.",
                ]
            )

        self.assertEqual(exit_code, 0)
        grace = self.load_mission().sessions[completed.session_id].retirement
        started = datetime.fromisoformat(grace["grace_started_at"])
        expires = datetime.fromisoformat(grace["grace_expires_at"])
        self.assertEqual((expires - started).total_seconds(), 17)

    def test_cli_and_persistent_transport_share_retirement_snapshot_semantics(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            mission.mission_id,
            "--agent-config",
            str(self.agent_config),
            "--session-id",
            completed.session_id,
            "--session-mission-id",
            mission.mission_id,
        ]
        output = io.StringIO()
        with (
            patch.object(
                AlbertMission,
                "_probe_retirement_quiescence",
                return_value=("absent", "absent"),
            ),
            redirect_stdout(output),
        ):
            exit_code = main(
                [
                    "retirement-preserve",
                    *common,
                    "--expected-revision",
                    str(completed.revision),
                    "--correlation-id",
                    "cli-preserve-1",
                ]
            )
        self.assertEqual(exit_code, 0)
        preserved = json.loads(output.getvalue())
        self.assertTrue(preserved["verified"])
        self.assertEqual(preserved["retirement"]["phase"], "preserved")

        replay_output = io.StringIO()
        with redirect_stdout(replay_output):
            replay_exit = main(
                [
                    "retirement-preserve",
                    *common,
                    "--expected-revision",
                    str(completed.revision),
                    "--correlation-id",
                    "cli-preserve-1",
                ]
            )
        self.assertEqual(replay_exit, 0)
        self.assertEqual(
            json.loads(replay_output.getvalue())["retirement"]["snapshot"],
            preserved["retirement"]["snapshot"],
        )

        transport_output = io.StringIO()
        serve(
            io.StringIO(
                json.dumps(
                    {
                        "id": "retirement-verify-1",
                        "argv": ["retirement-verify", *common],
                    }
                )
                + "\n"
            ),
            transport_output,
        )
        envelope = json.loads(transport_output.getvalue())
        self.assertTrue(envelope["success"])
        verified = json.loads(envelope["stdout"])
        self.assertTrue(verified["verified"])
        self.assertEqual(
            verified["retirement"]["snapshot"]["manifest_sha256"],
            preserved["retirement"]["snapshot"]["manifest_sha256"],
        )

    def test_cli_and_persistent_transport_share_storage_inspection_configuration(
        self,
    ) -> None:
        mission = self.load_mission()
        common = [
            "retirement-storage",
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            mission.mission_id,
            "--agent-config",
            str(self.agent_config),
            "--snapshot-storage-retention-seconds",
            "123",
            "--snapshot-storage-budget-bytes",
            str(64 * 1024 * 1024),
        ]
        output = io.StringIO()
        with redirect_stdout(output):
            exit_code = main(common)
        self.assertEqual(exit_code, 0)
        inspection = json.loads(output.getvalue())
        self.assertEqual(inspection["policy"]["retention_seconds"], 123)
        self.assertEqual(inspection["policy"]["budget_bytes"], 64 * 1024 * 1024)

        transport_output = io.StringIO()
        serve(
            io.StringIO(
                json.dumps({"id": "storage-inspection", "argv": common}) + "\n"
            ),
            transport_output,
        )
        envelope = json.loads(transport_output.getvalue())
        self.assertTrue(envelope["success"])
        self.assertEqual(json.loads(envelope["stdout"]), inspection)

    def test_cli_and_persistent_transport_expose_blocked_retirement_inspection(
        self,
    ) -> None:
        mission = self.load_mission()
        completed = self.completed_session(mission)
        with patch.object(
            mission,
            "_remove_retirement_worktree",
            side_effect=AlbertError("simulated exact removal failure"),
        ):
            mission.record_frontier_review(
                completed.session_id,
                "Approved",
                reason="Create a blocked unit for transport inspection.",
                allowed_session_statuses={"evidence-ready"},
                expected_revision=completed.revision,
            )
            mission.reconcile_retirement_unit(completed.session_id)
        common = [
            "retirement-inspect",
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            mission.mission_id,
            "--agent-config",
            str(self.agent_config),
            "--session-id",
            completed.session_id,
            "--session-mission-id",
            mission.mission_id,
        ]
        output = io.StringIO()
        with redirect_stdout(output):
            exit_code = main(common)
        self.assertEqual(exit_code, 0)
        inspection = json.loads(output.getvalue())
        self.assertEqual(inspection["phase"], "retirement-blocked")
        self.assertTrue(inspection["actions"]["discard"])

        transport_output = io.StringIO()
        serve(
            io.StringIO(
                json.dumps({"id": "retirement-inspect", "argv": common}) + "\n"
            ),
            transport_output,
        )
        envelope = json.loads(transport_output.getvalue())
        self.assertTrue(envelope["success"])
        self.assertEqual(json.loads(envelope["stdout"]), inspection)

    def test_cli_and_persistent_transport_share_blocked_retirement_actions(
        self,
    ) -> None:
        shutil.move(self.target_repo / ".git", self.root / "detached-target-git")
        for sequence in (2, 3, 4):
            self.add_issue(sequence)
        mission = self.load_mission()

        def blocked_session(issue_id: str):
            completed = self.completed_issue(mission, issue_id)
            with patch.object(
                mission,
                "_remove_retirement_worktree",
                side_effect=AlbertError("simulated exact removal failure"),
            ):
                mission.record_frontier_review(
                    completed.session_id,
                    "Approved",
                    reason="Create a blocked unit for CLI action coverage.",
                    allowed_session_statuses={"evidence-ready"},
                    expected_revision=completed.revision,
                )
                mission.reconcile_retirement_unit(completed.session_id)
            return mission._refresh_persisted_session(completed.session_id)

        pin_unit = blocked_session("ISS-01")
        export_unit = blocked_session("ISS-02")
        retry_unit = blocked_session("ISS-03")
        discard_unit = blocked_session("ISS-04")
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            mission.mission_id,
            "--agent-config",
            str(self.agent_config),
        ]
        commands = [
            [
                "retirement-pin",
                *common,
                "--session-id",
                pin_unit.session_id,
                "--expected-revision",
                str(pin_unit.revision),
                "--correlation-id",
                "cli-pin-action",
                "--pin-state",
                "pinned",
            ],
            [
                "retirement-export",
                *common,
                "--session-id",
                export_unit.session_id,
                "--expected-revision",
                str(export_unit.revision),
                "--correlation-id",
                "cli-export-action",
                "--destination",
                str(self.root / "cli-export"),
            ],
            [
                "retirement-retry",
                *common,
                "--session-id",
                retry_unit.session_id,
                "--expected-revision",
                str(retry_unit.revision),
                "--correlation-id",
                "cli-retry-action",
            ],
            [
                "retirement-discard",
                *common,
                "--session-id",
                discard_unit.session_id,
                "--expected-revision",
                str(discard_unit.revision),
                "--correlation-id",
                "cli-discard-action",
                "--confirmation",
                discard_unit.session_id,
                "--reason",
                "Mission Commander accepts exact irreversible deletion.",
            ],
        ]
        for index, argv in enumerate(commands, start=1):
            with self.subTest(command=argv[0]):
                output = io.StringIO()
                with redirect_stdout(output):
                    self.assertEqual(main(argv), 0)
                one_process = json.loads(output.getvalue())
                transport_output = io.StringIO()
                serve(
                    io.StringIO(
                        json.dumps({"id": f"blocked-action-{index}", "argv": argv})
                        + "\n"
                    ),
                    transport_output,
                )
                envelope = json.loads(transport_output.getvalue())
                self.assertTrue(envelope["success"])
                self.assertEqual(json.loads(envelope["stdout"]), one_process)

        pinned_unit = self.load_mission().sessions[pin_unit.session_id]
        workstation_argv = [
            "workstation-action",
            *common,
            "--correlation-id",
            "cli-workstation-unpin-action",
            "--expected-revision",
            str(pinned_unit.revision),
            "--action-type",
            "retirement-pin",
            "--actor",
            "mission-commander",
            "--target-kind",
            "agent-session",
            "--target-id",
            pinned_unit.session_id,
            "--action-mission-id",
            mission.mission_id,
            "--issue-id",
            pinned_unit.issue_id,
            "--session-id",
            pinned_unit.session_id,
            "--pin-state",
            "unpinned",
        ]
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(main(workstation_argv), 0)
        acknowledgement = json.loads(output.getvalue())
        self.assertEqual(acknowledgement["action_type"], "retirement-pin")
        self.assertIn("unpinned", acknowledgement["effect_summary"])


if __name__ == "__main__":
    unittest.main()
