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
from datetime import datetime
from pathlib import Path
from unittest.mock import patch

from albert_mvp.cli import main
from albert_mvp.core import AlbertError, AlbertMission, LaunchBlockedError
from albert_mvp.retirement import RetirementSnapshotStore
from albert_mvp.server import serve

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

    def load_mission(
        self,
        *,
        quiescence: tuple[str, str] | None = ("absent", "absent"),
        retention_grace_seconds: int = 72 * 60 * 60,
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
            **options,
        ).load()

    def completed_session(self, mission: AlbertMission):
        mission.assign_issue("ISS-01", "fake-local")
        mission.approve_issue("ISS-01")
        launched = mission.launch_issue("ISS-01")
        return mission.run_session(launched.session_id)

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
        prepare = RetirementSnapshotStore.prepare_git_non_force_removal

        def mutate_after_preservation(
            store: RetirementSnapshotStore,
            record: dict[str, object],
        ) -> None:
            (store.request.worktree_path / "late-change.txt").write_text(
                "not present in the verified snapshot\n",
                encoding="utf-8",
            )
            prepare(store, record)

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
        self.assertEqual(retained.retirement["phase"], "retiring")
        self.assertEqual(retained.retirement["retirement_attempts"], 1)
        self.assertIn(
            "changed after verified preservation",
            retained.retirement["blocked_reason"],
        )
        self.assertTrue(retained.worktree_path.is_dir())
        self.assertTrue((retained.worktree_path / "late-change.txt").is_file())

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
        thread.join(timeout=10)

        self.assertFalse(thread.is_alive())
        self.assertEqual(outcomes, ["retired"])
        retired = self.load_mission().sessions[launched.session_id]
        self.assertEqual(retired.status, "cancelled")
        self.assertEqual(retired.retirement["phase"], "retired")
        self.assertFalse(retired.worktree_path.exists())

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
            self.assertEqual(first.retirement["retirement_attempts"], 1)

            second = mission.reconcile_retirement_unit(completed.session_id)
            self.assertEqual(second.retirement["phase"], "retiring")
            self.assertEqual(second.retirement["retirement_attempts"], 2)

            third = mission.reconcile_retirement_unit(completed.session_id)
            self.assertEqual(third.retirement["phase"], "retirement-blocked")
            self.assertEqual(third.retirement["retirement_attempts"], 3)
            self.assertEqual(removal.call_count, 3)

            replayed = mission.reconcile_retirement_unit(completed.session_id)
            self.assertEqual(replayed.retirement["phase"], "retirement-blocked")
            self.assertEqual(removal.call_count, 3)

        self.assertTrue(completed.worktree_path.is_dir())

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
        ) -> None:
            (completed.worktree_path / "late-change.txt").write_text(
                "not preserved\n",
                encoding="utf-8",
            )
            prepare(store, record)

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
        self.assertEqual(retained.retirement["phase"], "retiring")
        self.assertTrue(retained.worktree_path.is_dir())
        self.assertEqual(retained.retirement["retirement_attempts"], 1)

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


if __name__ == "__main__":
    unittest.main()
