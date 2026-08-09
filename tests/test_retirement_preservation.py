from __future__ import annotations

import io
import json
import subprocess
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from contextlib import redirect_stdout
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
        mission = self.load_mission(quiescence=None)
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
        (self.target_repo / ".git").rename(self.root / "detached-git-metadata")
        mission = self.load_mission()
        completed = self.completed_session(mission)

        with self.assertRaisesRegex(AlbertError, "Worktree Identity"):
            mission.preserve_retirement_unit(
                completed.session_id,
                expected_revision=completed.revision,
                correlation_id="preserve-unregistered",
            )
        blocked = self.load_mission().sessions[completed.session_id]
        self.assertEqual(blocked.retirement["phase"], "preservation-blocked")
        self.assertTrue(blocked.preservation_budget["bound"])

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
