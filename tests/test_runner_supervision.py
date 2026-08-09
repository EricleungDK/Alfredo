from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from contextlib import redirect_stdout
import io

from albert_mvp.core import (
    AlbertError,
    AlbertMission,
    EvidencePackage,
    LocalAgentSession,
    RunnerObservation,
    _process_identity,
)
from albert_mvp.workspace import (
    AgentConsoleHistoryService,
    WorkspaceSnapshotService,
    WorkstationActionService,
)
from albert_mvp.cli import main
from albert_mvp.server import serve


ISSUE_BODY = """Status: ready-for-agent
Type: AFK
Risk: Medium
Suggested agent: fake-local
Assigned agent: fake-local

## Parent

PRD.md

## What to build

Exercise deterministic runner supervision.

## Acceptance criteria

- [ ] Supervision remains receipt-bound.

## Blocked by

None - can start immediately
"""


class RunnerSupervisionTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.target_repo = self.root / "target"
        self.target_repo.mkdir()
        self.tracker = self.root / "tracker"
        (self.tracker / "issues").mkdir(parents=True)
        (self.tracker / "PRD.md").write_text("# Supervision PRD\n", encoding="utf-8")
        (self.tracker / "issues" / "01-supervision.md").write_text(
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

    def load_mission(self) -> AlbertMission:
        return AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="mission-supervision",
            agent_config_path=self.agent_config,
        ).load()

    def running_session(self, mission: AlbertMission):
        identity = _process_identity(os.getpid())
        session = LocalAgentSession(
            session_id="session-ISS-01-1",
            issue_id="ISS-01",
            assigned_agent="fake-local",
            worktree_path=mission._session_worktree_path("session-ISS-01-1"),
            task_packet={},
            status="running",
            runner_started_at="2026-08-09T10:00:00Z",
            runner_pid=os.getpid(),
            runner_identity=identity,
            runner_process_pid=os.getpid(),
            runner_process_identity=identity,
            runner_operation_id="runner:mission-supervision:session-ISS-01-1:1",
        )
        mission.sessions[session.session_id] = session
        mission._persist()
        return mission.sessions[session.session_id]

    def test_healthy_observation_advances_only_the_cursor_and_stays_silent(self) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        session_before = session.to_dict()
        timeline_before = list(mission.timeline)

        receipt = mission.observe_runner(
            RunnerObservation(
                source_id="runner-events",
                source_incarnation="boot-a",
                sequence=1,
                mission_id=mission.mission_id,
                session_id=session.session_id,
                session_revision=session.revision,
                runner_operation_id=session.runner_operation_id,
                owner_signal="live-exact",
                process_group_signal="live-exact",
                worktree_identity=session.worktree_identity,
                result_signal="absent",
            )
        )

        self.assertEqual(receipt.outcome, "no-change")
        self.assertEqual(receipt.effect, "none")
        self.assertEqual(mission.sessions[session.session_id].to_dict(), session_before)
        self.assertEqual(mission.timeline, timeline_before)
        self.assertEqual(mission.supervision_state()["attentions"], {})
        self.assertEqual(
            mission.supervision_state()["observers"]["runner-events"]["cursor"],
            1,
        )
        snapshots = WorkspaceSnapshotService(mission)
        self.assertEqual(AgentConsoleHistoryService(snapshots).history(), ())

    def test_startup_keeps_a_healthy_runner_silent_and_does_not_create_an_event(
        self,
    ) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        before = json.loads(mission.runtime_path.read_text(encoding="utf-8"))

        with patch.object(
            AlbertMission,
            "_probe_runner_boundary",
            return_value=("live-exact", "live-exact"),
        ):
            restarted = self.load_mission()

        after = json.loads(restarted.runtime_path.read_text(encoding="utf-8"))
        self.assertEqual(after, before)
        self.assertEqual(restarted.sessions[session.session_id].status, "running")
        self.assertNotIn(
            "startup-reconciliation",
            restarted.supervision_state()["observers"],
        )

    def test_invalid_observation_boundaries_are_rejected_before_cursor_delivery(
        self,
    ) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        invalid = RunnerObservation(
            source_id="runner-events",
            source_incarnation="boot-a",
            sequence=True,
            mission_id=mission.mission_id,
            session_id=session.session_id,
            session_revision=session.revision,
            runner_operation_id=session.runner_operation_id,
            owner_signal="trusted-by-observer",
            process_group_signal="live-exact",
            worktree_identity=session.worktree_identity,
            result_signal="absent",
        )

        with self.assertRaises(AlbertError):
            mission.observe_runner(invalid)

        self.assertEqual(mission.supervision_state()["observers"], {})

    def test_malformed_session_supervision_boundaries_fail_runtime_load(self) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        baseline = json.loads(mission.runtime_path.read_text(encoding="utf-8"))

        for field_name, invalid_value in (
            ("revision", "not-an-integer"),
            ("automatic_recovery_count", -1),
            ("runner_pid", "not-a-pid"),
            ("runner_identity", {"not": "a string"}),
            ("supervision_receipt_id", 42),
            ("runner_result", "not-an-object"),
        ):
            corrupted = json.loads(json.dumps(baseline))
            corrupted["sessions"][session.session_id][field_name] = invalid_value
            mission.runtime_path.write_text(json.dumps(corrupted), encoding="utf-8")
            with self.subTest(field_name=field_name), self.assertRaises(AlbertError):
                self.load_mission()

        malformed_ledger = json.loads(json.dumps(baseline))
        malformed_ledger["supervision"]["receipts"] = {"bad-receipt": {}}
        mission.runtime_path.write_text(
            json.dumps(malformed_ledger),
            encoding="utf-8",
        )
        with self.assertRaises(AlbertError):
            self.load_mission()

    def test_runner_result_must_match_its_terminal_session_boundary(self) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        baseline = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        result = {
            "mission_id": mission.mission_id,
            "session_id": session.session_id,
            "runner_operation_id": session.runner_operation_id,
            "worktree_identity": session.worktree_identity,
            "status": "completed",
            "runner_exit_status": 0,
            "runner_ended_at": "2026-08-09T10:01:00Z",
            "evidence": None,
            "evidence_valid": False,
            "evidence_correlation_id": "",
            "artifacts": {},
        }
        result["digest"] = mission._runner_result_digest(result)
        baseline["sessions"][session.session_id]["runner_result"] = result

        corruptions = (
            ("status", "running"),
            ("mission_id", "another-mission"),
            ("session_id", "another-session"),
            ("runner_operation_id", "another-operation"),
            ("worktree_identity", "another-worktree"),
            ("digest", "sha256:" + "0" * 64),
        )
        for field_name, invalid_value in corruptions:
            corrupted = json.loads(json.dumps(baseline))
            corrupted["sessions"][session.session_id]["runner_result"][
                field_name
            ] = invalid_value
            mission.runtime_path.write_text(json.dumps(corrupted), encoding="utf-8")
            with self.subTest(field_name=field_name), self.assertRaises(AlbertError):
                self.load_mission()

    def test_pending_supervision_records_must_share_one_mission_session_and_effect(
        self,
    ) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        session.worktree_path.mkdir(parents=True)
        session.worktree_identity = mission.current_worktree_identity(session.session_id)
        mission._persist_session_update(session, expected_statuses={"running"})
        session = mission.sessions[session.session_id]
        observation = RunnerObservation(
            source_id="runner-events",
            source_incarnation="boot-a",
            sequence=1,
            mission_id=mission.mission_id,
            session_id=session.session_id,
            session_revision=session.revision,
            runner_operation_id=session.runner_operation_id,
            owner_signal="absent",
            process_group_signal="absent",
            worktree_identity=session.worktree_identity,
            result_signal="absent",
        )
        with patch.object(mission, "_apply_supervision_intent", return_value=None):
            mission.observe_runner(observation)
        baseline = json.loads(mission.runtime_path.read_text(encoding="utf-8"))
        receipt_id, receipt = next(iter(baseline["supervision"]["receipts"].items()))
        attention_id, attention = next(
            iter(baseline["supervision"]["attentions"].items())
        )
        intent_id, intent = next(iter(baseline["supervision"]["intents"].items()))
        self.assertEqual(receipt["effect"], "recover-same-session")

        corruptions = (
            ("receipt-mission", ("receipts", receipt_id, "mission_id"), "other"),
            ("receipt-session", ("receipts", receipt_id, "session_id"), "other"),
            ("attention-mission", ("attentions", attention_id, "mission_id"), "other"),
            ("attention-session", ("attentions", attention_id, "session_id"), "other"),
            ("intent-mission", ("intents", intent_id, "mission_id"), "other"),
            ("intent-session", ("intents", intent_id, "session_id"), "other"),
            (
                "intent-effect",
                ("intents", intent_id, "effect"),
                "reconcile-result",
            ),
            (
                "attention-effect",
                ("attentions", attention_id, "next_effect"),
                "reconcile-result",
            ),
            (
                "receipt-effect",
                ("receipts", receipt_id, "effect"),
                "reconcile-result",
            ),
            (
                "recover-result-signal",
                ("intents", intent_id, "result_signal"),
                "exact-valid",
            ),
        )
        for label, (collection, record_id, field_name), invalid_value in corruptions:
            corrupted = json.loads(json.dumps(baseline))
            corrupted["supervision"][collection][record_id][field_name] = invalid_value
            mission.runtime_path.write_text(json.dumps(corrupted), encoding="utf-8")
            with self.subTest(label=label), self.assertRaises(AlbertError):
                self.load_mission()

    def test_exact_dead_runner_is_recovered_once_and_semantic_replay_returns_receipt(
        self,
    ) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        session.worktree_path.mkdir(parents=True)
        session.worktree_identity = mission.current_worktree_identity(session.session_id)
        mission._persist_session_update(session, expected_statuses={"running"})
        session = mission.sessions[session.session_id]
        observation = RunnerObservation(
            source_id="runner-events",
            source_incarnation="boot-a",
            sequence=1,
            mission_id=mission.mission_id,
            session_id=session.session_id,
            session_revision=session.revision,
            runner_operation_id=session.runner_operation_id,
            owner_signal="absent",
            process_group_signal="absent",
            worktree_identity=session.worktree_identity,
            result_signal="absent",
        )
        writes: list[dict[str, object]] = []
        real_write = mission._write_runtime_payload

        def capture_write(payload):
            writes.append(json.loads(json.dumps(payload)))
            real_write(payload)

        with (
            patch.object(
                mission,
                "_probe_runner_boundary",
                return_value=("absent", "absent"),
            ),
            patch.object(mission, "_write_runtime_payload", side_effect=capture_write),
        ):
            receipt = mission.observe_runner(observation)

        self.assertEqual(receipt.outcome, "recovered")
        self.assertEqual(receipt.effect, "recover-same-session")
        self.assertGreaterEqual(len(writes), 2)
        delivered = writes[0]
        delivered_supervision = delivered["supervision"]
        self.assertEqual(
            delivered_supervision["observers"]["runner-events"]["cursor"],
            1,
        )
        self.assertEqual(
            list(delivered_supervision["attentions"].values())[0]["disposition"],
            "open",
        )
        self.assertEqual(
            list(delivered_supervision["intents"].values())[0]["status"],
            "pending",
        )
        self.assertEqual(delivered["sessions"][session.session_id]["status"], "running")

        recovered = mission.sessions[session.session_id]
        self.assertEqual(recovered.status, "queued")
        self.assertEqual(recovered.worktree_path, session.worktree_path)
        self.assertEqual(recovered.worktree_identity, session.worktree_identity)
        self.assertEqual(recovered.automatic_recovery_count, 1)
        self.assertIsNone(recovered.runner_pid)
        snapshots = WorkspaceSnapshotService(mission)
        projected = snapshots.snapshot().missions[0]
        self.assertEqual(projected.attention, ())
        self.assertEqual(projected.sessions[0].supervision_outcome, "recovered")
        history = AgentConsoleHistoryService(snapshots).history()
        self.assertEqual(len(history), 1)
        self.assertEqual(history[0].action_phase, "runner-recovered")
        self.assertIn(receipt.receipt_id, history[0].content)

        restarted = self.load_mission()
        replay = RunnerObservation(
            source_id="reconciliation-sweep",
            source_incarnation="boot-b",
            sequence=1,
            mission_id=observation.mission_id,
            session_id=observation.session_id,
            session_revision=observation.session_revision,
            runner_operation_id=observation.runner_operation_id,
            owner_signal=observation.owner_signal,
            process_group_signal=observation.process_group_signal,
            worktree_identity=observation.worktree_identity,
            result_signal=observation.result_signal,
        )
        replayed = restarted.observe_runner(replay)

        self.assertEqual(replayed, receipt)
        self.assertEqual(
            restarted.sessions[session.session_id].automatic_recovery_count,
            1,
        )

    def test_restart_applies_a_durable_intent_left_after_observer_delivery(self) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        session.worktree_path.mkdir(parents=True)
        session.worktree_identity = mission.current_worktree_identity(session.session_id)
        mission._persist_session_update(session, expected_statuses={"running"})
        session = mission.sessions[session.session_id]
        observation = RunnerObservation(
            source_id="runner-events",
            source_incarnation="boot-a",
            sequence=1,
            mission_id=mission.mission_id,
            session_id=session.session_id,
            session_revision=session.revision,
            runner_operation_id=session.runner_operation_id,
            owner_signal="absent",
            process_group_signal="absent",
            worktree_identity=session.worktree_identity,
            result_signal="absent",
        )

        with patch.object(
            mission,
            "_apply_supervision_intent",
            side_effect=RuntimeError("observer stopped after durable delivery"),
        ):
            with self.assertRaisesRegex(RuntimeError, "durable delivery"):
                mission.observe_runner(observation)

        durable = mission.supervision_state()
        self.assertEqual(durable["observers"]["runner-events"]["cursor"], 1)
        self.assertEqual(list(durable["intents"].values())[0]["status"], "pending")
        with patch.object(
            AlbertMission,
            "_probe_runner_boundary",
            return_value=("absent", "absent"),
        ):
            restarted = self.load_mission()

        recovered = restarted.sessions[session.session_id]
        self.assertEqual(recovered.status, "queued")
        self.assertEqual(recovered.automatic_recovery_count, 1)
        receipt = restarted.supervision_state()["receipts"][
            recovered.supervision_receipt_id
        ]
        self.assertEqual(receipt["outcome"], "recovered")

    def test_exact_late_result_is_reconciled_instead_of_rerun(self) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        session.worktree_path.mkdir(parents=True)
        session.worktree_identity = mission.current_worktree_identity(session.session_id)
        mission._persist_session_update(session, expected_statuses={"running"})
        session = mission.sessions[session.session_id]
        session.status = "evidence-ready"
        session.runner_exit_status = 0
        session.evidence = EvidencePackage(
            changed_files=["result.txt"],
            diff_summary="Created the requested result.",
            commands_run=["fake-local"],
            test_results="Focused tests passed.",
            known_risks="None.",
            proposed_context_updates="None.",
        )
        session.evidence_valid = True
        session.evidence_correlation_id = (
            f"evidence:{mission.mission_id}:{session.session_id}"
        )
        candidate = mission._persist_runner_result_candidate(session)
        observation = RunnerObservation(
            source_id="runner-events",
            source_incarnation="boot-a",
            sequence=1,
            mission_id=mission.mission_id,
            session_id=session.session_id,
            session_revision=candidate.revision,
            runner_operation_id=candidate.runner_operation_id,
            owner_signal="absent",
            process_group_signal="absent",
            worktree_identity=candidate.worktree_identity,
            result_signal="exact-valid",
            result_digest=candidate.runner_result["digest"],
        )

        with patch.object(
            mission,
            "_probe_runner_boundary",
            return_value=("absent", "absent"),
        ):
            receipt = mission.observe_runner(observation)

        reconciled = mission.sessions[session.session_id]
        self.assertEqual(receipt.outcome, "result-reconciled")
        self.assertEqual(receipt.effect, "reconcile-result")
        self.assertEqual(reconciled.status, "evidence-ready")
        self.assertTrue(reconciled.evidence_valid)
        self.assertEqual(reconciled.automatic_recovery_count, 0)
        self.assertIsNone(reconciled.runner_pid)

    def test_reused_process_identity_fails_closed_without_recovery(self) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        session.worktree_path.mkdir(parents=True)
        session.worktree_identity = mission.current_worktree_identity(session.session_id)
        mission._persist_session_update(session, expected_statuses={"running"})
        session = mission.sessions[session.session_id]
        observation = RunnerObservation(
            source_id="runner-events",
            source_incarnation="boot-a",
            sequence=1,
            mission_id=mission.mission_id,
            session_id=session.session_id,
            session_revision=session.revision,
            runner_operation_id=session.runner_operation_id,
            owner_signal="absent",
            process_group_signal="absent",
            worktree_identity=session.worktree_identity,
            result_signal="absent",
        )

        with patch.object(
            mission,
            "_probe_runner_boundary",
            return_value=("reused", "absent"),
        ):
            receipt = mission.observe_runner(observation)

        self.assertEqual(receipt.outcome, "decision-needed")
        self.assertEqual(receipt.effect, "mission-commander-decision")
        self.assertEqual(mission.sessions[session.session_id].status, "running")
        self.assertEqual(
            mission.sessions[session.session_id].supervision_receipt_id,
            receipt.receipt_id,
        )
        self.assertEqual(
            mission.supervision_state()["attentions"][receipt.attention_id][
                "disposition"
            ],
            "open",
        )

    def test_unobservable_process_token_scan_is_not_absence_proof(self) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        session.runner_process_token = "runner-token"

        with patch.object(
            mission,
            "_probe_process_identity",
            return_value="absent",
        ), patch(
            "albert_mvp.core._probe_process_token_pids",
            return_value=(set(), False),
        ):
            owner, process_group = mission._probe_runner_boundary(session)

        self.assertEqual(owner, "absent")
        self.assertEqual(process_group, "unavailable")

    def test_unavailable_contradictory_and_worktree_mismatch_observations_fail_closed(
        self,
    ) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        session.worktree_path.mkdir(parents=True)
        session.worktree_identity = mission.current_worktree_identity(session.session_id)
        mission._persist_session_update(session, expected_statuses={"running"})
        session = mission.sessions[session.session_id]
        cases = [
            ("unavailable", "unavailable", session.worktree_identity, "absent", ""),
            (
                "live-exact",
                "live-exact",
                session.worktree_identity,
                "exact-valid",
                "sha256:advisory-only",
            ),
            ("absent", "absent", "managed-directory:mismatch", "absent", ""),
        ]
        receipts = []
        for sequence, (owner, group, worktree, result, digest) in enumerate(cases, start=1):
            receipts.append(
                mission.observe_runner(
                    RunnerObservation(
                        source_id="runner-events",
                        source_incarnation="boot-a",
                        sequence=sequence,
                        mission_id=mission.mission_id,
                        session_id=session.session_id,
                        session_revision=session.revision,
                        runner_operation_id=session.runner_operation_id,
                        owner_signal=owner,
                        process_group_signal=group,
                        worktree_identity=worktree,
                        result_signal=result,
                        result_digest=digest,
                    )
                )
            )

        self.assertEqual(
            [receipt.outcome for receipt in receipts],
            ["decision-needed", "decision-needed", "decision-needed"],
        )
        self.assertEqual(
            [receipt.effect for receipt in receipts],
            ["mission-commander-decision"] * 3,
        )
        self.assertEqual(len({receipt.receipt_id for receipt in receipts}), 3)
        self.assertEqual(mission.sessions[session.session_id].status, "running")
        self.assertEqual(
            mission.sessions[session.session_id].supervision_receipt_id,
            receipts[-1].receipt_id,
        )
        self.assertEqual(len(mission.supervision_state()["attentions"]), 3)

    def test_failed_automatic_recovery_ends_automation_and_requests_a_decision(
        self,
    ) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        session.worktree_path.mkdir(parents=True)
        session.worktree_identity = mission.current_worktree_identity(session.session_id)
        session.automatic_recovery_count = 1
        mission._persist_session_update(session, expected_statuses={"running"})
        session = mission.sessions[session.session_id]
        observation = RunnerObservation(
            source_id="runner-events",
            source_incarnation="boot-recovery",
            sequence=1,
            mission_id=mission.mission_id,
            session_id=session.session_id,
            session_revision=session.revision,
            runner_operation_id=session.runner_operation_id,
            owner_signal="absent",
            process_group_signal="absent",
            worktree_identity=session.worktree_identity,
            result_signal="absent",
        )

        with patch.object(
            mission,
            "_probe_runner_boundary",
            return_value=("absent", "absent"),
        ):
            receipt = mission.observe_runner(observation)

        failed = mission.sessions[session.session_id]
        self.assertEqual(receipt.outcome, "decision-needed")
        self.assertEqual(receipt.effect, "mission-commander-decision")
        self.assertEqual(failed.status, "failed")
        self.assertEqual(failed.automatic_recovery_count, 1)
        self.assertEqual(failed.supervision_receipt_id, receipt.receipt_id)
        self.assertIn("automatic recovery failed", failed.task_packet["runner_failure"])
        self.assertIsNone(failed.runner_pid)

        snapshots = WorkspaceSnapshotService(mission)
        projection = snapshots.snapshot()
        projected_mission = projection.missions[0]
        projected_session = projected_mission.sessions[0]
        self.assertEqual(projected_session.supervision_receipt_id, receipt.receipt_id)
        self.assertEqual(projected_session.supervision_outcome, "decision-needed")
        self.assertEqual(projected_session.automatic_recovery_count, 1)
        self.assertEqual(len(projected_mission.attention), 1)
        self.assertEqual(projected_mission.attention[0].kind, "runner-supervision")
        self.assertEqual(projected_mission.attention[0].entity_id, session.session_id)
        messages = AgentConsoleHistoryService(snapshots).history()
        self.assertEqual(len(messages), 1)
        self.assertEqual(messages[0].correlation_id, receipt.correlation_id)
        self.assertEqual(messages[0].action_phase, "runner-decision-needed")

        projection = snapshots.snapshot()
        with patch.object(
            AlbertMission,
            "_validate_target_repository_boundary",
        ):
            acknowledgement = WorkstationActionService(snapshots).submit(
                correlation_id="manual-retry-after-supervision-failure",
                action_type="issue-retry",
                actor="mission-commander",
                expected_revision=projection.revision,
                target_kind="agent-session",
                target_id=failed.session_id,
                mission_id=mission.mission_id,
                issue_id=failed.issue_id,
                session_id=failed.session_id,
                reason="Retry manually after inspecting the runner failure.",
            )

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        retry = mission.sessions[acknowledgement.session_id]
        self.assertEqual(retry.status, "queued")
        self.assertEqual(
            retry.task_packet["repair_context"]["next_action"],
            "mission-commander-manual-retry",
        )
        self.assertEqual(
            retry.task_packet["repair_context"]["review_reason"],
            "Retry manually after inspecting the runner failure.",
        )

    def test_cli_and_persistent_transport_replay_the_same_observation_receipt(self) -> None:
        mission = self.load_mission()
        session = self.running_session(mission)
        session.worktree_path.mkdir(parents=True)
        session.worktree_identity = mission.current_worktree_identity(session.session_id)
        mission._persist_session_update(session, expected_statuses={"running"})
        session = mission.sessions[session.session_id]
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
        observation_args = [
            "runner-observe",
            *common,
            "--source-id",
            "cli-observer",
            "--source-incarnation",
            "boot-cli",
            "--sequence",
            "1",
            "--observation-mission-id",
            mission.mission_id,
            "--session-id",
            session.session_id,
            "--session-revision",
            str(session.revision),
            "--runner-operation-id",
            session.runner_operation_id,
            "--owner-signal",
            "live-exact",
            "--process-group-signal",
            "live-exact",
            "--worktree-identity",
            session.worktree_identity,
            "--result-signal",
            "absent",
        ]

        first_output = io.StringIO()
        second_output = io.StringIO()
        with (
            patch.object(
                AlbertMission,
                "_probe_runner_boundary",
                return_value=("live-exact", "live-exact"),
            ),
            redirect_stdout(first_output),
        ):
            self.assertEqual(main(observation_args), 0)
        with (
            patch.object(
                AlbertMission,
                "_probe_runner_boundary",
                return_value=("live-exact", "live-exact"),
            ),
            redirect_stdout(second_output),
        ):
            self.assertEqual(main(observation_args), 0)

        first_receipt = json.loads(first_output.getvalue())
        self.assertEqual(json.loads(second_output.getvalue()), first_receipt)

        transport_args = list(observation_args)
        transport_args[transport_args.index("cli-observer")] = "persistent-observer"
        transport_args[transport_args.index("boot-cli")] = "boot-server"
        requests = io.StringIO(
            json.dumps({"id": "observation", "argv": transport_args}) + "\n"
        )
        responses = io.StringIO()
        with patch.object(
            AlbertMission,
            "_probe_runner_boundary",
            return_value=("live-exact", "live-exact"),
        ):
            serve(requests, responses)
        response = json.loads(responses.getvalue())

        self.assertTrue(response["success"], response)
        self.assertEqual(json.loads(response["stdout"]), first_receipt)

    def test_real_session_runner_persists_operation_worktree_and_result_boundaries(
        self,
    ) -> None:
        mission = self.load_mission()
        mission.approve_issue("ISS-01")
        with patch.object(mission, "_validate_target_repository_boundary"):
            queued = mission.launch_issue("ISS-01")

        def prepare_worktree(session: LocalAgentSession) -> None:
            session.worktree_path.mkdir(parents=True, exist_ok=True)
            session.repository_snapshot = {
                "kind": "directory",
                "preparation": {"schema_version": 1, "state": "baseline-captured"},
                "review_baseline": {},
            }

        with (
            patch.object(mission, "_validate_target_repository_boundary"),
            patch.object(mission, "_ensure_session_worktree", side_effect=prepare_worktree),
        ):
            completed = mission.run_session(queued.session_id)

        self.assertEqual(completed.status, "evidence-ready")
        self.assertTrue(completed.runner_operation_id.startswith("runner-operation:"))
        self.assertTrue(completed.worktree_identity.startswith("managed-directory:"))
        self.assertEqual(completed.runner_result["mission_id"], mission.mission_id)
        self.assertEqual(completed.runner_result["session_id"], completed.session_id)
        candidate = {
            key: value for key, value in completed.runner_result.items() if key != "digest"
        }
        self.assertEqual(
            completed.runner_result["digest"],
            mission._runner_result_digest(candidate),
        )
        self.assertGreater(completed.revision, 0)
        self.assertIsNone(completed.runner_pid)
        self.assertIsNone(completed.runner_process_pid)

    def test_typed_failure_from_recovered_runner_requests_commander_decision(self) -> None:
        mission = self.load_mission()
        mission.approve_issue("ISS-01")
        with patch.object(mission, "_validate_target_repository_boundary"):
            queued = mission.launch_issue("ISS-01")
        queued.automatic_recovery_count = 1
        mission._persist_session_update(queued, expected_statuses={"queued"})

        def prepare_worktree(session: LocalAgentSession) -> None:
            session.worktree_path.mkdir(parents=True, exist_ok=True)
            session.repository_snapshot = {
                "kind": "directory",
                "preparation": {"schema_version": 1, "state": "baseline-captured"},
                "review_baseline": {},
            }

        with (
            patch.object(mission, "_validate_target_repository_boundary"),
            patch.object(mission, "_ensure_session_worktree", side_effect=prepare_worktree),
            patch.object(mission, "_run_fake_agent", side_effect=RuntimeError("retry failed")),
        ):
            with self.assertRaises(AlbertError):
                mission.run_session(queued.session_id)

        failed = mission.sessions[queued.session_id]
        self.assertEqual(failed.status, "failed")
        self.assertEqual(failed.automatic_recovery_count, 1)
        receipt = mission.supervision_state()["receipts"][
            failed.supervision_receipt_id
        ]
        self.assertEqual(receipt["outcome"], "decision-needed")
        attention = mission.supervision_state()["attentions"][
            receipt["attention_id"]
        ]
        self.assertEqual(attention["kind"], "automatic-recovery-failed")
        self.assertEqual(attention["next_effect"], "mission-commander-decision")


if __name__ == "__main__":
    unittest.main()
