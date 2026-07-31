from __future__ import annotations

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from albert_mvp.core import AlbertMission
from albert_mvp.workspace import (
    AgentConsoleHistoryService,
    WorkspaceQueueService,
    WorkspaceSnapshotService,
)


ISSUE = """Status: ready-for-agent
Type: AFK

## Parent

PRD.md

## What to build

Measure crash recovery.

## Acceptance criteria

- [ ] No partial approval survives.

## Blocked by

None - can start immediately
"""


class PerformanceCorrectnessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.target_repo = self.root / "target"
        self.target_repo.mkdir()
        self.tracker = self.root / "tracker"
        (self.tracker / "issues").mkdir(parents=True)
        (self.tracker / "PRD.md").write_text("# Performance Mission\n", encoding="utf-8")
        (self.tracker / "issues" / "01-performance.md").write_text(
            ISSUE,
            encoding="utf-8",
        )
        self.runtime = self.root / "runtime"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def load_service(self) -> WorkspaceSnapshotService:
        mission = AlbertMission(
            target_repo=self.target_repo,
            tracker_dir=self.tracker,
            runtime_root=self.runtime,
            mission_id="performance-mission",
            allow_empty_tracker=True,
        ).load()
        return WorkspaceSnapshotService(mission)

    def test_ad_hoc_decision_crash_before_first_durable_write_preserves_old_state(self) -> None:
        snapshots = self.load_service()
        origin = AgentConsoleHistoryService(snapshots).append(
            role="user",
            content="Keep the old state when the decision cannot begin its write.",
            outcome="proposed",
            source="mission-commander",
            expected_revision=1,
            expected_scope=snapshots.snapshot().conversation_scope,
        )
        queue = WorkspaceQueueService(snapshots)
        proposal = queue.propose_ad_hoc_delegation(
            correlation_id="pre-write-cut-proposal-1",
            expected_revision=1,
            source="agent-console",
            scope=snapshots.snapshot().conversation_scope,
            acceptance_criteria=["No partial approval survives the cut."],
            allowed_paths=["docs/performance.md"],
            command_policy={},
            proposed_agent="qwen-coder-local-1",
            originating_message_id=origin.message_id,
        )

        with patch.object(
            AlbertMission,
            "_persist",
            side_effect=OSError("simulated cut before first durable write"),
        ):
            with self.assertRaisesRegex(OSError, "before first durable write"):
                queue.decide(
                    correlation_id="pre-write-cut-decision-1",
                    expected_revision=proposal.revision,
                    item_id=proposal.item_id,
                    decision="approve",
                    reason="Exercise the pre-write cut.",
                )

        restored = self.load_service()
        item = WorkspaceQueueService(restored).inspect(
            item_type="ad-hoc-delegation"
        ).items[0]
        self.assertEqual(item.status, "pending")
        self.assertEqual(restored._primary_mission.sessions, {})


if __name__ == "__main__":
    unittest.main()
