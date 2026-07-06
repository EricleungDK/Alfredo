from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime, timezone
import io
import json
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

import albert_mvp.workspace as workspace_module
from albert_mvp.cli import main
from albert_mvp.core import (
    AlbertError,
    AlbertMission,
    DelegationDecision,
    EvidencePackage,
    EvidenceValidationError,
    LocalAgentSession,
)
from albert_mvp.workspace import (
    AgentConsoleHistoryService,
    ActivityJournalService,
    ConversationScope,
    MissionDraftService,
    ReviewWorkspaceService,
    ShellTerminalService,
    WorkspaceAction,
    WorkspaceQueueService,
    WorkspacePersistenceError,
    WorkspaceRevisionGapError,
    WorkspaceSnapshotService,
    WorkspaceScopeMismatchError,
    WorkspaceStaleActionError,
    WorkspaceSyncService,
    WorkstationActionService,
    WorkingContextCurationError,
    WorkingContextService,
)


ISSUE = """Status: ready-for-agent
Type: AFK

## Parent

PRD.md

## What to build

Restore the workspace session.

## Acceptance criteria

- [ ] The canonical snapshot is visible.

## Blocked by

None - can start immediately
"""


class WorkspaceSnapshotTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.target_repo = self.root / "target"
        self.target_repo.mkdir()
        self.tracker = self.root / "tracker"
        (self.tracker / "issues").mkdir(parents=True)
        (self.tracker / "PRD.md").write_text("# Command Deck Mission\n", encoding="utf-8")
        (self.tracker / "issues" / "01-restore.md").write_text(ISSUE, encoding="utf-8")
        self.runtime = self.root / "runtime"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_cli_returns_versioned_canonical_snapshot(self) -> None:
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workspace-snapshot",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                ]
            )

        snapshot = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(snapshot["schema_version"], 1)
        self.assertEqual(snapshot["revision"], 1)
        self.assertEqual(snapshot["workspace_session"]["status"], "ready")
        self.assertEqual(snapshot["active_mission"]["id"], "command-deck")
        self.assertEqual(snapshot["active_mission"]["title"], "Command Deck Mission")
        self.assertEqual(snapshot["conversation_scope"]["kind"], "working-directory")
        self.assertEqual(snapshot["operations_view"], "mission-board")

    def test_shell_terminal_executes_auto_allowed_command_without_accepting_output(
        self,
    ) -> None:
        snapshots = self.load_service()
        canonical_before = snapshots.snapshot().to_dict()

        result = ShellTerminalService(snapshots).submit(
            correlation_id="terminal-auto-allowed-1",
            command="python3 -m unittest --help",
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )

        self.assertEqual(result.command_id, "terminal-command-000001")
        self.assertEqual(result.classification, "auto-allowed")
        self.assertEqual(result.status, "completed")
        self.assertEqual(result.exit_code, 0)
        self.assertIn("usage:", result.stdout.lower())
        self.assertEqual(snapshots.snapshot().to_dict(), canonical_before)
        restored_history = AgentConsoleHistoryService(self.load_service()).history()
        restored_journal = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(
            [message.content for message in restored_history],
            ["Shell Terminal command completed with exit code 0: terminal-command-000001."],
        )
        self.assertEqual(len(restored_journal.entries), 1)
        self.assertEqual(restored_journal.entries[0].action_type, "shell-command-completed")
        self.assertNotIn("usage:", restored_journal.entries[0].summary.lower())

    def test_shell_terminal_waits_for_frontier_approval_before_execution(self) -> None:
        snapshots = self.load_service()
        command = "python3 -c \"print('frontier approved')\""
        snapshots._primary_mission.record_command_approval(
            command,
            "frontier-approvable",
        )
        terminal = ShellTerminalService(snapshots)

        pending = terminal.submit(
            correlation_id="terminal-frontier-pending-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )

        self.assertEqual(pending.classification, "frontier-approvable")
        self.assertEqual(pending.status, "pending-approval")
        self.assertIsNone(pending.exit_code)
        self.assertEqual(pending.stdout, "")
        approved = terminal.approve(
            command_id=pending.command_id,
            approver="frontier-model",
        )
        self.assertEqual(approved.status, "completed")
        self.assertEqual(approved.exit_code, 0)
        self.assertEqual(approved.stdout.strip(), "frontier approved")

    def test_shell_terminal_denial_prevents_human_required_command_execution(self) -> None:
        snapshots = self.load_service()
        marker = self.target_repo / "denied.txt"
        command = (
            "python3 -c \"from pathlib import Path; "
            "Path('denied.txt').write_text('must not run')\""
        )
        terminal = ShellTerminalService(snapshots)
        pending = terminal.submit(
            correlation_id="terminal-human-deny-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )

        denied = terminal.deny(
            command_id=pending.command_id,
            decider="mission-commander",
            reason="This command is outside the intended task.",
        )

        self.assertEqual(pending.classification, "human-required")
        self.assertEqual(denied.status, "denied")
        self.assertIsNone(denied.exit_code)
        self.assertEqual(denied.stdout, "")
        self.assertFalse(marker.exists())
        with self.assertRaisesRegex(AlbertError, "already denied"):
            terminal.approve(
                command_id=pending.command_id,
                approver="mission-commander",
            )

    def test_additional_path_grant_authorizes_bounded_external_read_access(self) -> None:
        snapshots = self.load_service()
        external = self.root / "external-docs"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)

        with patch("albert_mvp.workspace.datetime") as clock:
            clock.now.return_value = datetime(2026, 6, 27, 8, 0, tzinfo=timezone.utc)
            grant = terminal.create_path_grant(
                correlation_id="path-grant-read-1",
                expected_revision=0,
                path=str(external),
                access_level="read",
                duration_seconds=300,
                requester="mission-commander",
            )
            result = terminal.submit(
                correlation_id="terminal-external-read-1",
                command="python3 -m unittest --help",
                working_directory=str(external),
                requested_paths=[str(external)],
                requester="mission-commander",
                access_level="read",
            )

        self.assertEqual(grant.grant_id, "path-grant-000001")
        self.assertEqual(grant.path, str(external.resolve()))
        self.assertEqual(grant.access_level, "read")
        self.assertEqual(grant.duration_seconds, 300)
        self.assertEqual(grant.expires_at, "2026-06-27T08:05:00Z")
        self.assertEqual(result.status, "completed")
        self.assertEqual(result.exit_code, 0)

    def test_additional_path_grant_rejects_stale_terminal_revision(self) -> None:
        snapshots = self.load_service()
        external = self.root / "external-stale"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)

        with self.assertRaises(WorkspaceStaleActionError) as raised:
            terminal.create_path_grant(
                correlation_id="path-grant-stale-1",
                expected_revision=7,
                path=str(external),
                access_level="read",
                duration_seconds=300,
                requester="mission-commander",
            )

        self.assertEqual(raised.exception.expected_revision, 7)
        self.assertEqual(raised.exception.current_revision, 0)

    def test_expired_additional_path_grant_stops_authorizing_external_access(self) -> None:
        snapshots = self.load_service()
        external = self.root / "expired-external"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)
        with patch("albert_mvp.workspace.datetime") as clock:
            clock.now.return_value = datetime(2026, 6, 27, 8, 0, tzinfo=timezone.utc)
            terminal.create_path_grant(
                correlation_id="path-grant-expiry-1",
                expected_revision=0,
                path=str(external),
                access_level="read",
                duration_seconds=60,
                requester="mission-commander",
            )
            clock.now.return_value = datetime(2026, 6, 27, 8, 2, tzinfo=timezone.utc)
            with self.assertRaisesRegex(AlbertError, "expired"):
                terminal.submit(
                    correlation_id="terminal-expired-path-1",
                    command="python3 -m unittest --help",
                    working_directory=str(external),
                    requested_paths=[str(external)],
                    requester="mission-commander",
                    access_level="read",
                )

    def test_local_agent_cannot_expand_or_renew_additional_path_grant(self) -> None:
        snapshots = self.load_service()
        external = self.root / "agent-grant"
        external.mkdir()
        terminal = ShellTerminalService(snapshots)
        grant = terminal.create_path_grant(
            correlation_id="path-grant-agent-boundary-1",
            expected_revision=0,
            path=str(external),
            access_level="read",
            duration_seconds=60,
            requester="mission-commander",
        )

        with self.assertRaisesRegex(
            AlbertError,
            "cannot broaden, renew, or change",
        ):
            terminal.change_path_grant(
                grant_id=grant.grant_id,
                path=str(self.root),
                access_level="write",
                duration_seconds=3600,
                requester="local-agent",
            )

        with self.assertRaisesRegex(AlbertError, "no active write"):
            terminal.submit(
                correlation_id="terminal-agent-write-denied-1",
                command="python3 -m unittest --help",
                working_directory=str(external),
                requested_paths=[str(external)],
                requester="local-agent",
                access_level="write",
            )

    def test_cli_executes_shell_terminal_command_and_restores_metadata_without_bytes(
        self,
    ) -> None:
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        submit_output = io.StringIO()
        with redirect_stdout(submit_output):
            submit_exit = main(
                [
                    "shell-terminal-submit",
                    *common,
                    "--correlation-id",
                    "terminal-cli-submit-1",
                    "--command-text",
                    "python3 -m unittest --help",
                    "--working-directory",
                    str(self.target_repo),
                    "--requester",
                    "mission-commander",
                    "--access-level",
                    "read",
                ]
            )
        submitted = json.loads(submit_output.getvalue())
        inspect_output = io.StringIO()
        with redirect_stdout(inspect_output):
            inspect_exit = main(["shell-terminal", *common])
        projection = json.loads(inspect_output.getvalue())

        self.assertEqual(submit_exit, 0)
        self.assertEqual(inspect_exit, 0)
        self.assertEqual(submitted["status"], "completed")
        self.assertIn("usage:", submitted["stdout"].lower())
        self.assertEqual(projection["schema_version"], 1)
        self.assertEqual(projection["revision"], 1)
        self.assertEqual(projection["commands"][0]["command_id"], "terminal-command-000001")
        self.assertEqual(projection["commands"][0]["status"], "completed")
        self.assertNotIn("stdout", projection["commands"][0])
        self.assertNotIn("stderr", projection["commands"][0])

    def test_cli_creates_and_restores_additional_path_grant(self) -> None:
        external = self.root / "cli-external"
        external.mkdir()
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        create_output = io.StringIO()
        with redirect_stdout(create_output):
            create_exit = main(
                [
                    "additional-path-grant-create",
                    *common,
                    "--correlation-id",
                    "path-grant-cli-1",
                    "--expected-terminal-revision",
                    "0",
                    "--path",
                    str(external),
                    "--access-level",
                    "write",
                    "--duration-seconds",
                    "900",
                    "--requester",
                    "mission-commander",
                ]
            )
        created = json.loads(create_output.getvalue())
        inspect_output = io.StringIO()
        with redirect_stdout(inspect_output):
            inspect_exit = main(["shell-terminal", *common])
        projection = json.loads(inspect_output.getvalue())

        self.assertEqual(create_exit, 0)
        self.assertEqual(inspect_exit, 0)
        self.assertEqual(created["grant_id"], "path-grant-000001")
        self.assertEqual(projection["grants"][0]["path"], str(external.resolve()))
        self.assertEqual(projection["grants"][0]["access_level"], "write")
        self.assertEqual(projection["grants"][0]["duration_seconds"], 900)
        self.assertEqual(projection["grants"][0]["granted_by"], "mission-commander")

    def test_cli_approves_pending_human_required_shell_terminal_command(self) -> None:
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        pending_output = io.StringIO()
        with redirect_stdout(pending_output):
            pending_exit = main(
                [
                    "shell-terminal-submit",
                    *common,
                    "--correlation-id",
                    "terminal-cli-human-1",
                    "--command-text",
                    "python3 -c \"print('human approved')\"",
                    "--working-directory",
                    str(self.target_repo),
                    "--requester",
                    "mission-commander",
                ]
            )
        pending = json.loads(pending_output.getvalue())
        decision_output = io.StringIO()
        with redirect_stdout(decision_output):
            decision_exit = main(
                [
                    "shell-terminal-decision",
                    *common,
                    "--command-id",
                    pending["command_id"],
                    "--decision",
                    "approve",
                    "--actor",
                    "mission-commander",
                ]
            )
        approved = json.loads(decision_output.getvalue())

        self.assertEqual(pending_exit, 0)
        self.assertEqual(pending["status"], "pending-approval")
        self.assertEqual(pending["classification"], "human-required")
        self.assertEqual(decision_exit, 0)
        self.assertEqual(approved["status"], "completed")
        self.assertEqual(approved["stdout"].strip(), "human approved")

    def test_workspace_preferences_restore_after_service_restart(self) -> None:
        first = self.load_service()
        first.update_preferences(
            active_mission_id="command-deck",
            conversation_scope=ConversationScope(
                kind="issue-slice",
                target_id="ISS-01",
                label="Restore workspace session",
            ),
            operations_view="review-workspace",
        )

        restored = self.load_service().snapshot()

        self.assertEqual(restored.active_mission.id, "command-deck")
        self.assertEqual(restored.conversation_scope.kind, "issue-slice")
        self.assertEqual(restored.conversation_scope.target_id, "ISS-01")
        self.assertEqual(restored.operations_view, "review-workspace")

    def test_workstation_continuity_restores_meaningful_state_after_backend_restart(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Keep Alfredo continuity across restart.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        launch = workspace_module.WorkstationActionService(snapshots).submit(
            correlation_id="continuity-launch-1",
            action_type="issue-launch",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
            allowed_paths=["src"],
        )
        evidence_link = f"app-local://evidence/{launch.session_id}"
        mission.record_evidence(
            launch.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Persisted workstation continuity.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused persistence test passed.",
                known_risks="None.",
                proposed_context_updates="No domain model update.",
                artifact_links=[evidence_link],
            ),
        )
        proposal = WorkspaceQueueService(snapshots).propose_issue_contract_change(
            correlation_id="continuity-approval-1",
            expected_revision=launch.revision,
            issue_id="ISS-01",
            source="workstation-card",
            acceptance_criteria=["Continuity restores pending approval records."],
        )
        snapshots.update_preferences(
            active_mission_id="command-deck",
            conversation_scope=ConversationScope(
                kind="issue-slice",
                target_id="ISS-01",
                label="Restore workspace session",
            ),
            operations_view="workspace-queue",
            event_metadata={"correlation_id": "continuity-side-pane-1"},
        )

        restored = self.load_service()
        snapshot = restored.snapshot()
        restored_issue = snapshot.mission_board["issue_slices"][0]
        restored_queue = WorkspaceQueueService(restored).inspect()
        restored_history = AgentConsoleHistoryService(restored).history()
        restored_journal = ActivityJournalService(restored).inspect()

        self.assertEqual(snapshot.workspace_session.workspace_path, str(self.target_repo))
        self.assertEqual(snapshot.conversation_scope.kind, "issue-slice")
        self.assertEqual(snapshot.conversation_scope.target_id, "ISS-01")
        self.assertEqual(snapshot.operations_view, "workspace-queue")
        self.assertEqual(restored_history[0].content, "Keep Alfredo continuity across restart.")
        self.assertEqual(snapshot.missions[0].sessions[0].session_id, launch.session_id)
        self.assertEqual(snapshot.missions[0].sessions[0].status, "evidence-ready")
        self.assertEqual(restored_issue["evidence"]["artifact_links"], [evidence_link])
        self.assertEqual(restored_queue.items[0].item_id, proposal.item_id)
        self.assertEqual(restored_queue.items[0].status, "pending")
        self.assertEqual(
            [entry.action_type for entry in restored_journal.entries],
            ["issue-launch", "evidence-package-submitted"],
        )
        self.assertEqual(restored_journal.entries[1].evidence_links, (evidence_link,))
        self.assertNotIn("raw terminal byte", restored_journal.entries[0].summary)

    def test_review_workspace_lists_complete_and_incomplete_evidence_packages(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        complete_session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            complete_session.session_id,
            EvidencePackage(
                changed_files=["src/app.py", ".env"],
                diff_summary="Added the review workspace projection.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="Document Review Workspace as the evidence decision surface.",
                artifact_links=["app-local://evidence/session-ISS-01-1"],
            ),
        )
        incomplete_session = mission.launch_issue("ISS-01")

        projection = ReviewWorkspaceService(WorkspaceSnapshotService(mission)).inspect()
        items = {item.session_id: item for item in projection.items}

        self.assertEqual(projection.schema_version, 1)
        self.assertEqual(projection.revision, 1)
        self.assertEqual(set(items), {complete_session.session_id, incomplete_session.session_id})
        complete = items[complete_session.session_id]
        self.assertEqual(complete.issue_id, "ISS-01")
        self.assertEqual(complete.evidence_complete, True)
        self.assertEqual(complete.missing_evidence, [])
        self.assertEqual(complete.evidence.diff_summary, "Added the review workspace projection.")
        self.assertEqual(
            complete.evidence.proposed_context_updates,
            "Document Review Workspace as the evidence decision surface.",
        )
        self.assertEqual(complete.visibility_limitations[0].path, ".env")
        self.assertEqual(complete.visibility_limitations[0].classification, "Blocked")
        self.assertEqual(complete.can_accept, True)
        incomplete = items[incomplete_session.session_id]
        self.assertEqual(incomplete.evidence_complete, False)
        self.assertEqual(
            incomplete.missing_evidence,
            [
                "changed_files",
                "diff_summary",
                "commands_run",
                "test_results",
                "known_risks",
                "proposed_context_updates",
            ],
        )
        self.assertEqual(incomplete.can_accept, False)

    def test_locked_issue_contract_edit_creates_queue_proposal_without_mutating_state(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        accepted_criteria = list(mission.issues["ISS-01"].acceptance_criteria)

        acknowledgement = WorkspaceQueueService(snapshots).propose_issue_contract_change(
            correlation_id="proposal-acceptance-criteria-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=[
                "The canonical snapshot is visible.",
                "Queue proposals preserve accepted state until approval.",
            ],
        )
        reloaded = WorkspaceQueueService(self.load_service()).inspect()

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.item_id, "issue-change-command-deck-ISS-01-000001")
        self.assertEqual(mission.issues["ISS-01"].acceptance_criteria, accepted_criteria)
        self.assertEqual(reloaded.revision, acknowledgement.revision)
        self.assertEqual(len(reloaded.items), 1)
        proposal = reloaded.items[0]
        self.assertEqual(proposal.item_id, "issue-change-command-deck-ISS-01-000001")
        self.assertEqual(proposal.item_type, "issue-change-proposal")
        self.assertEqual(proposal.status, "pending")
        self.assertEqual(proposal.mission_id, "command-deck")
        self.assertEqual(proposal.issue_id, "ISS-01")
        self.assertEqual(proposal.source, "issue-slice-inspector")
        self.assertEqual(proposal.requested_action, "Change accepted Issue Slice contract")
        self.assertEqual(proposal.affected_boundary, "acceptance_criteria")
        self.assertEqual(
            proposal.consequence,
            "Approval will reopen ISS-01 for re-review with the proposed governed-field changes.",
        )
        self.assertEqual(
            proposal.proposed_changes["acceptance_criteria"],
            [
                "The canonical snapshot is visible.",
                "Queue proposals preserve accepted state until approval.",
            ],
        )

    def test_workspace_queue_approves_issue_change_proposal_and_reopens_slice(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_issue_contract_change(
            correlation_id="proposal-approve-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Approval applies this proposal."],
        )

        acknowledgement = service.decide(
            correlation_id="proposal-approve-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Accepted by Mission Commander.",
        )
        projected = service.inspect().items[0]

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.item_status, "approved")
        self.assertEqual(acknowledgement.revision, proposal.revision + 1)
        self.assertEqual(projected.status, "approved")
        self.assertEqual(mission.issues["ISS-01"].acceptance_criteria, ["Approval applies this proposal."])
        self.assertEqual(mission.issues["ISS-01"].review_state, "needs-review")
        self.assertEqual(mission.issues["ISS-01"].status, "needs-review")
        self.assertEqual(mission.issues["ISS-01"].locked, False)

    def test_workspace_queue_rejects_and_stale_decisions_preserve_authoritative_state(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        service = WorkspaceQueueService(snapshots)
        accepted_criteria = list(mission.issues["ISS-01"].acceptance_criteria)
        rejected = service.propose_issue_contract_change(
            correlation_id="proposal-reject-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Rejected proposal must not apply."],
        )

        rejection = service.decide(
            correlation_id="proposal-reject-decision-1",
            expected_revision=rejected.revision,
            item_id=rejected.item_id,
            decision="reject",
            reason="Does not match the accepted mission boundary.",
        )
        stale = service.propose_issue_contract_change(
            correlation_id="proposal-stale-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="agent-console",
            acceptance_criteria=["Stale approval must not apply."],
        )
        service.propose_issue_contract_change(
            correlation_id="proposal-stale-bump-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="background-attention",
            acceptance_criteria=["This proposal only bumps the queue revision."],
        )

        with self.assertRaises(WorkspaceStaleActionError):
            service.decide(
                correlation_id="proposal-stale-decision-1",
                expected_revision=stale.revision,
                item_id=stale.item_id,
                decision="approve",
                reason="Stale approval.",
            )

        projected = {item.item_id: item for item in service.inspect().items}
        self.assertEqual(rejection.item_status, "rejected")
        self.assertEqual(projected[rejected.item_id].status, "rejected")
        self.assertEqual(projected[stale.item_id].status, "pending")
        self.assertEqual(mission.issues["ISS-01"].acceptance_criteria, accepted_criteria)
        self.assertEqual(mission.issues["ISS-01"].review_state, "approved")
        self.assertEqual(mission.issues["ISS-01"].locked, True)

    def test_workspace_queue_decision_accepts_typed_workstation_metadata(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_issue_contract_change(
            correlation_id="proposal-workstation-typed-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="workstation-card",
            acceptance_criteria=["Route decisions from the workstation card."],
        )

        acknowledgement = service.decide(
            correlation_id="proposal-workstation-typed-decision-1",
            action_type="workspace-queue-decision",
            actor="mission-commander",
            expected_revision=proposal.revision,
            target_kind="workspace-queue-item",
            target_id=proposal.item_id,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approved from the workstation card.",
        )

        self.assertEqual(acknowledgement.item_status, "approved")
        entry = ActivityJournalService(self.load_service()).inspect().entries[0]
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.action_type, "workspace-queue-decision")
        self.assertEqual(entry.correlation_id, "proposal-workstation-typed-decision-1")

    def test_workspace_queue_decision_rejects_mismatched_workstation_target(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_issue_contract_change(
            correlation_id="proposal-workstation-invalid-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="workstation-card",
            acceptance_criteria=["Reject mismatched workstation targets."],
        )

        with self.assertRaisesRegex(AlbertError, "target id must match item id"):
            service.decide(
                correlation_id="proposal-workstation-invalid-decision-1",
                action_type="workspace-queue-decision",
                actor="mission-commander",
                expected_revision=proposal.revision,
                target_kind="workspace-queue-item",
                target_id="another-item",
                item_id=proposal.item_id,
                decision="approve",
                reason="This must not apply.",
            )

        reloaded = WorkspaceQueueService(self.load_service()).inspect()
        self.assertEqual(reloaded.items[0].status, "pending")

    def test_workstation_action_launches_issue_through_expected_revision_guard(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")

        acknowledgement = workspace_module.WorkstationActionService(snapshots).submit(
            correlation_id="workstation-launch-1",
            action_type="issue-launch",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
            allowed_paths=["src"],
        )

        self.assertEqual(acknowledgement.action_type, "issue-launch")
        self.assertEqual(acknowledgement.issue_id, "ISS-01")
        self.assertEqual(acknowledgement.session_id, "session-ISS-01-1")
        self.assertEqual(acknowledgement.revision, 2)
        self.assertIn("launched", acknowledgement.effect_summary)
        self.assertIn("session-ISS-01-1", mission.sessions)
        self.assertEqual(mission.sessions["session-ISS-01-1"].task_packet["allowed_paths"], ["src"])
        entry = ActivityJournalService(self.load_service()).inspect().entries[0]
        self.assertEqual(entry.actor, "orchestrator")
        self.assertEqual(entry.action_type, "issue-launch")
        self.assertEqual(entry.correlation_id, "workstation-launch-1")

    def test_workstation_action_rejects_stale_launch_without_mutating_sessions(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="revision-bump-before-launch",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=snapshots.snapshot().conversation_scope,
                operations_view="mission-board",
            )
        )

        with self.assertRaises(WorkspaceStaleActionError):
            workspace_module.WorkstationActionService(snapshots).submit(
                correlation_id="workstation-launch-stale-1",
                action_type="issue-launch",
                actor="mission-commander",
                expected_revision=1,
                target_kind="issue-slice",
                target_id="ISS-01",
                issue_id="ISS-01",
            )

        self.assertEqual(mission.sessions, {})

    def test_workstation_action_changes_model_assignment_with_typed_target(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission

        acknowledgement = workspace_module.WorkstationActionService(snapshots).submit(
            correlation_id="workstation-model-assignment-1",
            action_type="model-assignment-change",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
            agent_id="qwen3.6-27b",
            reason="Use the stronger local model.",
        )

        self.assertEqual(acknowledgement.action_type, "model-assignment-change")
        self.assertEqual(acknowledgement.issue_id, "ISS-01")
        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(mission.issues["ISS-01"].assigned_agent, "qwen3.6-27b")
        entry = ActivityJournalService(self.load_service()).inspect().entries[0]
        self.assertEqual(entry.action_type, "model-assignment-change")
        self.assertIn("qwen3.6-27b", entry.summary)

    def test_workstation_action_rejects_mismatched_model_assignment_target(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission

        with self.assertRaisesRegex(AlbertError, "target id must match issue id"):
            workspace_module.WorkstationActionService(snapshots).submit(
                correlation_id="workstation-model-assignment-invalid-1",
                action_type="model-assignment-change",
                actor="mission-commander",
                expected_revision=1,
                target_kind="issue-slice",
                target_id="ISS-02",
                issue_id="ISS-01",
                agent_id="qwen3.6-27b",
                reason="This target does not match.",
            )

        self.assertNotEqual(mission.issues["ISS-01"].assigned_agent, "qwen3.6-27b")

    def test_workstation_action_retries_repairable_session_and_cancels_active_session(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        first_session = mission.launch_issue("ISS-01")
        mission.record_frontier_review(
            first_session.session_id,
            "Needs repair",
            reason="Acceptance criteria are not met.",
        )

        retry = workspace_module.WorkstationActionService(snapshots).submit(
            correlation_id="workstation-retry-1",
            action_type="issue-retry",
            actor="mission-commander",
            expected_revision=1,
            target_kind="agent-session",
            target_id=first_session.session_id,
            issue_id="ISS-01",
            session_id=first_session.session_id,
            reason="Run the repair.",
        )

        self.assertEqual(retry.action_type, "issue-retry")
        self.assertEqual(retry.session_id, "session-ISS-01-2")
        self.assertIn("session-ISS-01-2", mission.sessions)

        cancel = workspace_module.WorkstationActionService(snapshots).submit(
            correlation_id="workstation-cancel-1",
            action_type="session-cancel",
            actor="mission-commander",
            expected_revision=retry.revision,
            target_kind="agent-session",
            target_id="session-ISS-01-2",
            issue_id="ISS-01",
            session_id="session-ISS-01-2",
            reason="Stop this repair run.",
        )

        self.assertEqual(cancel.action_type, "session-cancel")
        self.assertEqual(cancel.session_id, "session-ISS-01-2")
        self.assertEqual(mission.sessions["session-ISS-01-2"].status, "cancelled")
        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual([entry.action_type for entry in entries[-2:]], ["issue-retry", "session-cancel"])

    def test_workstation_action_rejects_retry_and_cancel_without_required_reason(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        first_session = mission.launch_issue("ISS-01")
        mission.record_frontier_review(
            first_session.session_id,
            "Needs repair",
            reason="Acceptance criteria are not met.",
        )

        with self.assertRaisesRegex(AlbertError, "Retry requires a reason"):
            workspace_module.WorkstationActionService(snapshots).submit(
                correlation_id="workstation-retry-without-reason-1",
                action_type="issue-retry",
                actor="mission-commander",
                expected_revision=1,
                target_kind="agent-session",
                target_id=first_session.session_id,
                issue_id="ISS-01",
                session_id=first_session.session_id,
            )

        self.assertNotIn("session-ISS-01-2", mission.sessions)
        repair_session = mission.launch_repair(first_session.session_id)

        with self.assertRaisesRegex(AlbertError, "Session cancellation requires a reason"):
            workspace_module.WorkstationActionService(snapshots).submit(
                correlation_id="workstation-cancel-without-reason-1",
                action_type="session-cancel",
                actor="mission-commander",
                expected_revision=1,
                target_kind="agent-session",
                target_id=repair_session.session_id,
                issue_id="ISS-01",
                session_id=repair_session.session_id,
            )

        self.assertEqual(mission.sessions[repair_session.session_id].status, "launched")

    def test_cli_submits_workstation_action_as_json(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workstation-action",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "workstation-cli-launch-1",
                    "--expected-revision",
                    "1",
                    "--action-type",
                    "issue-launch",
                    "--actor",
                    "mission-commander",
                    "--target-kind",
                    "issue-slice",
                    "--target-id",
                    "ISS-01",
                    "--issue-id",
                    "ISS-01",
                    "--allowed-path",
                    "src",
                ]
            )

        payload = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(payload["action_type"], "issue-launch")
        self.assertEqual(payload["session_id"], "session-ISS-01-1")
        self.assertEqual(payload["revision"], 2)

    def test_cli_reports_stale_workstation_action_as_structured_json(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="workstation-cli-stale-bump-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=snapshots.snapshot().conversation_scope,
                operations_view="mission-board",
            )
        )
        output = io.StringIO()
        error = io.StringIO()

        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(
                [
                    "workstation-action",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "workstation-cli-launch-stale-1",
                    "--expected-revision",
                    "1",
                    "--action-type",
                    "issue-launch",
                    "--actor",
                    "mission-commander",
                    "--target-kind",
                    "issue-slice",
                    "--target-id",
                    "ISS-01",
                    "--issue-id",
                    "ISS-01",
                ]
            )

        payload = json.loads(error.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(payload["error"]["code"], "stale-action")
        self.assertEqual(payload["error"]["expected_revision"], 1)
        self.assertEqual(payload["error"]["current_revision"], 2)
        self.assertEqual(mission.sessions, {})

    def test_workspace_queue_groups_and_filters_items_by_type_and_mission(self) -> None:
        primary = self.load_service()._primary_mission
        primary.approve_issue("ISS-01")
        background_tracker = self.root / "queue-background-tracker"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text("# Background Queue Mission\n", encoding="utf-8")
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Run background queue work."),
            encoding="utf-8",
        )
        background = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-queue",
            allow_empty_tracker=True,
        ).load()
        background.approve_issue("ISS-01")
        service = WorkspaceQueueService(
            WorkspaceSnapshotService(primary, missions=(background,))
        )
        service.propose_issue_contract_change(
            correlation_id="proposal-primary-filter-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Primary Mission proposal."],
        )
        service.propose_issue_contract_change(
            correlation_id="proposal-background-filter-1",
            expected_revision=1,
            mission_id="background-queue",
            issue_id="ISS-01",
            source="mission-board",
            acceptance_criteria=["Background Mission proposal."],
        )

        projection = service.inspect(
            item_type="issue-change-proposal",
            mission_id="background-queue",
        )

        self.assertEqual(len(projection.items), 1)
        self.assertEqual(projection.items[0].mission_id, "background-queue")
        self.assertEqual(projection.items[0].item_type, "issue-change-proposal")
        self.assertEqual(len(projection.groups), 1)
        self.assertEqual(projection.groups[0].group_id, "issue-change-proposal:background-queue")
        self.assertEqual(projection.groups[0].item_type, "issue-change-proposal")
        self.assertEqual(projection.groups[0].mission_id, "background-queue")
        self.assertEqual(projection.groups[0].item_count, 1)
        self.assertEqual(projection.groups[0].items[0].source, "mission-board")

    def test_frontier_confirmation_remains_pending_without_applying_risky_action(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")

        acknowledgement = WorkspaceQueueService(snapshots).request_frontier_confirmation(
            correlation_id="frontier-confirm-launch-boundary-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="frontier-router",
            requested_action="Expand launch boundary for deployment files",
            affected_boundary="launch-boundary",
            consequence="Approval allows the next launch to include deployment paths.",
            payload={"allowed_paths": ["deploy/", "infra/"]},
        )
        projection = WorkspaceQueueService(snapshots).inspect(item_type="frontier-confirmation")

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.item_status, "pending")
        self.assertEqual(len(projection.items), 1)
        confirmation = projection.items[0]
        self.assertEqual(confirmation.item_id, "frontier-confirmation-command-deck-ISS-01-000001")
        self.assertEqual(confirmation.status, "pending")
        self.assertEqual(confirmation.source, "frontier-router")
        self.assertEqual(confirmation.requested_action, "Expand launch boundary for deployment files")
        self.assertEqual(confirmation.affected_boundary, "launch-boundary")
        self.assertEqual(
            confirmation.consequence,
            "Approval allows the next launch to include deployment paths.",
        )
        self.assertEqual(confirmation.proposed_changes["allowed_paths"], ["deploy/", "infra/"])
        self.assertEqual(mission.issues["ISS-01"].review_state, "approved")
        self.assertEqual(mission.sessions, {})

    def test_ad_hoc_delegation_proposal_records_boundaries_without_launching(self) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        origin = history.append(
            role="user",
            content="Have a local agent update the smoke-test notes.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )

        acknowledgement = WorkspaceQueueService(snapshots).propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Smoke-test notes describe the latest verification command."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={"python3 -m unittest tests.test_workspace_snapshot": "auto-allowed"},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        projection = WorkspaceQueueService(self.load_service()).inspect(
            item_type="ad-hoc-delegation"
        )

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.item_status, "pending")
        self.assertEqual(acknowledgement.item_id, "ad-hoc-delegation-command-deck-000001")
        self.assertEqual(len(projection.items), 1)
        item = projection.items[0]
        self.assertEqual(item.item_type, "ad-hoc-delegation")
        self.assertEqual(item.status, "pending")
        self.assertEqual(item.issue_id, "ADHOC-000001")
        self.assertEqual(item.source, "agent-console")
        self.assertEqual(item.requested_action, "Approve Ad Hoc Delegation")
        self.assertEqual(item.affected_boundary, "ad-hoc-delegation")
        self.assertEqual(
            item.consequence,
            "Approval will launch ADHOC-000001 within the proposed scope, permissions, and acceptance criteria.",
        )
        self.assertEqual(item.proposed_changes["scope"]["kind"], "working-directory")
        self.assertEqual(
            item.proposed_changes["acceptance_criteria"],
            ["Smoke-test notes describe the latest verification command."],
        )
        self.assertEqual(item.proposed_changes["allowed_paths"], ["docs/smoke-tests.md"])
        self.assertEqual(item.proposed_changes["proposed_agent"], "qwen-coder-local-1")
        self.assertEqual(item.proposed_changes["originating_message_id"], origin.message_id)
        self.assertEqual(snapshots._primary_mission.sessions, {})

    def test_rejected_ad_hoc_delegation_preserves_no_launch_state(self) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Try a bounded documentation update.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-reject-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Documentation is updated."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        acknowledgement = service.decide(
            correlation_id="ad-hoc-delegation-reject-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="reject",
            reason="Not approved for this Mission.",
        )
        item = service.inspect(item_type="ad-hoc-delegation").items[0]

        self.assertEqual(acknowledgement.item_status, "rejected")
        self.assertEqual(item.status, "rejected")
        self.assertEqual(snapshots._primary_mission.sessions, {})

    def test_approved_ad_hoc_delegation_launches_bounded_session_without_issue_slice(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Run a bounded docs update through a local agent.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-approve-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Docs mention the focused unit test command."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={"python3 -m unittest tests.test_workspace_snapshot": "auto-allowed"},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        acknowledgement = service.decide(
            correlation_id="ad-hoc-delegation-approve-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approved for bounded docs update.",
        )
        session = mission.sessions["session-ADHOC-000001-1"]

        self.assertEqual(acknowledgement.item_status, "approved")
        self.assertNotIn("ADHOC-000001", mission.issues)
        self.assertEqual(session.issue_id, "ADHOC-000001")
        self.assertEqual(session.task_packet["work_kind"], "ad-hoc-delegation")
        self.assertEqual(session.task_packet["originating_message_id"], origin.message_id)
        self.assertEqual(session.task_packet["conversation_scope"]["kind"], "working-directory")
        self.assertEqual(session.task_packet["goal"], "Ad Hoc Delegation ADHOC-000001")
        self.assertEqual(session.task_packet["acceptance_criteria"], ["Docs mention the focused unit test command."])
        self.assertEqual(session.task_packet["allowed_paths"], ["docs/smoke-tests.md"])
        self.assertEqual(
            session.task_packet["command_policy"],
            {"python3 -m unittest tests.test_workspace_snapshot": "auto-allowed"},
        )
        self.assertEqual(session.assigned_agent, "qwen-coder-local-1")

    def test_ad_hoc_delegation_denies_launch_when_command_policy_is_not_auto_allowed(self) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Prepare a deploy note, but do not run deploy commands.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-deny-command-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Deploy note is drafted without pushing changes."],
            allowed_paths=["docs/deploy-note.md"],
            command_policy={"git push origin main": "frontier-approvable"},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        with self.assertRaisesRegex(AlbertError, "requires auto-allowed command policy"):
            service.decide(
                correlation_id="ad-hoc-delegation-deny-command-decision-1",
                expected_revision=proposal.revision,
                item_id=proposal.item_id,
                decision="approve",
                reason="Attempted approval outside accepted command boundary.",
            )

        item = service.inspect(item_type="ad-hoc-delegation").items[0]
        self.assertEqual(item.status, "pending")
        self.assertEqual(snapshots._primary_mission.sessions, {})

    def test_ad_hoc_delegation_session_summary_exposes_status_and_model_provenance(self) -> None:
        (self.target_repo / ".albert").mkdir()
        (self.target_repo / ".albert" / "agents.json").write_text(
            json.dumps(
                {
                    "agents": [
                        {
                            "id": "qwen-coder-local-1",
                            "role": "local-agent",
                            "provider": "ollama",
                            "runner": "ollama",
                            "model": "qwen2.5-coder:14b",
                            "availability": "available",
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Launch a bounded docs-only session.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-session-summary-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Docs-only session is visible in workspace summary."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        service.decide(
            correlation_id="ad-hoc-delegation-session-summary-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approved for workspace session visibility.",
        )
        session_summary = snapshots.snapshot().missions[0].sessions[0]

        self.assertEqual(session_summary.session_id, "session-ADHOC-000001-1")
        self.assertEqual(session_summary.issue_id, "ADHOC-000001")
        self.assertEqual(session_summary.status, "launched")
        self.assertEqual(session_summary.role, "local-agent")
        self.assertEqual(session_summary.provider, "ollama")
        self.assertEqual(session_summary.model, "qwen2.5-coder:14b")

    def test_review_workspace_accepts_ad_hoc_delegation_evidence_without_issue_slice(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Run a bounded docs update and submit evidence.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        service = WorkspaceQueueService(snapshots)
        proposal = service.propose_ad_hoc_delegation(
            correlation_id="ad-hoc-delegation-review-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Evidence package proves the docs update."],
            allowed_paths=["docs/smoke-tests.md"],
            command_policy={"python3 -m unittest tests.test_workspace_snapshot": "auto-allowed"},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        service.decide(
            correlation_id="ad-hoc-delegation-review-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approved for evidence review.",
        )
        mission.record_evidence(
            "session-ADHOC-000001-1",
            EvidencePackage(
                changed_files=["docs/smoke-tests.md"],
                diff_summary="Updated smoke-test notes.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="Document ad hoc delegation evidence handling.",
                artifact_links=["app-local://evidence/session-ADHOC-000001-1"],
            ),
        )
        review_service = ReviewWorkspaceService(snapshots)
        review_item = review_service.inspect().items[0]

        acknowledgement = review_service.decide(
            correlation_id="ad-hoc-delegation-review-accept-1",
            expected_revision=snapshots.snapshot().revision,
            session_id="session-ADHOC-000001-1",
            decision="accept",
            reason="Evidence accepted.",
        )

        self.assertEqual(review_item.issue_id, "ADHOC-000001")
        self.assertEqual(review_item.evidence_complete, True)
        self.assertEqual(acknowledgement.review_outcome, "Approved")
        self.assertEqual(acknowledgement.next_action, "complete")
        self.assertEqual(acknowledgement.issue_lifecycle, "Complete")
        self.assertNotIn("ADHOC-000001", mission.issues)
        self.assertEqual(mission.sessions["session-ADHOC-000001-1"].status, "reviewed")
        self.assertEqual(mission.sessions["session-ADHOC-000001-1"].cleanup_eligible, True)

    def test_mission_draft_records_selected_excluded_and_new_work_without_accepting_state(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        initial_issue_ids = tuple(mission.issues)
        initial_session_ids = tuple(mission.sessions)
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Turn the useful ad hoc work into a proper Mission Draft.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        first = queue.propose_ad_hoc_delegation(
            correlation_id="mission-draft-ad-hoc-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Summarize useful command deck follow-up work."],
            allowed_paths=["docs/command-deck.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        queue.propose_ad_hoc_delegation(
            correlation_id="mission-draft-ad-hoc-2",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Investigate an unrelated packaging idea."],
            allowed_paths=["docs/packaging.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        acknowledgement = MissionDraftService(snapshots).create_draft(
            correlation_id="mission-draft-create-1",
            expected_revision=1,
            proposed_goal="Create a focused Command Deck follow-up mission.",
            selected_ad_hoc_ids=["ADHOC-000001"],
            excluded_ad_hoc_ids=["ADHOC-000002"],
            new_work_items=["Add a Mission Draft confirmation path."],
            dependencies=["Issue 10 ad hoc delegation approvals stay authoritative."],
            unresolved_decisions=["Confirm whether packaging belongs in a later mission."],
        )
        reloaded = MissionDraftService(self.load_service()).inspect()

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.draft_id, "mission-draft-command-deck-000001")
        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(first.item_id, "ad-hoc-delegation-command-deck-000001")
        self.assertEqual(len(reloaded.drafts), 1)
        draft = reloaded.drafts[0]
        self.assertEqual(draft.draft_id, "mission-draft-command-deck-000001")
        self.assertEqual(draft.status, "draft")
        self.assertEqual(draft.mission_id, "command-deck")
        self.assertEqual(draft.proposed_goal, "Create a focused Command Deck follow-up mission.")
        self.assertEqual([item.work_id for item in draft.included_ad_hoc_work], ["ADHOC-000001"])
        self.assertEqual(
            draft.included_ad_hoc_work[0].acceptance_criteria,
            ("Summarize useful command deck follow-up work.",),
        )
        self.assertEqual(draft.excluded_ad_hoc_work_ids, ("ADHOC-000002",))
        self.assertEqual(draft.new_work_items, ("Add a Mission Draft confirmation path.",))
        self.assertEqual(
            draft.dependencies,
            ("Issue 10 ad hoc delegation approvals stay authoritative.",),
        )
        self.assertEqual(
            draft.unresolved_decisions,
            ("Confirm whether packaging belongs in a later mission.",),
        )
        self.assertEqual(tuple(mission.issues), initial_issue_ids)
        self.assertEqual(tuple(mission.sessions), initial_session_ids)
        self.assertNotIn("ADHOC-000001", mission.issues)
        self.assertEqual(snapshots.snapshot().revision, 1)

    def test_mission_draft_revision_keeps_unselected_ad_hoc_work_outside(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Revise the Mission Draft after more ad hoc work appears.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        for correlation_id, criteria, path in [
            ("mission-draft-revision-ad-hoc-1", "Keep this work in the draft.", "docs/included.md"),
            ("mission-draft-revision-ad-hoc-2", "Keep this work excluded.", "docs/excluded.md"),
            ("mission-draft-revision-ad-hoc-3", "This later work is unrelated.", "docs/unselected.md"),
        ]:
            queue.propose_ad_hoc_delegation(
                correlation_id=correlation_id,
                expected_revision=1,
                source="agent-console",
                scope=snapshots.snapshot().conversation_scope,
                acceptance_criteria=[criteria],
                allowed_paths=[path],
                command_policy={},
                proposed_agent="qwen-coder-local-1",
                originating_message_id=origin.message_id,
            )
        service = MissionDraftService(snapshots)
        created = service.create_draft(
            correlation_id="mission-draft-revision-create-1",
            expected_revision=1,
            proposed_goal="Create a focused Command Deck follow-up mission.",
            selected_ad_hoc_ids=["ADHOC-000001"],
            excluded_ad_hoc_ids=["ADHOC-000002"],
            new_work_items=["Add a confirmation path."],
            dependencies=[],
            unresolved_decisions=[],
        )

        acknowledgement = service.update_draft(
            correlation_id="mission-draft-revision-update-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            proposed_goal="Create a focused Command Deck confirmation mission.",
            selected_ad_hoc_ids=["ADHOC-000001"],
            excluded_ad_hoc_ids=["ADHOC-000002"],
            new_work_items=["Add a confirmation path.", "Add rejection handling."],
            dependencies=["Issue 10 remains the source for ad hoc delegation records."],
            unresolved_decisions=[],
        )
        reloaded = MissionDraftService(self.load_service()).inspect().drafts[0]

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.revision, created.revision + 1)
        self.assertEqual(reloaded.proposed_goal, "Create a focused Command Deck confirmation mission.")
        self.assertEqual([item.work_id for item in reloaded.included_ad_hoc_work], ["ADHOC-000001"])
        self.assertEqual(reloaded.excluded_ad_hoc_work_ids, ("ADHOC-000002",))
        self.assertNotIn("ADHOC-000003", [item.work_id for item in reloaded.included_ad_hoc_work])
        self.assertNotIn("ADHOC-000003", reloaded.excluded_ad_hoc_work_ids)
        self.assertEqual(
            reloaded.new_work_items,
            ("Add a confirmation path.", "Add rejection handling."),
        )
        self.assertEqual(tuple(mission.sessions), ())
        self.assertNotIn("ADHOC-000003", mission.issues)

    def test_mission_draft_confirmation_creates_accepted_issue_slice(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Confirm the draft as accepted mission work.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        queue.propose_ad_hoc_delegation(
            correlation_id="mission-draft-confirm-ad-hoc-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Ad hoc evidence informs the confirmed mission."],
            allowed_paths=["docs/confirmed.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        service = MissionDraftService(snapshots)
        created = service.create_draft(
            correlation_id="mission-draft-confirm-create-1",
            expected_revision=1,
            proposed_goal="Create the confirmed Command Deck mission scope.",
            selected_ad_hoc_ids=["ADHOC-000001"],
            excluded_ad_hoc_ids=[],
            new_work_items=["Add a confirmation path."],
            dependencies=["Draft review is complete."],
            unresolved_decisions=["Choose the final UI placement."],
        )

        acknowledgement = service.confirm_draft(
            correlation_id="mission-draft-confirm-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            reason="Mission Commander confirmed the draft.",
        )
        reloaded_service = self.load_service()
        reloaded_draft = MissionDraftService(reloaded_service).inspect().drafts[0]
        accepted_issue = reloaded_service._primary_mission.issues["ISS-02"]

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.draft_status, "confirmed")
        self.assertEqual(acknowledgement.accepted_issue_id, "ISS-02")
        self.assertEqual(reloaded_draft.status, "confirmed")
        self.assertEqual(accepted_issue.title, "Create The Confirmed Command Deck Mission Scope")
        self.assertEqual(
            accepted_issue.what_to_build,
            "Create the confirmed Command Deck mission scope.",
        )
        self.assertIn("Include Ad Hoc Delegation ADHOC-000001.", accepted_issue.acceptance_criteria)
        self.assertIn("New work: Add a confirmation path.", accepted_issue.acceptance_criteria)
        self.assertIn("Dependency: Draft review is complete.", accepted_issue.acceptance_criteria)
        self.assertIn("Resolve decision: Choose the final UI placement.", accepted_issue.acceptance_criteria)
        self.assertEqual(tuple(mission.sessions), ())
        self.assertNotIn("ADHOC-000001", mission.issues)

    def test_mission_draft_abandonment_preserves_existing_missions(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        initial_issue_ids = tuple(mission.issues)
        service = MissionDraftService(snapshots)
        created = service.create_draft(
            correlation_id="mission-draft-abandon-create-1",
            expected_revision=1,
            proposed_goal="Create a mission that should not be accepted.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Explore work that will be abandoned."],
            dependencies=[],
            unresolved_decisions=[],
        )

        acknowledgement = service.abandon_draft(
            correlation_id="mission-draft-abandon-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            reason="Mission Commander rejected this draft.",
        )
        reloaded_service = self.load_service()
        reloaded_draft = MissionDraftService(reloaded_service).inspect().drafts[0]

        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.draft_status, "abandoned")
        self.assertEqual(reloaded_draft.status, "abandoned")
        self.assertEqual(tuple(mission.issues), initial_issue_ids)
        self.assertEqual(tuple(reloaded_service._primary_mission.issues), initial_issue_ids)
        self.assertEqual(tuple(mission.sessions), ())

    def test_stale_mission_draft_confirmation_preserves_mission_state(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        initial_issue_ids = tuple(mission.issues)
        service = MissionDraftService(snapshots)
        created = service.create_draft(
            correlation_id="mission-draft-stale-create-1",
            expected_revision=1,
            proposed_goal="Create a mission only from the fresh draft revision.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Original draft work."],
            dependencies=[],
            unresolved_decisions=[],
        )
        service.update_draft(
            correlation_id="mission-draft-stale-update-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            proposed_goal="Create a mission only from the updated draft revision.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Updated draft work."],
            dependencies=[],
            unresolved_decisions=[],
        )

        with self.assertRaises(WorkspaceStaleActionError):
            service.confirm_draft(
                correlation_id="mission-draft-stale-confirm-1",
                expected_revision=created.revision,
                draft_id=created.draft_id,
                reason="This confirmation used a stale draft revision.",
            )
        reloaded_service = self.load_service()

        self.assertEqual(tuple(mission.issues), initial_issue_ids)
        self.assertEqual(tuple(reloaded_service._primary_mission.issues), initial_issue_ids)
        self.assertEqual(MissionDraftService(reloaded_service).inspect().drafts[0].status, "draft")

    def test_cli_creates_inspects_and_confirms_mission_draft_across_restart(self) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Create a draft through the CLI.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        WorkspaceQueueService(snapshots).propose_ad_hoc_delegation(
            correlation_id="mission-draft-cli-ad-hoc-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["CLI-selected ad hoc work is represented."],
            allowed_paths=["docs/cli-draft.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        create_output = io.StringIO()
        with redirect_stdout(create_output):
            create_exit = main(
                [
                    "mission-draft-create",
                    *common,
                    "--correlation-id",
                    "mission-draft-cli-create-1",
                    "--expected-revision",
                    "1",
                    "--proposed-goal",
                    "Create a CLI confirmed mission draft.",
                    "--selected-ad-hoc-id",
                    "ADHOC-000001",
                    "--new-work-item",
                    "Expose Mission Drafts through the CLI.",
                ]
            )
        created = json.loads(create_output.getvalue())
        inspect_output = io.StringIO()
        with redirect_stdout(inspect_output):
            inspect_exit = main(["mission-drafts", *common])
        inspected = json.loads(inspect_output.getvalue())
        confirm_output = io.StringIO()
        with redirect_stdout(confirm_output):
            confirm_exit = main(
                [
                    "mission-draft-confirm",
                    *common,
                    "--correlation-id",
                    "mission-draft-cli-confirm-1",
                    "--expected-draft-revision",
                    str(created["revision"]),
                    "--draft-id",
                    created["draft_id"],
                    "--reason",
                    "Confirmed through CLI.",
                ]
            )
        confirmed = json.loads(confirm_output.getvalue())
        reloaded = self.load_service()

        self.assertEqual(create_exit, 0)
        self.assertEqual(inspect_exit, 0)
        self.assertEqual(confirm_exit, 0)
        self.assertEqual(created["draft_status"], "draft")
        self.assertEqual(inspected["drafts"][0]["included_ad_hoc_work"][0]["work_id"], "ADHOC-000001")
        self.assertEqual(confirmed["draft_status"], "confirmed")
        self.assertEqual(confirmed["accepted_issue_id"], "ISS-02")
        self.assertIn("ISS-02", reloaded._primary_mission.issues)

    def test_workspace_snapshot_attention_links_to_pending_queue_items(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        proposal = WorkspaceQueueService(snapshots).propose_issue_contract_change(
            correlation_id="proposal-attention-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Attention links to the queue item."],
        )

        summary = snapshots.snapshot().missions[0]

        self.assertEqual(summary.attention[0].attention_id, proposal.item_id)
        self.assertEqual(summary.attention[0].kind, "issue-change-proposal")
        self.assertEqual(summary.attention[0].label, "ISS-01 Issue Change Proposal pending")
        self.assertEqual(
            summary.attention[0].queue_link,
            f"workspace-queue#{proposal.item_id}",
        )

    def test_cli_returns_workspace_queue_projection_and_acknowledges_decision_as_json(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        accepted_criteria = list(mission.issues["ISS-01"].acceptance_criteria)
        proposal = WorkspaceQueueService(snapshots).propose_issue_contract_change(
            correlation_id="proposal-cli-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["CLI rejected proposal."],
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        projection_output = io.StringIO()
        with redirect_stdout(projection_output):
            projection_exit = main(
                [
                    "workspace-queue",
                    *common,
                    "--item-type",
                    "issue-change-proposal",
                    "--queue-mission-id",
                    "command-deck",
                ]
            )
        decision_output = io.StringIO()
        with redirect_stdout(decision_output):
            decision_exit = main(
                [
                    "workspace-queue-decision",
                    *common,
                    "--correlation-id",
                    "proposal-cli-reject-1",
                    "--expected-queue-revision",
                    str(proposal.revision),
                    "--item-id",
                    proposal.item_id,
                    "--decision",
                    "reject",
                    "--reason",
                    "Rejected from CLI.",
                ]
            )

        projection = json.loads(projection_output.getvalue())
        acknowledgement = json.loads(decision_output.getvalue())
        self.assertEqual(projection_exit, 0)
        self.assertEqual(projection["items"][0]["item_id"], proposal.item_id)
        self.assertEqual(projection["groups"][0]["group_id"], "issue-change-proposal:command-deck")
        self.assertEqual(decision_exit, 0)
        self.assertEqual(acknowledgement["item_status"], "rejected")
        self.assertEqual(mission.issues["ISS-01"].acceptance_criteria, accepted_criteria)

    def test_cli_proposes_ad_hoc_delegation_and_returns_queue_projection_as_json(self) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Have a local agent refresh smoke-test notes.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        acknowledgement_output = io.StringIO()
        with redirect_stdout(acknowledgement_output):
            acknowledgement_exit = main(
                [
                    "ad-hoc-delegation-proposal",
                    *common,
                    "--correlation-id",
                    "ad-hoc-cli-proposal-1",
                    "--expected-revision",
                    "1",
                    "--source",
                    "agent-console",
                    "--scope-kind",
                    "working-directory",
                    "--scope-target",
                    str(self.target_repo),
                    "--scope-label",
                    "target",
                    "--acceptance-criterion",
                    "Smoke-test notes mention the focused unit command.",
                    "--allowed-path",
                    "docs/smoke-tests.md",
                    "--command-policy",
                    "python3 -m unittest tests.test_workspace_snapshot=auto-allowed",
                    "--proposed-agent",
                    "qwen-coder-local-1",
                    "--originating-message-id",
                    origin.message_id,
                ]
            )
        projection_output = io.StringIO()
        with redirect_stdout(projection_output):
            projection_exit = main(
                [
                    "workspace-queue",
                    *common,
                    "--item-type",
                    "ad-hoc-delegation",
                ]
            )

        acknowledgement = json.loads(acknowledgement_output.getvalue())
        projection = json.loads(projection_output.getvalue())
        self.assertEqual(acknowledgement_exit, 0)
        self.assertEqual(acknowledgement["item_status"], "pending")
        self.assertEqual(acknowledgement["item_id"], "ad-hoc-delegation-command-deck-000001")
        self.assertEqual(projection_exit, 0)
        self.assertEqual(projection["items"][0]["item_type"], "ad-hoc-delegation")
        self.assertEqual(projection["items"][0]["issue_id"], "ADHOC-000001")
        self.assertEqual(
            projection["items"][0]["proposed_changes"]["originating_message_id"],
            origin.message_id,
        )

    def test_review_workspace_accepts_valid_evidence_as_complete_pr_ready_not_merged(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added the review workspace decision path.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="No glossary changes.",
            ),
        )

        acknowledgement = ReviewWorkspaceService(WorkspaceSnapshotService(mission)).decide(
            correlation_id="review-accept-1",
            expected_revision=1,
            session_id=session.session_id,
            decision="accept",
            reason="Evidence satisfies the Issue Slice.",
        )

        self.assertEqual(acknowledgement.correlation_id, "review-accept-1")
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(acknowledgement.review_outcome, "Approved")
        self.assertEqual(acknowledgement.next_action, "prepare-pr")
        self.assertEqual(acknowledgement.issue_lifecycle, "Complete")
        self.assertNotEqual(acknowledgement.issue_lifecycle, "Merged")
        self.assertEqual(mission.issues["ISS-01"].review_state, "pr-ready")

    def test_review_workspace_rejects_acceptance_when_required_evidence_is_missing(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with self.assertRaisesRegex(
            EvidenceValidationError,
            "changed_files, diff_summary, commands_run, test_results, known_risks, proposed_context_updates",
        ):
            ReviewWorkspaceService(WorkspaceSnapshotService(mission)).decide(
                correlation_id="review-accept-incomplete-1",
                expected_revision=1,
                session_id=session.session_id,
                decision="accept",
                reason="Looks fine.",
            )

        self.assertEqual(mission.issues["ISS-01"].review_state, "approved")
        self.assertEqual(mission.sessions[session.session_id].status, "launched")

    def test_review_workspace_routes_repair_human_escalation_and_stale_decisions(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        repair_session = mission.launch_issue("ISS-01")
        service = ReviewWorkspaceService(WorkspaceSnapshotService(mission))

        with self.assertRaisesRegex(AlbertError, "Repair review decisions require a reason"):
            service.decide(
                correlation_id="repair-without-reason",
                expected_revision=1,
                session_id=repair_session.session_id,
                decision="repair",
            )

        repair = service.decide(
            correlation_id="repair-with-reason",
            expected_revision=1,
            session_id=repair_session.session_id,
            decision="repair",
            reason="Acceptance criteria are not met.",
        )

        self.assertEqual(repair.review_outcome, "Needs repair")
        self.assertEqual(repair.next_action, "same-local-agent-repair")
        self.assertEqual(mission.issues["ISS-01"].review_state, "needs-repair")
        human_session = mission.launch_repair(repair_session.session_id)
        human = ReviewWorkspaceService(WorkspaceSnapshotService(mission)).decide(
            correlation_id="human-escalation",
            expected_revision=2,
            session_id=human_session.session_id,
            decision="escalate-human",
            reason="Human operator must inspect local-only evidence.",
        )

        self.assertEqual(human.review_outcome, "Needs human review")
        self.assertEqual(human.next_action, "user-review")
        self.assertEqual(mission.issues["ISS-01"].review_state, "needs-human-review")
        with self.assertRaises(WorkspaceStaleActionError):
            ReviewWorkspaceService(WorkspaceSnapshotService(mission)).decide(
                correlation_id="stale-review",
                expected_revision=1,
                session_id=human_session.session_id,
                decision="escalate-human",
                reason="Stale.",
            )

    def test_cli_returns_review_workspace_projection_as_json(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added JSON review projection.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="No glossary changes.",
            ),
        )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "review-workspace",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                ]
            )

        projection = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(projection["schema_version"], 1)
        self.assertEqual(projection["items"][0]["session_id"], session.session_id)
        self.assertEqual(projection["items"][0]["evidence"]["diff_summary"], "Added JSON review projection.")

    def test_cli_acknowledges_review_workspace_decision_as_json(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added JSON review decision.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="No glossary changes.",
            ),
        )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "review-decision",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "review-accept-cli-1",
                    "--expected-revision",
                    "1",
                    "--session-id",
                    session.session_id,
                    "--decision",
                    "accept",
                    "--reason",
                    "Evidence satisfies the Issue Slice.",
                ]
            )

        acknowledgement = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(acknowledgement["correlation_id"], "review-accept-cli-1")
        self.assertEqual(acknowledgement["review_outcome"], "Approved")
        self.assertEqual(acknowledgement["next_action"], "prepare-pr")
        self.assertEqual(acknowledgement["issue_lifecycle"], "Complete")
        self.assertEqual(self.load_service()._primary_mission.issues["ISS-01"].review_state, "pr-ready")

    def test_cli_returns_filtered_activity_journal_as_json(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added CLI Activity Journal projection.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="Document Activity Journal CLI.",
                artifact_links=["app-local://evidence/session-ISS-01-1"],
            ),
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]

        with redirect_stdout(io.StringIO()):
            decision_exit = main(
                [
                    "review-decision",
                    *common,
                    "--correlation-id",
                    "review-activity-cli-1",
                    "--expected-revision",
                    "1",
                    "--session-id",
                    session.session_id,
                    "--decision",
                    "accept",
                    "--reason",
                    "Evidence is complete.",
                ]
            )
        output = io.StringIO()

        with redirect_stdout(output):
            journal_exit = main(
                [
                    "activity-journal",
                    *common,
                    "--activity-mission-id",
                    "command-deck",
                    "--actor",
                    "mission-commander",
                    "--action-type",
                    "review-decision",
                    "--search",
                    "evidence",
                    "--started-at",
                    "2026-01-01T00:00:00Z",
                ]
            )

        projection = json.loads(output.getvalue())
        entry = projection["entries"][0]
        self.assertEqual(decision_exit, 0)
        self.assertEqual(journal_exit, 0)
        self.assertEqual(projection["schema_version"], 1)
        self.assertEqual(projection["revision"], 2)
        self.assertEqual(len(projection["entries"]), 1)
        self.assertEqual(entry["correlation_id"], "review-activity-cli-1")
        self.assertEqual(entry["action_type"], "review-decision")
        self.assertEqual(entry["evidence_links"], ["app-local://evidence/session-ISS-01-1"])
        self.assertEqual(
            [entity["entity_type"] for entity in entry["affected_entities"]],
            ["mission", "issue-slice", "local-agent-session", "evidence-package"],
        )

    def test_cli_reports_incomplete_review_decision_as_structured_json(self) -> None:
        mission = self.load_service()._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        output = io.StringIO()
        error = io.StringIO()

        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(
                [
                    "review-decision",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "review-accept-incomplete-cli-1",
                    "--expected-revision",
                    "1",
                    "--session-id",
                    session.session_id,
                    "--decision",
                    "accept",
                    "--reason",
                    "Looks acceptable.",
                ]
            )

        payload = json.loads(error.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(payload["error"]["code"], "evidence-incomplete")
        self.assertEqual(payload["error"]["recoverable"], True)
        self.assertIn("changed_files", payload["error"]["message"])
        self.assertEqual(self.load_service()._primary_mission.issues["ISS-01"].review_state, "approved")

    def test_stale_semantic_action_is_rejected_without_changing_accepted_state(self) -> None:
        sync = WorkspaceSyncService(self.load_service())
        action = WorkspaceAction(
            correlation_id="view-stale-1",
            expected_revision=0,
            active_mission_id="command-deck",
            conversation_scope=ConversationScope(
                kind="working-directory",
                target_id=str(self.target_repo),
                label="target",
            ),
            operations_view="activity",
        )

        with self.assertRaises(WorkspaceStaleActionError) as raised:
            sync.submit_action(action)

        unchanged = self.load_service().snapshot()
        self.assertEqual(raised.exception.current_revision, 1)
        self.assertEqual(unchanged.revision, 1)
        self.assertEqual(unchanged.operations_view, "mission-board")

    def test_accepted_actions_produce_ordered_batch_after_known_revision(self) -> None:
        sync = WorkspaceSyncService(self.load_service())
        scope = ConversationScope(
            kind="working-directory",
            target_id=str(self.target_repo),
            label="target",
        )

        first = sync.submit_action(
            WorkspaceAction(
                correlation_id="view-review-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="review-workspace",
            )
        )
        second = sync.submit_action(
            WorkspaceAction(
                correlation_id="view-activity-2",
                expected_revision=2,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="activity",
            )
        )

        batch = sync.updates_after(1)

        self.assertEqual(first.revision, 2)
        self.assertEqual(second.revision, 3)
        self.assertEqual([event.revision for event in batch.events], [2, 3])
        self.assertEqual(
            [event.correlation_id for event in batch.events],
            ["view-review-1", "view-activity-2"],
        )
        self.assertEqual(len({event.event_id for event in batch.events}), 2)
        self.assertEqual(batch.current_revision, 3)
        self.assertEqual(self.load_service().snapshot().operations_view, "activity")

    def test_activity_journal_records_acknowledged_workspace_actions_only(self) -> None:
        snapshots = self.load_service()
        sync = WorkspaceSyncService(snapshots)
        scope = ConversationScope(
            kind="working-directory",
            target_id=str(self.target_repo),
            label="target",
        )

        acknowledgement = sync.submit_action(
            WorkspaceAction(
                correlation_id="view-activity-journal-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="activity",
            )
        )
        with self.assertRaises(WorkspaceStaleActionError):
            sync.submit_action(
                WorkspaceAction(
                    correlation_id="view-stale-journal-2",
                    expected_revision=1,
                    active_mission_id="command-deck",
                    conversation_scope=scope,
                    operations_view="mission-board",
                )
            )

        reloaded = self.load_service()
        projection = ActivityJournalService(reloaded).inspect()
        session_id = reloaded.snapshot().workspace_session.id

        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(projection.revision, 1)
        self.assertEqual(len(projection.entries), 1)
        entry = projection.entries[0]
        self.assertEqual(entry.entry_id, "activity-000001")
        self.assertEqual(entry.sequence, 1)
        self.assertTrue(entry.recorded_at.endswith("Z"))
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.action_type, "operations-view-selected")
        self.assertEqual(entry.correlation_id, "view-activity-journal-1")
        self.assertIn("Activity", entry.summary)
        self.assertEqual(entry.evidence_links, ())
        self.assertEqual(
            [(entity.entity_type, entity.entity_id) for entity in entry.affected_entities],
            [("workspace-session", session_id), ("mission", "command-deck")],
        )

    def test_activity_journal_searches_and_filters_chronological_entries(self) -> None:
        snapshots = self.load_service()
        sync = WorkspaceSyncService(snapshots)
        scope = ConversationScope(
            kind="working-directory",
            target_id=str(self.target_repo),
            label="target",
        )
        sync.submit_action(
            WorkspaceAction(
                correlation_id="view-review-journal-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="review-workspace",
            )
        )
        sync.submit_action(
            WorkspaceAction(
                correlation_id="view-queue-journal-2",
                expected_revision=2,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="workspace-queue",
            )
        )

        journal = ActivityJournalService(self.load_service())

        self.assertEqual(
            [entry.correlation_id for entry in journal.inspect().entries],
            ["view-review-journal-1", "view-queue-journal-2"],
        )
        self.assertEqual(
            [entry.correlation_id for entry in journal.inspect(search="queue").entries],
            ["view-queue-journal-2"],
        )
        self.assertEqual(
            [
                entry.correlation_id
                for entry in journal.inspect(
                    mission_id="command-deck",
                    actor="mission-commander",
                    action_type="operations-view-selected",
                ).entries
            ],
            ["view-review-journal-1", "view-queue-journal-2"],
        )
        self.assertEqual(journal.inspect(mission_id="unknown-mission").entries, ())

    def test_activity_journal_filters_by_recorded_time_window(self) -> None:
        snapshots = self.load_service()
        sync = WorkspaceSyncService(snapshots)
        scope = ConversationScope(
            kind="working-directory",
            target_id=str(self.target_repo),
            label="target",
        )

        with patch("albert_mvp.workspace.datetime") as clock:
            clock.now.side_effect = [
                datetime(2026, 6, 26, 10, 0, tzinfo=timezone.utc),
                datetime(2026, 6, 26, 10, 10, tzinfo=timezone.utc),
            ]
            sync.submit_action(
                WorkspaceAction(
                    correlation_id="view-review-time-1",
                    expected_revision=1,
                    active_mission_id="command-deck",
                    conversation_scope=scope,
                    operations_view="review-workspace",
                )
            )
            sync.submit_action(
                WorkspaceAction(
                    correlation_id="view-activity-time-2",
                    expected_revision=2,
                    active_mission_id="command-deck",
                    conversation_scope=scope,
                    operations_view="activity",
                )
            )

        journal = ActivityJournalService(self.load_service())

        self.assertEqual(
            [
                entry.correlation_id
                for entry in journal.inspect(started_at="2026-06-26T10:05:00Z").entries
            ],
            ["view-activity-time-2"],
        )
        self.assertEqual(
            [
                entry.correlation_id
                for entry in journal.inspect(ended_at="2026-06-26T10:05:00Z").entries
            ],
            ["view-review-time-1"],
        )
        self.assertEqual(
            [
                entry.correlation_id
                for entry in journal.inspect(
                    started_at="2026-06-26T09:55:00Z",
                    ended_at="2026-06-26T10:10:00Z",
                ).entries
            ],
            ["view-review-time-1", "view-activity-time-2"],
        )

    def test_activity_journal_records_review_decision_with_links(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")
        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/app.py"],
                diff_summary="Added the Activity Journal review decision link.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Focused workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="Document Activity Journal review links.",
                artifact_links=["app-local://evidence/session-ISS-01-1"],
            ),
        )

        acknowledgement = ReviewWorkspaceService(snapshots).decide(
            correlation_id="review-journal-accept-1",
            expected_revision=1,
            session_id=session.session_id,
            decision="accept",
            reason="Evidence is complete and linked.",
        )

        projection = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(
            [entry.action_type for entry in projection.entries],
            ["evidence-package-submitted", "review-decision"],
        )
        entry = projection.entries[1]
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.action_type, "review-decision")
        self.assertEqual(entry.correlation_id, "review-journal-accept-1")
        self.assertIn("Approved", entry.summary)
        self.assertEqual(entry.evidence_links, ("app-local://evidence/session-ISS-01-1",))
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "issue-slice",
                    "ISS-01",
                    "app-local://missions/command-deck/issues/ISS-01",
                ),
                (
                    "local-agent-session",
                    session.session_id,
                    f"app-local://missions/command-deck/sessions/{session.session_id}",
                ),
                (
                    "evidence-package",
                    session.session_id,
                    "app-local://evidence/session-ISS-01-1",
                ),
            ],
        )

    def test_activity_journal_records_acknowledged_workspace_queue_decision_with_links(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_issue_contract_change(
            correlation_id="queue-journal-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Record the acknowledged queue decision."],
        )

        acknowledgement = queue.decide(
            correlation_id="queue-journal-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Accepted by the Mission Commander.",
        )

        projection = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(len(projection.entries), 1)
        entry = projection.entries[0]
        self.assertTrue(entry.recorded_at.endswith("Z"))
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.action_type, "workspace-queue-decision")
        self.assertEqual(entry.correlation_id, "queue-journal-decision-1")
        self.assertIn("Approved", entry.summary)
        self.assertIn(proposal.item_id, entry.summary)
        self.assertEqual(entry.evidence_links, ())
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "issue-slice",
                    "ISS-01",
                    "app-local://missions/command-deck/issues/ISS-01",
                ),
                (
                    "workspace-queue-item",
                    proposal.item_id,
                    f"workspace-queue#{proposal.item_id}",
                ),
                (
                    "workspace-queue-decision",
                    "queue-journal-decision-1",
                    f"workspace-queue#{proposal.item_id}",
                ),
            ],
        )

    def test_activity_journal_records_acknowledged_mission_draft_creation_with_links(
        self,
    ) -> None:
        snapshots = self.load_service()

        acknowledgement = MissionDraftService(snapshots).create_draft(
            correlation_id="mission-draft-journal-create-1",
            expected_revision=1,
            proposed_goal="Create linked Activity Journal coverage.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Record Mission Draft creation."],
            dependencies=[],
            unresolved_decisions=[],
        )

        projection = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(len(projection.entries), 1)
        entry = projection.entries[0]
        self.assertTrue(entry.recorded_at.endswith("Z"))
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.action_type, "mission-draft-created")
        self.assertEqual(entry.correlation_id, "mission-draft-journal-create-1")
        self.assertIn(acknowledgement.draft_id, entry.summary)
        self.assertEqual(entry.evidence_links, ())
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "mission-draft",
                    acknowledgement.draft_id,
                    f"workspace-queue#{acknowledgement.draft_id}",
                ),
            ],
        )

    def test_activity_journal_records_confirmed_mission_draft_and_accepted_issue_link(
        self,
    ) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-journal-confirm-create-1",
            expected_revision=1,
            proposed_goal="Confirm linked Activity Journal coverage.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Record Mission Draft confirmation."],
            dependencies=[],
            unresolved_decisions=[],
        )

        acknowledgement = drafts.confirm_draft(
            correlation_id="mission-draft-journal-confirm-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            reason="The Mission Commander accepted this scope.",
        )

        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(
            [entry.action_type for entry in entries],
            ["mission-draft-created", "mission-draft-confirmed"],
        )
        entry = entries[1]
        self.assertEqual(entry.actor, "mission-commander")
        self.assertEqual(entry.correlation_id, "mission-draft-journal-confirm-1")
        self.assertIn(acknowledgement.accepted_issue_id, entry.summary)
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "mission-draft",
                    created.draft_id,
                    f"workspace-queue#{created.draft_id}",
                ),
                (
                    "issue-slice",
                    acknowledgement.accepted_issue_id,
                    (
                        "app-local://missions/command-deck/issues/"
                        f"{acknowledgement.accepted_issue_id}"
                    ),
                ),
            ],
        )

    def test_activity_journal_records_acknowledged_mission_draft_revision(self) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-journal-update-create-1",
            expected_revision=1,
            proposed_goal="Create initial Activity Journal coverage.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Record the initial draft."],
            dependencies=[],
            unresolved_decisions=[],
        )

        acknowledgement = drafts.update_draft(
            correlation_id="mission-draft-journal-update-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            proposed_goal="Create revised Activity Journal coverage.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Record the revised draft."],
            dependencies=[],
            unresolved_decisions=[],
        )

        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(
            [entry.action_type for entry in entries],
            ["mission-draft-created", "mission-draft-updated"],
        )
        entry = entries[1]
        self.assertEqual(entry.correlation_id, "mission-draft-journal-update-1")
        self.assertIn(created.draft_id, entry.summary)
        self.assertEqual(entry.affected_entities[1].entity_type, "mission-draft")
        self.assertEqual(entry.affected_entities[1].label, "Create revised Activity Journal coverage.")

    def test_activity_journal_records_acknowledged_mission_draft_abandonment(self) -> None:
        snapshots = self.load_service()
        drafts = MissionDraftService(snapshots)
        created = drafts.create_draft(
            correlation_id="mission-draft-journal-abandon-create-1",
            expected_revision=1,
            proposed_goal="Do not accept this Activity Journal scope.",
            selected_ad_hoc_ids=[],
            excluded_ad_hoc_ids=[],
            new_work_items=["Discard this draft after review."],
            dependencies=[],
            unresolved_decisions=[],
        )

        acknowledgement = drafts.abandon_draft(
            correlation_id="mission-draft-journal-abandon-1",
            expected_revision=created.revision,
            draft_id=created.draft_id,
            reason="The Mission Commander rejected this scope.",
        )

        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(
            [entry.action_type for entry in entries],
            ["mission-draft-created", "mission-draft-abandoned"],
        )
        entry = entries[1]
        self.assertEqual(entry.correlation_id, "mission-draft-journal-abandon-1")
        self.assertIn(created.draft_id, entry.summary)
        self.assertEqual(entry.affected_entities[1].entity_type, "mission-draft")
        self.assertEqual(entry.affected_entities[1].entity_id, created.draft_id)

    def test_activity_journal_records_frontier_confirmation_request_with_links(self) -> None:
        snapshots = self.load_service()
        snapshots._primary_mission.approve_issue("ISS-01")

        acknowledgement = WorkspaceQueueService(snapshots).request_frontier_confirmation(
            correlation_id="frontier-journal-confirmation-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="frontier-router",
            requested_action="Expand the launch boundary",
            affected_boundary="allowed_paths",
            consequence="Approval permits an additional generated file.",
            payload={"allowed_paths": ["generated/report.md"]},
        )

        projection = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(len(projection.entries), 1)
        entry = projection.entries[0]
        self.assertEqual(entry.actor, "frontier-model")
        self.assertEqual(entry.action_type, "frontier-confirmation-requested")
        self.assertEqual(entry.correlation_id, "frontier-journal-confirmation-1")
        self.assertIn(acknowledgement.item_id, entry.summary)
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "issue-slice",
                    "ISS-01",
                    "app-local://missions/command-deck/issues/ISS-01",
                ),
                (
                    "workspace-queue-item",
                    acknowledgement.item_id,
                    f"workspace-queue#{acknowledgement.item_id}",
                ),
            ],
        )

    def test_activity_journal_records_orchestrator_ad_hoc_session_launch_with_links(
        self,
    ) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Launch a bounded local-agent documentation session.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_ad_hoc_delegation(
            correlation_id="orchestrator-journal-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["Document the focused Activity Journal tests."],
            allowed_paths=["docs/activity-journal.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        acknowledgement = queue.decide(
            correlation_id="orchestrator-journal-decision-1",
            expected_revision=proposal.revision,
            item_id=proposal.item_id,
            decision="approve",
            reason="Approved within the proposed boundaries.",
        )

        entries = ActivityJournalService(self.load_service()).inspect().entries
        self.assertEqual(acknowledgement.outcome, "acknowledged")
        self.assertEqual(
            [(entry.actor, entry.action_type) for entry in entries],
            [
                ("mission-commander", "workspace-queue-decision"),
                ("orchestrator", "local-agent-session-launched"),
            ],
        )
        entry = entries[1]
        session_id = "session-ADHOC-000001-1"
        self.assertEqual(entry.correlation_id, "orchestrator-journal-decision-1")
        self.assertIn(session_id, entry.summary)
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "ad-hoc-delegation",
                    "ADHOC-000001",
                    "app-local://missions/command-deck/issues/ADHOC-000001",
                ),
                (
                    "local-agent-session",
                    session_id,
                    f"app-local://missions/command-deck/sessions/{session_id}",
                ),
                (
                    "workspace-queue-item",
                    proposal.item_id,
                    f"workspace-queue#{proposal.item_id}",
                ),
            ],
        )

    def test_activity_journal_records_local_agent_validated_evidence_with_links(self) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        mission.record_evidence(
            session.session_id,
            EvidencePackage(
                changed_files=["src/activity.py"],
                diff_summary="Recorded a meaningful Local Agent completion.",
                commands_run=["python3 -m unittest tests.test_workspace_snapshot"],
                test_results="Workspace tests passed.",
                known_risks="None.",
                proposed_context_updates="Document Local Agent Activity entries.",
                artifact_links=["app-local://evidence/local-agent-session-1"],
            ),
        )

        projection = ActivityJournalService(self.load_service()).inspect()
        self.assertEqual(len(projection.entries), 1)
        entry = projection.entries[0]
        self.assertEqual(entry.actor, "local-agent")
        self.assertEqual(entry.action_type, "evidence-package-submitted")
        self.assertEqual(entry.correlation_id, f"evidence:{session.session_id}")
        self.assertIn(session.session_id, entry.summary)
        self.assertEqual(
            entry.evidence_links,
            ("app-local://evidence/local-agent-session-1",),
        )
        self.assertEqual(
            [
                (entity.entity_type, entity.entity_id, entity.href)
                for entity in entry.affected_entities
            ],
            [
                ("mission", "command-deck", "app-local://missions/command-deck"),
                (
                    "issue-slice",
                    "ISS-01",
                    "app-local://missions/command-deck/issues/ISS-01",
                ),
                (
                    "local-agent-session",
                    session.session_id,
                    f"app-local://missions/command-deck/sessions/{session.session_id}",
                ),
                (
                    "evidence-package",
                    session.session_id,
                    "app-local://evidence/local-agent-session-1",
                ),
            ],
        )

    def test_activity_journal_write_failure_surfaces_after_canonical_queue_persistence(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_issue_contract_change(
            correlation_id="journal-write-failure-proposal-1",
            expected_revision=1,
            issue_id="ISS-01",
            source="issue-slice-inspector",
            acceptance_criteria=["Canonical state persists before journal failure."],
        )
        atomic_write = WorkspaceSnapshotService._write_json_atomically

        def fail_activity_write(path: Path, data: dict[str, object]) -> None:
            if path.name == "activity-journal.json":
                raise OSError("simulated full disk")
            atomic_write(path, data)

        with patch.object(
            WorkspaceSnapshotService,
            "_write_json_atomically",
            side_effect=fail_activity_write,
        ):
            with self.assertRaisesRegex(
                WorkspacePersistenceError,
                "Activity Journal persistence write failed",
            ):
                queue.decide(
                    correlation_id="journal-write-failure-decision-1",
                    expected_revision=proposal.revision,
                    item_id=proposal.item_id,
                    decision="approve",
                    reason="Exercise the explicit partial-write contract.",
                )

        reloaded = self.load_service()
        persisted_item = WorkspaceQueueService(reloaded).inspect().items[0]
        self.assertEqual(persisted_item.status, "approved")
        self.assertEqual(
            reloaded._primary_mission.issues["ISS-01"].acceptance_criteria,
            ["Canonical state persists before journal failure."],
        )
        self.assertEqual(ActivityJournalService(reloaded).inspect().entries, ())

    def test_restart_reconstructs_canonical_snapshot_and_retains_separate_journal_order(
        self,
    ) -> None:
        snapshots = self.load_service()
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        scope = snapshots.snapshot().conversation_scope
        WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="restart-journal-view-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=scope,
                operations_view="activity",
            )
        )
        WorkspaceQueueService(snapshots).request_frontier_confirmation(
            correlation_id="restart-journal-frontier-2",
            expected_revision=2,
            issue_id="ISS-01",
            source="frontier-router",
            requested_action="Confirm restart-safe launch scope",
            affected_boundary="allowed_paths",
            consequence="Approval would extend the launch boundary.",
            payload={"allowed_paths": ["docs/restart.md"]},
        )
        canonical_before_restart = snapshots.snapshot().to_dict()
        journal_before_restart = ActivityJournalService(snapshots).inspect()

        reloaded = self.load_service()
        canonical_after_restart = reloaded.snapshot().to_dict()
        journal_after_restart = ActivityJournalService(reloaded).inspect()

        self.assertEqual(canonical_after_restart, canonical_before_restart)
        self.assertEqual(journal_after_restart, journal_before_restart)
        self.assertEqual(
            [entry.correlation_id for entry in journal_after_restart.entries],
            ["restart-journal-view-1", "restart-journal-frontier-2"],
        )
        self.assertEqual(
            [entry.actor for entry in journal_after_restart.entries],
            ["mission-commander", "frontier-model"],
        )
        self.assertNotEqual(
            ActivityJournalService(reloaded).journal_path,
            reloaded._primary_mission.runtime_path,
        )

    def test_activity_journal_excludes_transient_output_and_failed_actions(self) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        with self.assertRaisesRegex(AlbertError, "Transient stream telemetry"):
            history.append(
                role="assistant",
                content="partial model token fragment",
                outcome="pending",
                source="frontier-model-stream",
            )
        with self.assertRaisesRegex(AlbertError, "Transient stream telemetry"):
            history.append(
                role="system",
                content="raw terminal byte chunk",
                outcome="pending",
                source="shell-terminal-stream",
            )
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")
        session = mission.launch_issue("ISS-01")

        with self.assertRaises(EvidenceValidationError):
            mission.record_evidence(
                session.session_id,
                EvidencePackage(changed_files=["src/incomplete.py"]),
            )
        with self.assertRaises(WorkspaceStaleActionError):
            WorkspaceSyncService(snapshots).submit_action(
                WorkspaceAction(
                    correlation_id="unacknowledged-stale-action-1",
                    expected_revision=0,
                    active_mission_id="command-deck",
                    conversation_scope=snapshots.snapshot().conversation_scope,
                    operations_view="activity",
                )
            )

        self.assertEqual(ActivityJournalService(self.load_service()).inspect().entries, ())

    def test_malformed_event_order_is_rejected_without_projecting_updates(self) -> None:
        snapshots = self.load_service()
        sync = WorkspaceSyncService(snapshots)
        sync.submit_action(
            WorkspaceAction(
                correlation_id="view-review-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=ConversationScope(
                    kind="working-directory",
                    target_id=str(self.target_repo),
                    label="target",
                ),
                operations_view="review-workspace",
            )
        )
        persisted = json.loads(snapshots.preferences_path.read_text(encoding="utf-8"))
        persisted["events"][0]["revision"] = 3
        snapshots.preferences_path.write_text(json.dumps(persisted), encoding="utf-8")

        with self.assertRaises(WorkspacePersistenceError):
            sync.updates_after(1)

    def test_event_lag_requires_resynchronization_instead_of_partial_batch(self) -> None:
        snapshots = self.load_service()
        sync = WorkspaceSyncService(snapshots)
        scope = ConversationScope(
            kind="working-directory",
            target_id=str(self.target_repo),
            label="target",
        )
        sync.submit_action(
            WorkspaceAction("view-review-1", 1, "command-deck", scope, "review-workspace")
        )
        sync.submit_action(
            WorkspaceAction("view-activity-2", 2, "command-deck", scope, "activity")
        )
        persisted = json.loads(snapshots.preferences_path.read_text(encoding="utf-8"))
        persisted["events"] = persisted["events"][1:]
        snapshots.preferences_path.write_text(json.dumps(persisted), encoding="utf-8")

        with self.assertRaises(WorkspaceRevisionGapError) as raised:
            sync.updates_after(1)

        self.assertEqual(raised.exception.requested_revision, 1)
        self.assertEqual(raised.exception.current_revision, 3)

    def test_empty_tracker_returns_valid_empty_workspace_snapshot(self) -> None:
        (self.tracker / "issues" / "01-restore.md").unlink()

        snapshot = self.load_service().snapshot()

        self.assertEqual(snapshot.workspace_session.status, "empty")
        self.assertEqual(snapshot.active_mission.issue_count, 0)
        self.assertEqual(snapshot.mission_board["ordered_issue_ids"], [])

    def test_mission_board_projects_authoritative_issue_graph_and_inspector_payload(self) -> None:
        (self.tracker / "issues" / "02-sync.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Synchronize live state.").replace(
                "None - can start immediately",
                "- `01-restore.md`",
            ),
            encoding="utf-8",
        )
        snapshots = self.load_service()
        mission = snapshots._mission
        mission.approve_issue("ISS-02")
        session = LocalAgentSession(
            session_id="session-ISS-02-1",
            issue_id="ISS-02",
            assigned_agent="qwen-coder-local",
            worktree_path=self.target_repo / ".albert-worktrees" / "ISS-02",
            task_packet={
                "agent_config": {
                    "role": "local-agent",
                    "provider": "ollama",
                    "model": "qwen2.5-coder:14b",
                }
            },
            status="launched",
            evidence=EvidencePackage(
                changed_files=["mission-control/src/App.tsx"],
                diff_summary="Added Mission Board inspector.",
                commands_run=["npm test -- --run App.test.tsx"],
                test_results="Frontend interaction test passed.",
                known_risks="Disconnected before review.",
                proposed_context_updates="Issue 06 progress.",
                artifact_links=["app-local://evidence/ISS-02"],
            ),
            evidence_valid=True,
            runner_started_at="2026-06-25T10:00:00Z",
        )
        mission.sessions[session.session_id] = session

        board = snapshots.snapshot().mission_board
        sync = next(issue for issue in board["issue_slices"] if issue["issue_id"] == "ISS-02")

        self.assertEqual(board["ready_issue_ids"], [])
        self.assertEqual(sync["lifecycle"], "Approved")
        self.assertEqual(sync["progress"], "Waiting on ISS-01")
        self.assertEqual(sync["blockers"][0]["issue_id"], "ISS-01")
        self.assertEqual(sync["blockers"][0]["satisfied"], False)
        self.assertEqual(sync["accepted_boundary"]["what_to_build"], "Synchronize live state.")
        self.assertEqual(sync["sessions"][0]["session_id"], "session-ISS-02-1")
        self.assertEqual(sync["sessions"][0]["provider"], "ollama")
        self.assertEqual(sync["sessions"][0]["model"], "qwen2.5-coder:14b")
        self.assertEqual(sync["evidence"]["test_results"], "Frontend interaction test passed.")
        self.assertEqual(sync["working_context_sources"][0]["kind"], "shared-context")

    def test_corrupt_preferences_return_structured_persistence_failure(self) -> None:
        service = self.load_service()
        preferences = service.preferences_path
        preferences.write_text("{broken", encoding="utf-8")
        output = io.StringIO()
        error = io.StringIO()

        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(
                [
                    "workspace-snapshot",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                ]
            )

        failure = json.loads(error.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(failure["error"]["code"], "persistence-read-failure")
        self.assertEqual(failure["error"]["recoverable"], True)

    def test_cli_can_load_prd_and_issue_records_from_explicit_separate_locations(self) -> None:
        prd_tracker = self.root / "command-deck-tracker"
        prd_tracker.mkdir()
        (prd_tracker / "PRD.md").write_text("# Current Command Deck\n", encoding="utf-8")
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workspace-snapshot",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(prd_tracker),
                    "--issues-dir",
                    str(self.tracker / "issues"),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "current-command-deck",
                ]
            )

        snapshot = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(snapshot["active_mission"]["title"], "Current Command Deck")
        self.assertEqual(snapshot["mission_board"]["ordered_issue_ids"], ["ISS-01"])

    def test_cli_acknowledges_correlated_workspace_action(self) -> None:
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workspace-action",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "view-activity-1",
                    "--expected-revision",
                    "1",
                    "--operations-view",
                    "activity",
                ]
            )

        acknowledgement = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(acknowledgement["correlation_id"], "view-activity-1")
        self.assertEqual(acknowledgement["outcome"], "acknowledged")
        self.assertEqual(acknowledgement["revision"], 2)

    def test_cli_returns_ordered_workspace_updates_after_known_revision(self) -> None:
        sync = WorkspaceSyncService(self.load_service())
        current = self.load_service().snapshot()
        sync.submit_action(
            WorkspaceAction(
                "view-activity-1",
                1,
                "command-deck",
                current.conversation_scope,
                "activity",
            )
        )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workspace-updates",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--after-revision",
                    "1",
                ]
            )

        batch = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(batch["after_revision"], 1)
        self.assertEqual(batch["current_revision"], 2)
        self.assertEqual([event["revision"] for event in batch["events"]], [2])
        self.assertEqual(batch["events"][0]["correlation_id"], "view-activity-1")

    def test_cli_returns_structured_stale_action_without_success_output(self) -> None:
        current = self.load_service().snapshot()
        WorkspaceSyncService(self.load_service()).submit_action(
            WorkspaceAction(
                "view-activity-1",
                1,
                "command-deck",
                current.conversation_scope,
                "activity",
            )
        )
        output = io.StringIO()
        error = io.StringIO()

        with redirect_stdout(output), redirect_stderr(error):
            exit_code = main(
                [
                    "workspace-action",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "view-stale-2",
                    "--expected-revision",
                    "1",
                    "--operations-view",
                    "mission-board",
                ]
            )

        failure = json.loads(error.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(failure["error"]["code"], "stale-action")
        self.assertEqual(failure["error"]["current_revision"], 2)
        self.assertEqual(self.load_service().snapshot().operations_view, "activity")

    def test_cli_deliberately_changes_mission_scope_without_authorization_side_effects(self) -> None:
        issue_path = self.tracker / "issues" / "01-restore.md"
        original_issue = issue_path.read_text(encoding="utf-8")
        before = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="command-deck",
        ).load()
        before_detail = before.issue_detail("ISS-01")
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "workspace-scope",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--correlation-id",
                    "scope-mission-1",
                    "--expected-revision",
                    "1",
                    "--scope-kind",
                    "mission",
                    "--scope-target",
                    "command-deck",
                    "--scope-label",
                    "Command Deck Mission",
                ]
            )

        acknowledgement = json.loads(output.getvalue())
        restored = self.load_service().snapshot()
        after = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="command-deck",
        ).load()
        after_detail = after.issue_detail("ISS-01")
        self.assertEqual(exit_code, 0)
        self.assertEqual(acknowledgement["correlation_id"], "scope-mission-1")
        self.assertEqual(acknowledgement["revision"], 2)
        self.assertEqual(restored.conversation_scope.kind, "mission")
        self.assertEqual(restored.conversation_scope.target_id, "command-deck")
        self.assertEqual(restored.operations_view, "mission-board")
        self.assertEqual(issue_path.read_text(encoding="utf-8"), original_issue)
        self.assertEqual(after_detail["review_state"], before_detail["review_state"])
        self.assertEqual(after_detail["runtime_status"], before_detail["runtime_status"])
        self.assertEqual(after_detail["assigned_agent"], before_detail["assigned_agent"])
        self.assertEqual(len(after.sessions), len(before.sessions))

    def test_agent_console_history_restores_proposed_message_with_original_scope(self) -> None:
        first = AgentConsoleHistoryService(self.load_service())

        appended = first.append(
            role="user",
            content="Review the restore boundary.",
            outcome="proposed",
            source="mission-commander",
        )
        restored = AgentConsoleHistoryService(self.load_service()).history()

        self.assertEqual(appended.message_id, "console-000001")
        self.assertEqual(appended.sequence, 1)
        self.assertEqual(restored, (appended,))
        self.assertEqual(restored[0].scope.kind, "working-directory")
        self.assertEqual(restored[0].scope.target_id, str(self.target_repo))

    def test_agent_console_history_rejects_malformed_scope_kind(self) -> None:
        history = AgentConsoleHistoryService(self.load_service())
        history.append(
            role="assistant",
            content="This is model analysis only.",
            outcome="model-commentary",
            source="frontier-model",
        )
        persisted = json.loads(history.history_path.read_text(encoding="utf-8"))
        persisted["messages"][0]["scope"]["kind"] = "selected-panel"
        history.history_path.write_text(json.dumps(persisted), encoding="utf-8")

        with self.assertRaises(WorkspacePersistenceError):
            AgentConsoleHistoryService(self.load_service()).history()

    def test_cli_appends_scoped_agent_console_message(self) -> None:
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "agent-console-message",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--role",
                    "user",
                    "--content",
                    "Explain the current mission.",
                    "--outcome",
                    "proposed",
                    "--source",
                    "mission-commander",
                    "--expected-revision",
                    "1",
                    "--scope-kind",
                    "working-directory",
                    "--scope-target",
                    str(self.target_repo),
                    "--scope-label",
                    "target",
                ]
            )

        message = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(message["message_id"], "console-000001")
        self.assertEqual(message["sequence"], 1)
        self.assertEqual(message["scope"]["kind"], "working-directory")
        self.assertEqual(message["outcome"], "proposed")

    def test_cli_appends_issue_scoped_message_with_displayed_scope_values(self) -> None:
        snapshots = self.load_service()
        WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="scope-issue-1",
                expected_revision=1,
                active_mission_id="command-deck",
                conversation_scope=ConversationScope(
                    kind="issue-slice",
                    target_id="ISS-01",
                    label="Restore workspace session",
                ),
                operations_view="mission-board",
            )
        )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "agent-console-message",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--role",
                    "user",
                    "--content",
                    "Explain this Issue Slice.",
                    "--outcome",
                    "proposed",
                    "--source",
                    "mission-commander",
                    "--expected-revision",
                    "2",
                    "--scope-kind",
                    "issue-slice",
                    "--scope-target",
                    "ISS-01",
                    "--scope-label",
                    "Restore workspace session",
                ]
            )

        message = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(message["scope"]["kind"], "issue-slice")
        self.assertEqual(message["scope"]["target_id"], "ISS-01")
        self.assertEqual(message["scope"]["mission_id"], "command-deck")

    def test_cli_reads_agent_console_history_after_restart(self) -> None:
        AgentConsoleHistoryService(self.load_service()).append(
            role="assistant",
            content="The mission remains in progress.",
            outcome="model-commentary",
            source="frontier-model",
        )
        output = io.StringIO()

        with redirect_stdout(output):
            exit_code = main(
                [
                    "agent-console-history",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                ]
            )

        history = json.loads(output.getvalue())
        self.assertEqual(exit_code, 0)
        self.assertEqual(history["schema_version"], 1)
        self.assertEqual(len(history["messages"]), 1)
        self.assertEqual(history["messages"][0]["outcome"], "model-commentary")
        self.assertEqual(history["messages"][0]["source"], "frontier-model")

    def test_agent_console_history_skips_legacy_transient_telemetry_on_restore(
        self,
    ) -> None:
        history = AgentConsoleHistoryService(self.load_service())
        durable = history.append(
            role="user",
            content="Keep this durable prompt.",
            outcome="proposed",
            source="mission-commander",
        )
        persisted = json.loads(history.history_path.read_text(encoding="utf-8"))
        persisted["messages"].append(
            {
                "message_id": "console-000002",
                "sequence": 2,
                "role": "system",
                "content": "raw telemetry chunk",
                "scope": persisted["messages"][0]["scope"],
                "outcome": "pending",
                "source": "raw-telemetry",
            }
        )
        history.history_path.write_text(json.dumps(persisted), encoding="utf-8")

        restored = AgentConsoleHistoryService(self.load_service()).history()

        self.assertEqual([message.content for message in restored], [durable.content])
        self.assertEqual(restored[0].message_id, "console-000001")
        self.assertEqual(restored[0].sequence, 1)

    def test_release_transcript_persists_prompts_actions_outcomes_and_excludes_navigation(
        self,
    ) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        history.append(
            role="user",
            content="Launch the ready Alfredo workstation slice.",
            outcome="proposed",
            source="mission-commander",
        )
        history.append(
            role="assistant",
            content="I will route the launch through the Orchestrator.",
            outcome="model-commentary",
            source="frontier-model",
        )
        mission = snapshots._primary_mission
        mission.approve_issue("ISS-01")

        acknowledgement = WorkstationActionService(snapshots).submit(
            correlation_id="release-seam-launch-1",
            action_type="issue-launch",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
            allowed_paths=["src"],
            command_policy={"python3 -m unittest": "auto-allowed"},
        )
        WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="release-seam-routine-navigation-1",
                expected_revision=acknowledgement.revision,
                active_mission_id="command-deck",
                conversation_scope=snapshots.snapshot().conversation_scope,
                operations_view="activity",
            )
        )

        restored = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual(
            [(message.source, message.outcome) for message in restored],
            [
                ("mission-commander", "proposed"),
                ("frontier-model", "model-commentary"),
                ("mission-commander", "pending"),
                ("orchestrator", "acknowledged"),
            ],
        )
        self.assertEqual(
            [message.content for message in restored],
            [
                "Launch the ready Alfredo workstation slice.",
                "I will route the launch through the Orchestrator.",
                "Workstation action: Mission Commander requested issue launch for ISS-01.",
                "Orchestrator accepted workstation action: Orchestrator launched ISS-01 as session-ISS-01-1.",
            ],
        )
        self.assertNotIn(
            "release-seam-routine-navigation-1",
            "\n".join(message.content for message in restored),
        )
        with self.assertRaisesRegex(AlbertError, "Transient stream telemetry"):
            history.append(
                role="system",
                content="raw telemetry chunk",
                outcome="pending",
                source="raw-telemetry",
            )

    def test_release_transcript_preserves_reason_required_workstation_action_detail(
        self,
    ) -> None:
        snapshots = self.load_service()

        acknowledgement = WorkstationActionService(snapshots).submit(
            correlation_id="release-seam-model-assignment-1",
            action_type="model-assignment-change",
            actor="mission-commander",
            expected_revision=1,
            target_kind="issue-slice",
            target_id="ISS-01",
            issue_id="ISS-01",
            agent_id="qwen3.6-27b",
            reason="Use the release verification model.",
        )

        restored = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(
            [message.content for message in restored],
            [
                "Workstation action: Mission Commander requested model assignment change for ISS-01.",
                (
                    "Orchestrator accepted workstation action: Mission Commander assigned "
                    "ISS-01 to qwen3.6-27b: Use the release verification model."
                ),
            ],
        )
        entry = ActivityJournalService(self.load_service()).inspect().entries[0]
        self.assertEqual(entry.action_type, "model-assignment-change")
        self.assertEqual(entry.actor, "mission-commander")
        self.assertIn("Use the release verification model.", entry.summary)

    def test_release_transcript_and_journal_record_shell_decisions_without_raw_bytes(
        self,
    ) -> None:
        snapshots = self.load_service()
        terminal = ShellTerminalService(snapshots)
        command = "python3 -c \"print('raw terminal byte should stay transient')\""

        pending = terminal.submit(
            correlation_id="release-terminal-command-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )
        denied = terminal.deny(
            command_id=pending.command_id,
            decider="mission-commander",
            reason="Do not run this command during release verification.",
        )
        external = self.root / "release-grant"
        external.mkdir()
        grant = terminal.create_path_grant(
            correlation_id="release-path-grant-1",
            expected_revision=2,
            path=str(external),
            access_level="read",
            duration_seconds=300,
            requester="mission-commander",
        )
        approved_command = terminal.submit(
            correlation_id="release-terminal-approved-1",
            command=command,
            working_directory=str(self.target_repo),
            requested_paths=[],
            requester="mission-commander",
        )
        approved = terminal.approve(
            command_id=approved_command.command_id,
            approver="mission-commander",
        )

        self.assertEqual(denied.status, "denied")
        self.assertEqual(grant.grant_id, "path-grant-000001")
        self.assertEqual(approved.status, "completed")
        self.assertIn("raw terminal byte", approved.stdout)

        restored_history = AgentConsoleHistoryService(self.load_service()).history()
        restored_journal = ActivityJournalService(self.load_service()).inspect()

        self.assertEqual(
            [message.outcome for message in restored_history],
            [
                "pending",
                "rejected",
                "acknowledged",
                "pending",
                "acknowledged",
            ],
        )
        history_text = "\n".join(message.content for message in restored_history)
        self.assertIn("Shell Terminal command requires mission-commander approval", history_text)
        self.assertIn("Mission Commander denied Shell Terminal command", history_text)
        self.assertIn("Mission Commander granted read Additional Path Grant", history_text)
        self.assertIn("Shell Terminal command completed with exit code 0", history_text)
        self.assertNotIn("raw terminal byte should stay transient", history_text)

        self.assertEqual(
            [entry.action_type for entry in restored_journal.entries],
            [
                "shell-command-approval-requested",
                "shell-command-denied",
                "additional-path-grant-created",
                "shell-command-approval-requested",
                "shell-command-completed",
            ],
        )
        self.assertEqual(
            [entry.actor for entry in restored_journal.entries],
            [
                "orchestrator",
                "mission-commander",
                "mission-commander",
                "orchestrator",
                "orchestrator",
            ],
        )
        journal_text = "\n".join(entry.summary for entry in restored_journal.entries)
        self.assertIn("release-path-grant-1", journal_text)
        self.assertNotIn("raw terminal byte should stay transient", journal_text)
        self.assertIn(
            "terminal-command-000002",
            {
                entity.entity_id
                for entry in restored_journal.entries
                for entity in entry.affected_entities
            },
        )

    def test_agent_console_history_preserves_all_distinct_outcomes_in_order(self) -> None:
        history = AgentConsoleHistoryService(self.load_service())
        outcomes = [
            "proposed",
            "pending",
            "acknowledged",
            "rejected",
            "model-commentary",
        ]

        for outcome in outcomes:
            history.append(
                role="assistant" if outcome == "model-commentary" else "system",
                content=f"Outcome: {outcome}",
                outcome=outcome,
                source="frontier-model" if outcome == "model-commentary" else "orchestrator",
            )

        restored = AgentConsoleHistoryService(self.load_service()).history()
        self.assertEqual([message.outcome for message in restored], outcomes)
        self.assertEqual([message.sequence for message in restored], [1, 2, 3, 4, 5])
        self.assertEqual(
            [message.message_id for message in restored],
            [f"console-{sequence:06d}" for sequence in range(1, 6)],
        )

    def test_agent_console_rejects_message_when_displayed_scope_is_not_acknowledged_scope(self) -> None:
        history = AgentConsoleHistoryService(self.load_service())

        with self.assertRaises(WorkspaceScopeMismatchError):
            history.append(
                role="user",
                content="This must stay targeted to the Mission.",
                outcome="proposed",
                source="mission-commander",
                expected_revision=1,
                expected_scope=ConversationScope(
                    kind="mission",
                    target_id="command-deck",
                    label="Command Deck Mission",
                ),
            )

        self.assertEqual(history.history(), ())

    def test_working_context_reconstructs_bounded_sources_for_issue_scope(self) -> None:
        snapshots = self.load_service()
        snapshots.update_preferences(
            active_mission_id="command-deck",
            conversation_scope=ConversationScope(
                kind="issue-slice",
                target_id="ISS-01",
                label="Restore workspace session",
            ),
            operations_view="mission-board",
        )
        history = AgentConsoleHistoryService(snapshots)
        for sequence in range(1, 9):
            history.append(
                role="user",
                content=f"Scoped message {sequence}",
                outcome="proposed",
                source="mission-commander",
            )

        contexts = WorkingContextService(self.load_service())
        acknowledgement = contexts.curate(
            source_id="message:console-000001",
            disposition="pinned",
            expected_revision=1,
        )
        projection = WorkingContextService(self.load_service()).inspect()

        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(
            {source.kind for source in projection.sources},
            {
                "workspace-session",
                "shared-context",
                "unresolved-item",
                "recent-conversation",
                "deliberate-reference",
            },
        )
        governed = [source for source in projection.sources if source.governed]
        self.assertEqual(
            {source.kind for source in governed},
            {"workspace-session", "shared-context"},
        )
        self.assertTrue(all(source.disposition == "required" for source in governed))
        recent = [source for source in projection.sources if source.kind == "recent-conversation"]
        self.assertEqual(len(recent), 6)
        self.assertEqual(recent[0].source_id, "message:console-000003")
        deliberate = [
            source for source in projection.sources if source.kind == "deliberate-reference"
        ]
        self.assertEqual([source.source_id for source in deliberate], ["message:console-000001"])
        self.assertLessEqual(projection.content_character_count, 4_000)
        self.assertEqual(len(AgentConsoleHistoryService(self.load_service()).history()), 8)

    def test_working_context_recent_window_uses_only_acknowledged_scope(self) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        history.append(
            role="user",
            content="Working directory question",
            outcome="proposed",
            source="mission-commander",
        )
        snapshots.update_preferences(
            active_mission_id="command-deck",
            conversation_scope=ConversationScope(
                kind="issue-slice",
                target_id="ISS-01",
                label="Restore workspace session",
            ),
            operations_view="mission-board",
        )
        history.append(
            role="user",
            content="Issue Slice question",
            outcome="proposed",
            source="mission-commander",
        )

        projection = WorkingContextService(self.load_service()).inspect()
        recent_content = [
            source.content
            for source in projection.sources
            if source.kind == "recent-conversation"
        ]

        self.assertEqual(len(recent_content), 1)
        self.assertIn("Issue Slice question", recent_content[0])
        self.assertNotIn("Working directory question", " ".join(recent_content))
        self.assertEqual(len(AgentConsoleHistoryService(self.load_service()).history()), 2)

    def test_working_context_rejects_duplicate_persisted_curation(self) -> None:
        contexts = WorkingContextService(self.load_service())
        contexts.curation_path.parent.mkdir(parents=True, exist_ok=True)
        contexts.curation_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "revision": 2,
                    "pinned_source_ids": [
                        "issue:command-deck:ISS-01",
                        "issue:command-deck:ISS-01",
                    ],
                    "excluded_source_ids": [],
                }
            ),
            encoding="utf-8",
        )

        with self.assertRaises(WorkspacePersistenceError):
            WorkingContextService(self.load_service()).inspect()

    def test_working_context_pin_and_exclude_restore_at_acknowledged_revision(self) -> None:
        snapshots = self.load_service()
        history = AgentConsoleHistoryService(snapshots)
        history.append(
            role="user",
            content="Keep this source available",
            outcome="proposed",
            source="mission-commander",
        )
        contexts = WorkingContextService(snapshots)

        pinned = contexts.curate(
            source_id="message:console-000001",
            disposition="pinned",
            expected_revision=1,
        )
        excluded = WorkingContextService(self.load_service()).curate(
            source_id="issue:command-deck:ISS-01",
            disposition="excluded",
            expected_revision=pinned.revision,
        )
        restored = WorkingContextService(self.load_service()).inspect()
        dispositions = {source.source_id: source.disposition for source in restored.sources}

        self.assertEqual(excluded.revision, 3)
        self.assertEqual(restored.revision, 3)
        self.assertEqual(dispositions["message:console-000001"], "pinned")
        self.assertEqual(dispositions["issue:command-deck:ISS-01"], "excluded")
        self.assertEqual(self.load_service().snapshot().revision, 1)
        self.assertEqual(len(AgentConsoleHistoryService(self.load_service()).history()), 1)

    def test_working_context_rejects_governed_and_stale_curation_without_side_effects(self) -> None:
        snapshots = self.load_service()
        contexts = WorkingContextService(snapshots)
        before_snapshot = snapshots.snapshot()
        issue_path = self.tracker / "issues" / "01-restore.md"
        before_issue = issue_path.read_text(encoding="utf-8")

        with self.assertRaisesRegex(WorkingContextCurationError, "not eligible"):
            contexts.curate(
                source_id=f"shared-context:command-deck:working-directory:{self.target_repo}",
                disposition="excluded",
                expected_revision=1,
            )
        with self.assertRaises(WorkspaceStaleActionError):
            contexts.curate(
                source_id="issue:command-deck:ISS-01",
                disposition="pinned",
                expected_revision=0,
            )

        after_snapshot = self.load_service().snapshot()
        self.assertEqual(after_snapshot, before_snapshot)
        self.assertEqual(issue_path.read_text(encoding="utf-8"), before_issue)
        self.assertFalse(contexts.curation_path.exists())
        self.assertEqual(AgentConsoleHistoryService(self.load_service()).history(), ())

    def test_cli_curates_and_restores_working_context_projection(self) -> None:
        AgentConsoleHistoryService(self.load_service()).append(
            role="user",
            content="Pin this CLI source",
            outcome="proposed",
            source="mission-commander",
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
        ]
        acknowledgement_output = io.StringIO()
        with redirect_stdout(acknowledgement_output):
            curate_exit = main(
                [
                    "working-context-curate",
                    *common,
                    "--source-id",
                    "message:console-000001",
                    "--disposition",
                    "pinned",
                    "--expected-context-revision",
                    "1",
                ]
            )
        projection_output = io.StringIO()
        with redirect_stdout(projection_output):
            inspect_exit = main(["working-context", *common])

        acknowledgement = json.loads(acknowledgement_output.getvalue())
        projection = json.loads(projection_output.getvalue())
        message_source = next(
            source
            for source in projection["sources"]
            if source["source_id"] == "message:console-000001"
        )
        self.assertEqual(curate_exit, 0)
        self.assertEqual(inspect_exit, 0)
        self.assertEqual(acknowledgement, {"outcome": "acknowledged", "revision": 2})
        self.assertEqual(projection["revision"], 2)
        self.assertEqual(message_source["disposition"], "pinned")
        self.assertEqual(projection["scope"]["kind"], "working-directory")

    def test_cli_returns_structured_rejection_for_governed_context_source(self) -> None:
        error_output = io.StringIO()

        with redirect_stderr(error_output):
            exit_code = main(
                [
                    "working-context-curate",
                    "--target-repo",
                    str(self.target_repo),
                    "--tracker-dir",
                    str(self.tracker),
                    "--runtime-root",
                    str(self.runtime),
                    "--mission-id",
                    "command-deck",
                    "--source-id",
                    f"shared-context:command-deck:working-directory:{self.target_repo}",
                    "--disposition",
                    "excluded",
                    "--expected-context-revision",
                    "1",
                ]
            )

        failure = json.loads(error_output.getvalue())
        self.assertEqual(exit_code, 1)
        self.assertEqual(failure["error"]["code"], "context-source-ineligible")
        self.assertFalse(WorkingContextService(self.load_service()).curation_path.exists())

    def test_runtime_persistence_never_exposes_a_truncated_canonical_file(self) -> None:
        mission = self.load_service()._mission
        runtime_path = mission.runtime_path
        original_write_text = Path.write_text
        write_started = threading.Event()
        release_write = threading.Event()

        def paused_write(path: Path, data: str, *args: object, **kwargs: object) -> int:
            if path != runtime_path:
                return original_write_text(path, data, *args, **kwargs)
            encoding = kwargs.get("encoding", "utf-8")
            with path.open("w", encoding=str(encoding)) as stream:
                write_started.set()
                self.assertTrue(release_write.wait(timeout=2), "test must release persistence")
                return stream.write(data)

        original_replace = Path.replace

        def paused_replace(path: Path, target: Path) -> Path:
            if Path(target) == runtime_path:
                write_started.set()
                self.assertTrue(release_write.wait(timeout=2), "test must release persistence")
            return original_replace(path, target)

        writer = threading.Thread(target=mission._persist)
        with (
            patch.object(Path, "write_text", new=paused_write),
            patch.object(Path, "replace", new=paused_replace),
        ):
            writer.start()
            self.assertTrue(write_started.wait(timeout=2), "persistence should begin")
            try:
                persisted = json.loads(runtime_path.read_text(encoding="utf-8"))
            finally:
                release_write.set()
                writer.join(timeout=2)

        self.assertEqual(persisted["mission_id"], "command-deck")
        self.assertFalse(writer.is_alive())

    def test_switches_active_mission_without_stopping_background_work_or_retargeting(self) -> None:
        primary = self.load_service()._mission
        primary.sessions["session-ISS-01-1"] = LocalAgentSession(
            session_id="session-ISS-01-1",
            issue_id="ISS-01",
            assigned_agent="local-agent",
            worktree_path=self.target_repo / ".albert-worktrees" / "ISS-01",
            task_packet={"mission_id": "command-deck"},
            status="launched",
        )
        primary.delegations["ISS-01"] = DelegationDecision(
            issue_id="ISS-01",
            router_agent="frontier-router",
            recommended_agent="gated-worker",
            complexity="high",
            reason="Background approval required",
            requires_approval=True,
            approved=False,
        )
        primary.issues["ISS-01"].review_state = "needs-human-review"
        background_tracker = self.root / "background-tracker"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text(
            "# Background Mission\n", encoding="utf-8"
        )
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Run background work."),
            encoding="utf-8",
        )
        second = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-mission",
            allow_empty_tracker=True,
        ).load()
        snapshots = WorkspaceSnapshotService(primary, missions=(second,))
        history = AgentConsoleHistoryService(snapshots)
        history.append(
            role="user",
            content="Keep this Workspace conversation continuous",
            outcome="proposed",
            source="mission-commander",
        )
        before = snapshots.snapshot()

        acknowledgement = WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id="active-mission-background-1",
                expected_revision=1,
                active_mission_id="background-mission",
                conversation_scope=before.conversation_scope,
                operations_view="mission-board",
            )
        )
        switched = snapshots.snapshot()
        previous = next(mission for mission in switched.missions if mission.id == "command-deck")

        self.assertEqual(acknowledgement.revision, 2)
        self.assertEqual(switched.active_mission.id, "background-mission")
        self.assertEqual(switched.mission_board["prd_title"], "Background Mission")
        self.assertEqual(switched.workspace_session, before.workspace_session)
        self.assertEqual(switched.conversation_scope, before.conversation_scope)
        self.assertFalse(previous.is_active)
        self.assertEqual(previous.sessions[0].status, "launched")
        self.assertEqual(previous.sessions[0].session_id, "session-ISS-01-1")
        self.assertEqual(previous.attention[0].kind, "delegation-approval")
        self.assertEqual(previous.attention[0].queue_link, "workspace-queue#delegation-command-deck-ISS-01")
        self.assertEqual(previous.attention[1].kind, "clarification")
        self.assertEqual(previous.attention[1].queue_link, "workspace-queue#clarification-command-deck-ISS-01")
        self.assertEqual(AgentConsoleHistoryService(snapshots).history(), history.history())
        self.assertEqual(primary.sessions["session-ISS-01-1"].status, "launched")

    def test_issue_scope_ownership_selects_shared_context_from_the_active_mission(self) -> None:
        primary = self.load_service()._mission
        background_tracker = self.root / "background-scope-tracker"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text(
            "# Background Scope Mission\n", encoding="utf-8"
        )
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Run background-only work."),
            encoding="utf-8",
        )
        background = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=background_tracker,
            runtime_root=self.runtime,
            mission_id="background-scope",
            allow_empty_tracker=True,
        ).load()
        snapshots = WorkspaceSnapshotService(primary, missions=(background,))
        sync = WorkspaceSyncService(snapshots)
        initial = snapshots.snapshot()
        switched = sync.submit_action(
            WorkspaceAction(
                correlation_id="switch-background-scope",
                expected_revision=1,
                active_mission_id="background-scope",
                conversation_scope=initial.conversation_scope,
                operations_view="mission-board",
            )
        )
        sync.submit_action(
            WorkspaceAction(
                correlation_id="scope-background-issue",
                expected_revision=switched.revision,
                active_mission_id="background-scope",
                conversation_scope=ConversationScope(
                    kind="issue-slice",
                    target_id="ISS-01",
                    label="Background Issue",
                ),
                operations_view="mission-board",
            )
        )

        scoped = snapshots.snapshot()
        projection = WorkingContextService(snapshots).inspect()
        shared = next(source for source in projection.sources if source.kind == "shared-context")

        self.assertEqual(scoped.conversation_scope.mission_id, "background-scope")
        self.assertIn("Background Scope Mission", shared.content)
        self.assertIn("Run background-only work", shared.content)
        self.assertNotIn("Restore the workspace session", shared.content)

    def test_cli_switches_catalog_mission_and_restores_background_state(self) -> None:
        primary = self.load_service()._mission
        primary.sessions["session-ISS-01-1"] = LocalAgentSession(
            session_id="session-ISS-01-1",
            issue_id="ISS-01",
            assigned_agent="local-agent",
            worktree_path=self.target_repo / ".albert-worktrees" / "ISS-01",
            task_packet={"mission_id": "command-deck"},
            status="launched",
        )
        primary.delegations["ISS-01"] = DelegationDecision(
            issue_id="ISS-01",
            router_agent="frontier-router",
            recommended_agent="gated-worker",
            complexity="high",
            reason="Approval required",
            requires_approval=True,
            approved=False,
        )
        primary._persist()
        AgentConsoleHistoryService(WorkspaceSnapshotService(primary)).append(
            role="user",
            content="Continuous catalog conversation",
            outcome="proposed",
            source="mission-commander",
        )
        background_tracker = self.root / "catalog-background"
        (background_tracker / "issues").mkdir(parents=True)
        (background_tracker / "PRD.md").write_text("# Catalog Background\n", encoding="utf-8")
        (background_tracker / "issues" / "01-background.md").write_text(
            ISSUE.replace("Restore the workspace session.", "Catalog background work."),
            encoding="utf-8",
        )
        catalog = self.root / "mission-catalog.json"
        catalog.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "missions": [
                        {
                            "mission_id": "catalog-background",
                            "tracker_dir": str(background_tracker),
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )
        common = [
            "--target-repo",
            str(self.target_repo),
            "--tracker-dir",
            str(self.tracker),
            "--runtime-root",
            str(self.runtime),
            "--mission-id",
            "command-deck",
            "--mission-catalog",
            str(catalog),
        ]
        acknowledgement_output = io.StringIO()
        with redirect_stdout(acknowledgement_output):
            switch_exit = main(
                [
                    "workspace-mission-switch",
                    *common,
                    "--correlation-id",
                    "switch-catalog-background",
                    "--expected-revision",
                    "1",
                    "--active-mission-id",
                    "catalog-background",
                ]
            )
        snapshot_output = io.StringIO()
        with redirect_stdout(snapshot_output):
            snapshot_exit = main(["workspace-snapshot", *common])
        history_output = io.StringIO()
        with redirect_stdout(history_output):
            history_exit = main(["agent-console-history", *common])

        acknowledgement = json.loads(acknowledgement_output.getvalue())
        restored = json.loads(snapshot_output.getvalue())
        history = json.loads(history_output.getvalue())
        previous = next(item for item in restored["missions"] if item["id"] == "command-deck")
        self.assertEqual(switch_exit, 0)
        self.assertEqual(snapshot_exit, 0)
        self.assertEqual(history_exit, 0)
        self.assertEqual(acknowledgement["revision"], 2)
        self.assertEqual(restored["active_mission"]["id"], "catalog-background")
        self.assertEqual(restored["conversation_scope"]["kind"], "working-directory")
        self.assertEqual(previous["sessions"][0]["status"], "launched")
        self.assertEqual(previous["attention"][0]["kind"], "delegation-approval")
        self.assertEqual(history["messages"][0]["content"], "Continuous catalog conversation")

    def load_service(self) -> WorkspaceSnapshotService:
        mission = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="command-deck",
            allow_empty_tracker=True,
        ).load()
        return WorkspaceSnapshotService(mission)


if __name__ == "__main__":
    unittest.main()
