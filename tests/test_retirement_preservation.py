from __future__ import annotations

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
from contextlib import redirect_stdout
from datetime import datetime, timedelta
from pathlib import Path
from unittest.mock import patch

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
        ).load()

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
        mission.runtime_path.write_text(
            json.dumps(runtime, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        budget = 32 * 1024 * 1024 + max(
            first_record["snapshot_bytes"], second_record["snapshot_bytes"]
        )

        constrained = self.load_mission(
            snapshot_storage_budget_bytes=budget,
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
        self.assertEqual(resolved["counts"]["retained_payloads"], 0)

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
        remove_tree = shutil.rmtree
        interrupted = False

        def interrupt_after_payload_deletion(path: Path, *args, **kwargs) -> None:
            nonlocal interrupted
            remove_tree(path, *args, **kwargs)
            if Path(path) == Path(first_record["payload_path"]) and not interrupted:
                interrupted = True
                raise KeyboardInterrupt("crash after payload deletion")

        with patch(
            "albert_mvp.core.shutil.rmtree",
            side_effect=interrupt_after_payload_deletion,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "payload deletion"):
                constrained.launch_issue(third_issue)

        self.assertFalse(Path(first_record["payload_path"]).exists())
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

    def test_preservation_blocked_does_not_advertise_unavailable_export(self) -> None:
        mission = self.load_mission(quiescence=("absent", "live-exact"))
        completed = self.completed_session(mission)
        with self.assertRaisesRegex(LaunchBlockedError, "Runner Quiescence"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preservation-blocked-actions",
            )

        blocked = mission._refresh_persisted_session(completed.session_id)
        self.assertEqual(blocked.retirement["phase"], "preservation-blocked")
        inspection = mission.inspect_retirement_unit(completed.session_id)
        self.assertEqual(
            inspection["actions"],
            {"retry": True, "inspect": True, "export": False, "discard": True},
        )

    def test_concurrent_headless_admission_serializes_capacity_reservation(self) -> None:
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

    def test_retirement_export_recovers_materialization_before_marker(self) -> None:
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
        write_file = mission._write

        def interrupt_before_export_marker(path: Path, content: str) -> None:
            if path.resolve(strict=False) == (
                destination / "retirement-export.json"
            ).resolve(strict=False):
                raise KeyboardInterrupt("crash before export marker")
            write_file(path, content)

        with patch.object(
            mission,
            "_write",
            side_effect=interrupt_before_export_marker,
        ):
            with self.assertRaisesRegex(KeyboardInterrupt, "export marker"):
                mission.export_retirement_unit(
                    completed.session_id,
                    destination=destination,
                    expected_revision=blocked.revision,
                    correlation_id="export-pre-marker-crash-cut",
                )

        self.assertTrue((destination / "repository").is_dir())
        (destination / "repository" / "tracked.txt").write_text(
            "changed after crash\n",
            encoding="utf-8",
        )
        recovered = self.load_mission()
        with self.assertRaisesRegex(AlbertError, "exported repository content"):
            recovered.export_retirement_unit(
                completed.session_id,
                destination=destination,
                expected_revision=blocked.revision,
                correlation_id="export-pre-marker-crash-cut",
            )
        (destination / "repository" / "tracked.txt").write_text(
            "baseline\n",
            encoding="utf-8",
        )
        receipt = recovered.export_retirement_unit(
            completed.session_id,
            destination=destination,
            expected_revision=blocked.revision,
            correlation_id="export-pre-marker-crash-cut",
        )
        self.assertEqual(
            receipt["destination"], str(destination.resolve() / "repository")
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
        self.assertIn("process handle", retained.retirement["blocked_reason"])

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
                    "board",
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
