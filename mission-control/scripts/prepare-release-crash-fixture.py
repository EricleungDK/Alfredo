#!/usr/bin/env python3
"""Prepare the exact pre-effect dead-owner boundary used by the release seam."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from albert_mvp.core import AlbertMission


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target-repo", required=True)
    parser.add_argument("--tracker-dir", required=True)
    parser.add_argument("--issues-dir", required=True)
    parser.add_argument("--runtime-root", required=True)
    parser.add_argument("--mission-id", required=True)
    parser.add_argument("--agent-config", required=True)
    parser.add_argument("--session-id", required=True)
    args = parser.parse_args()

    mission = AlbertMission(
        target_repo=Path(args.target_repo),
        tracker_dir=Path(args.tracker_dir),
        issues_dir=Path(args.issues_dir),
        runtime_root=Path(args.runtime_root),
        mission_id=args.mission_id,
        agent_config_path=Path(args.agent_config),
    ).load(perform_startup_effects=False)
    session = mission.sessions[args.session_id]
    if session.status != "queued":
        raise RuntimeError("release crash fixture requires a queued session")

    session.status = "running"
    session.runner_started_at = "2026-08-30T00:00:00+00:00"
    session.runner_pid = 2_000_000_001
    session.runner_identity = "release-fixture-dead-owner"
    session.runner_process_pid = 2_000_000_002
    session.runner_process_identity = "release-fixture-dead-process-group"
    session.runner_operation_id = f"runner-operation:release-fixture:{session.session_id}"
    mission._persist_session_update(
        session,
        expected_statuses={"queued"},
    )
    mission._ensure_session_worktree(session)
    session.worktree_identity = mission.current_worktree_identity(session.session_id)
    persisted = mission._persist_session_update(
        session,
        expected_statuses={"running"},
        timeline_message=(
            f"{session.issue_id} controlled pre-effect crash boundary recorded for "
            f"{session.session_id}."
        ),
    )
    print(
        json.dumps(
            {
                "session_id": persisted.session_id,
                "revision": persisted.revision,
                "runner_operation_id": persisted.runner_operation_id,
                "runner_process_pid": persisted.runner_process_pid,
                "worktree_identity": persisted.worktree_identity,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
