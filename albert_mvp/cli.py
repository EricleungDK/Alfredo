from __future__ import annotations

import argparse
from dataclasses import asdict
import json
import sys
from pathlib import Path
from typing import Any

from .agents import AgentConfigError
from .core import AlbertError, AlbertMission, EvidencePackage, EvidenceValidationError
from .tui import build_tui_state, perform_tui_action, render_tui_state
from .workspace import (
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
    WorkingContextCurationError,
    WorkingContextService,
)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="albert", description="Local coding-agent MVP command surface.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    board = subparsers.add_parser("board", help="Render the current mission board summary.")
    _add_common_args(board)

    agents = subparsers.add_parser("agents", help="List configured Frontier and Local Agent models.")
    _add_common_args(agents)

    headless_run = subparsers.add_parser(
        "headless-run",
        help="Launch terminal-only model work through the Orchestrator and print JSON lifecycle output.",
    )
    _add_common_args(headless_run)
    headless_run.add_argument("--agent", required=True)
    headless_run.add_argument("--allowed-path", action="append", default=[])
    headless_run.add_argument("prompt")

    headless_review = subparsers.add_parser(
        "headless-review",
        help="Launch terminal-only review work through the Orchestrator and print JSON lifecycle output.",
    )
    _add_common_args(headless_review)
    headless_review.add_argument("--agent", required=True)
    headless_review.add_argument("--allowed-path", action="append", default=[])
    headless_review.add_argument("session_id", nargs="?", default="")

    tui = subparsers.add_parser("tui", help="Render the mission-control TUI surface.")
    _add_common_args(tui)
    tui.add_argument("--select", default="")

    workspace_snapshot = subparsers.add_parser(
        "workspace-snapshot",
        help="Return the versioned canonical Workspace Session snapshot as JSON.",
    )
    _add_common_args(workspace_snapshot)

    workspace_action = subparsers.add_parser(
        "workspace-action",
        help="Submit a correlated semantic Workspace Session action as JSON.",
    )
    _add_common_args(workspace_action)
    workspace_action.add_argument("--correlation-id", required=True)
    workspace_action.add_argument("--expected-revision", required=True, type=int)
    workspace_action.add_argument(
        "--operations-view",
        required=True,
        choices=["mission-board", "review-workspace", "workspace-queue", "activity"],
    )

    workspace_updates = subparsers.add_parser(
        "workspace-updates",
        help="Return ordered Workspace Session updates after a known revision as JSON.",
    )
    _add_common_args(workspace_updates)
    workspace_updates.add_argument("--after-revision", required=True, type=int)

    workspace_scope = subparsers.add_parser(
        "workspace-scope",
        help="Deliberately change Conversation Scope without granting authority.",
    )
    _add_common_args(workspace_scope)
    workspace_scope.add_argument("--correlation-id", required=True)
    workspace_scope.add_argument("--expected-revision", required=True, type=int)
    workspace_scope.add_argument(
        "--scope-kind",
        required=True,
        choices=["working-directory", "mission", "issue-slice"],
    )
    workspace_scope.add_argument("--scope-target", required=True)
    workspace_scope.add_argument("--scope-label", required=True)

    workspace_mission_switch = subparsers.add_parser(
        "workspace-mission-switch",
        help="Switch the Active Mission without stopping Background Missions.",
    )
    _add_common_args(workspace_mission_switch)
    workspace_mission_switch.add_argument("--correlation-id", required=True)
    workspace_mission_switch.add_argument("--expected-revision", required=True, type=int)
    workspace_mission_switch.add_argument("--active-mission-id", required=True)

    agent_console_message = subparsers.add_parser(
        "agent-console-message",
        help="Append one typed, scoped Agent Console message as JSON.",
    )
    _add_common_args(agent_console_message)
    agent_console_message.add_argument(
        "--role", required=True, choices=["user", "assistant", "system"]
    )
    agent_console_message.add_argument("--content", required=True)
    agent_console_message.add_argument(
        "--outcome",
        required=True,
        choices=["proposed", "pending", "acknowledged", "rejected", "model-commentary"],
    )
    agent_console_message.add_argument("--source", required=True)
    agent_console_message.add_argument("--expected-revision", required=True, type=int)
    agent_console_message.add_argument(
        "--scope-kind",
        required=True,
        choices=["working-directory", "mission", "issue-slice"],
    )
    agent_console_message.add_argument("--scope-target", required=True)
    agent_console_message.add_argument("--scope-label", required=True)

    agent_console_history = subparsers.add_parser(
        "agent-console-history",
        help="Return the continuous Agent Console history as JSON.",
    )
    _add_common_args(agent_console_history)

    working_context = subparsers.add_parser(
        "working-context",
        help="Return the bounded, reconstructable Working Context projection as JSON.",
    )
    _add_common_args(working_context)

    working_context_curate = subparsers.add_parser(
        "working-context-curate",
        help="Pin, exclude, or restore one eligible Working Context source.",
    )
    _add_common_args(working_context_curate)
    working_context_curate.add_argument("--source-id", required=True)
    working_context_curate.add_argument(
        "--disposition", required=True, choices=["included", "pinned", "excluded"]
    )
    working_context_curate.add_argument(
        "--expected-context-revision", required=True, type=int
    )

    review_workspace = subparsers.add_parser(
        "review-workspace",
        help="Return the active Mission Review Workspace projection as JSON.",
    )
    _add_common_args(review_workspace)

    activity_journal = subparsers.add_parser(
        "activity-journal",
        help="Return the searchable Activity Journal projection as JSON.",
    )
    _add_common_args(activity_journal)
    activity_journal.add_argument("--search", default="")
    activity_journal.add_argument("--activity-mission-id", default="")
    activity_journal.add_argument(
        "--actor",
        choices=["mission-commander", "orchestrator", "frontier-model", "local-agent"],
        default="",
    )
    activity_journal.add_argument("--action-type", default="")
    activity_journal.add_argument("--started-at", default="")
    activity_journal.add_argument("--ended-at", default="")

    shell_terminal = subparsers.add_parser(
        "shell-terminal",
        help="Return governed Shell Terminal metadata as JSON.",
    )
    _add_common_args(shell_terminal)

    shell_terminal_submit = subparsers.add_parser(
        "shell-terminal-submit",
        help="Submit one governed Shell Terminal command as JSON.",
    )
    _add_common_args(shell_terminal_submit)
    shell_terminal_submit.add_argument("--correlation-id", required=True)
    shell_terminal_submit.add_argument("--command-text", required=True)
    shell_terminal_submit.add_argument("--working-directory", required=True)
    shell_terminal_submit.add_argument("--requested-path", action="append", default=[])
    shell_terminal_submit.add_argument("--requester", required=True)
    shell_terminal_submit.add_argument(
        "--access-level", choices=["read", "write"], default="read"
    )

    path_grant_create = subparsers.add_parser(
        "additional-path-grant-create",
        help="Create one bounded Additional Path Grant as JSON.",
    )
    _add_common_args(path_grant_create)
    path_grant_create.add_argument("--correlation-id", required=True)
    path_grant_create.add_argument("--expected-terminal-revision", required=True, type=int)
    path_grant_create.add_argument("--path", required=True)
    path_grant_create.add_argument(
        "--access-level", required=True, choices=["read", "write"]
    )
    path_grant_create.add_argument("--duration-seconds", required=True, type=int)
    path_grant_create.add_argument("--requester", required=True)

    shell_terminal_decision = subparsers.add_parser(
        "shell-terminal-decision",
        help="Approve or deny one pending Shell Terminal command as JSON.",
    )
    _add_common_args(shell_terminal_decision)
    shell_terminal_decision.add_argument("--command-id", required=True)
    shell_terminal_decision.add_argument(
        "--decision", required=True, choices=["approve", "deny"]
    )
    shell_terminal_decision.add_argument(
        "--actor", required=True, choices=["mission-commander", "frontier-model"]
    )
    shell_terminal_decision.add_argument("--reason", default="")

    review_decision = subparsers.add_parser(
        "review-decision",
        help="Submit an acknowledged Review Workspace decision as JSON.",
    )
    _add_common_args(review_decision)
    review_decision.add_argument("--correlation-id", required=True)
    review_decision.add_argument("--expected-revision", required=True, type=int)
    review_decision.add_argument("--session-id", required=True)
    review_decision.add_argument(
        "--decision", required=True, choices=["accept", "repair", "escalate-human"]
    )
    review_decision.add_argument("--reason", default="")
    review_decision.add_argument("--failure-type", default="")

    workspace_queue = subparsers.add_parser(
        "workspace-queue",
        help="Return the Workspace Queue projection as JSON.",
    )
    _add_common_args(workspace_queue)
    workspace_queue.add_argument(
        "--item-type",
        choices=["issue-change-proposal", "frontier-confirmation", "ad-hoc-delegation"],
        default="",
    )
    workspace_queue.add_argument("--queue-mission-id", default="")

    ad_hoc_delegation_proposal = subparsers.add_parser(
        "ad-hoc-delegation-proposal",
        help="Create a pending Ad Hoc Delegation proposal in Workspace Queue.",
    )
    _add_common_args(ad_hoc_delegation_proposal)
    ad_hoc_delegation_proposal.add_argument("--correlation-id", required=True)
    ad_hoc_delegation_proposal.add_argument("--expected-revision", required=True, type=int)
    ad_hoc_delegation_proposal.add_argument("--source", required=True)
    ad_hoc_delegation_proposal.add_argument(
        "--scope-kind",
        required=True,
        choices=["working-directory", "mission", "issue-slice"],
    )
    ad_hoc_delegation_proposal.add_argument("--scope-target", required=True)
    ad_hoc_delegation_proposal.add_argument("--scope-label", required=True)
    ad_hoc_delegation_proposal.add_argument(
        "--acceptance-criterion", action="append", default=[]
    )
    ad_hoc_delegation_proposal.add_argument("--allowed-path", action="append", default=[])
    ad_hoc_delegation_proposal.add_argument("--command-policy", action="append", default=[])
    ad_hoc_delegation_proposal.add_argument("--proposed-agent", required=True)
    ad_hoc_delegation_proposal.add_argument("--originating-message-id", required=True)
    ad_hoc_delegation_proposal.add_argument("--queue-mission-id", default="")

    workspace_queue_decision = subparsers.add_parser(
        "workspace-queue-decision",
        help="Submit an acknowledged Workspace Queue decision as JSON.",
    )
    _add_common_args(workspace_queue_decision)
    workspace_queue_decision.add_argument("--correlation-id", required=True)
    workspace_queue_decision.add_argument("--expected-queue-revision", required=True, type=int)
    workspace_queue_decision.add_argument("--item-id", required=True)
    workspace_queue_decision.add_argument(
        "--decision", required=True, choices=["approve", "reject", "defer"]
    )
    workspace_queue_decision.add_argument("--reason", default="")
    workspace_queue_decision.add_argument("--action-type", default="")
    workspace_queue_decision.add_argument(
        "--actor", choices=["mission-commander"], default=""
    )
    workspace_queue_decision.add_argument("--target-kind", default="")
    workspace_queue_decision.add_argument("--target-id", default="")

    mission_drafts = subparsers.add_parser(
        "mission-drafts",
        help="Return the Mission Draft projection as JSON.",
    )
    _add_common_args(mission_drafts)
    mission_drafts.add_argument("--draft-mission-id", default="")

    mission_draft_create = subparsers.add_parser(
        "mission-draft-create",
        help="Create a proposed Mission Draft as JSON.",
    )
    _add_common_args(mission_draft_create)
    mission_draft_create.add_argument("--correlation-id", required=True)
    mission_draft_create.add_argument("--expected-revision", required=True, type=int)
    mission_draft_create.add_argument("--proposed-goal", required=True)
    mission_draft_create.add_argument("--selected-ad-hoc-id", action="append", default=[])
    mission_draft_create.add_argument("--excluded-ad-hoc-id", action="append", default=[])
    mission_draft_create.add_argument("--new-work-item", action="append", default=[])
    mission_draft_create.add_argument("--dependency", action="append", default=[])
    mission_draft_create.add_argument("--unresolved-decision", action="append", default=[])
    mission_draft_create.add_argument("--draft-mission-id", default="")

    mission_draft_update = subparsers.add_parser(
        "mission-draft-update",
        help="Revise a proposed Mission Draft as JSON.",
    )
    _add_common_args(mission_draft_update)
    mission_draft_update.add_argument("--correlation-id", required=True)
    mission_draft_update.add_argument("--expected-draft-revision", required=True, type=int)
    mission_draft_update.add_argument("--draft-id", required=True)
    mission_draft_update.add_argument("--proposed-goal", required=True)
    mission_draft_update.add_argument("--selected-ad-hoc-id", action="append", default=[])
    mission_draft_update.add_argument("--excluded-ad-hoc-id", action="append", default=[])
    mission_draft_update.add_argument("--new-work-item", action="append", default=[])
    mission_draft_update.add_argument("--dependency", action="append", default=[])
    mission_draft_update.add_argument("--unresolved-decision", action="append", default=[])

    mission_draft_confirm = subparsers.add_parser(
        "mission-draft-confirm",
        help="Confirm a Mission Draft into accepted Mission state as JSON.",
    )
    _add_common_args(mission_draft_confirm)
    mission_draft_confirm.add_argument("--correlation-id", required=True)
    mission_draft_confirm.add_argument("--expected-draft-revision", required=True, type=int)
    mission_draft_confirm.add_argument("--draft-id", required=True)
    mission_draft_confirm.add_argument("--reason", required=True)

    mission_draft_abandon = subparsers.add_parser(
        "mission-draft-abandon",
        help="Abandon a Mission Draft without changing accepted Mission state as JSON.",
    )
    _add_common_args(mission_draft_abandon)
    mission_draft_abandon.add_argument("--correlation-id", required=True)
    mission_draft_abandon.add_argument("--expected-draft-revision", required=True, type=int)
    mission_draft_abandon.add_argument("--draft-id", required=True)
    mission_draft_abandon.add_argument("--reason", required=True)

    tui_action = subparsers.add_parser("tui-action", help="Perform a mission-control TUI action.")
    _add_common_args(tui_action)
    tui_action.add_argument("action", choices=["assign", "approve", "launch", "repair", "review", "prepare-pr"])
    tui_action.add_argument("issue_id")
    tui_action.add_argument("--agent", default="")
    tui_action.add_argument("--notes", default="")
    tui_action.add_argument("--allowed-path", action="append", default=[])
    tui_action.add_argument("--session", default="")
    tui_action.add_argument("--outcome", default="")
    tui_action.add_argument("--reason", default="")
    tui_action.add_argument("--failure-type", default="")
    tui_action.add_argument("--gh-available", action="store_true")

    show = subparsers.add_parser("show", help="Show one Issue Slice lifecycle detail.")
    _add_common_args(show)
    show.add_argument("issue_id")

    approve = subparsers.add_parser("approve", help="Approve and lock an Issue Slice.")
    _add_common_args(approve)
    approve.add_argument("issue_id")

    assign = subparsers.add_parser("assign", help="Override the assigned model agent for an Issue Slice.")
    _add_common_args(assign)
    assign.add_argument("issue_id")
    assign.add_argument("--agent", required=True)
    assign.add_argument("--notes", default="")

    reopen = subparsers.add_parser("reopen", help="Explicitly reopen an Issue Slice for re-review.")
    _add_common_args(reopen)
    reopen.add_argument("issue_id")
    reopen.add_argument("--reason", required=True)

    launch = subparsers.add_parser("launch", help="Launch an approved, unblocked Issue Slice.")
    _add_common_args(launch)
    launch.add_argument("issue_id")
    launch.add_argument("--allowed-path", action="append", default=[])

    route = subparsers.add_parser("route", help="Ask the Frontier router to choose the right worker model.")
    _add_common_args(route)
    route.add_argument("issue_id")

    approve_delegation = subparsers.add_parser("approve-delegation", help="Approve a gated delegation chosen by the Frontier router.")
    _add_common_args(approve_delegation)
    approve_delegation.add_argument("issue_id")

    evidence = subparsers.add_parser("evidence", help="Record an Evidence Package for a Local Agent session.")
    _add_common_args(evidence)
    evidence.add_argument("session_id")
    evidence.add_argument("--changed-file", action="append", default=[])
    evidence.add_argument("--diff-summary", required=True)
    evidence.add_argument("--command-run", action="append", default=[])
    evidence.add_argument("--test-results", required=True)
    evidence.add_argument("--known-risks", required=True)
    evidence.add_argument("--context-updates", required=True)
    evidence.add_argument("--artifact-link", action="append", default=[])

    review = subparsers.add_parser("review", help="Record a Frontier Reviewer decision.")
    _add_common_args(review)
    review.add_argument("session_id")
    review.add_argument("--outcome", required=True)
    review.add_argument("--reason", required=True)
    review.add_argument("--failure-type", default="")

    records = subparsers.add_parser("records", help="Generate mission Markdown records.")
    _add_common_args(records)

    pr = subparsers.add_parser("pr", help="Prepare PR-ready summary or GitHub command.")
    _add_common_args(pr)
    pr.add_argument("issue_id")
    pr.add_argument("--gh-available", action="store_true")

    return parser


def _add_common_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--target-repo", required=True)
    parser.add_argument("--tracker-dir", required=True)
    parser.add_argument("--issues-dir", default="")
    parser.add_argument("--runtime-root", required=True)
    parser.add_argument("--mission-id", default="mission-001")
    parser.add_argument("--agent-config", default="")
    parser.add_argument("--mission-catalog", default="")


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return _run(args)
    except WorkspaceStaleActionError as exc:
        print(
            json.dumps(
                {
                    "error": {
                        "code": exc.code,
                        "message": str(exc),
                        "recoverable": True,
                        "expected_revision": exc.expected_revision,
                        "current_revision": exc.current_revision,
                    }
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1
    except WorkspaceRevisionGapError as exc:
        print(
            json.dumps(
                {
                    "error": {
                        "code": exc.code,
                        "message": str(exc),
                        "recoverable": True,
                        "requested_revision": exc.requested_revision,
                        "current_revision": exc.current_revision,
                    }
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1
    except WorkspaceScopeMismatchError as exc:
        print(
            json.dumps(
                {
                    "error": {
                        "code": exc.code,
                        "message": str(exc),
                        "recoverable": True,
                        "expected_scope": asdict(exc.expected_scope),
                        "current_scope": asdict(exc.current_scope),
                    }
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1
    except WorkspacePersistenceError as exc:
        print(
            json.dumps(
                {
                    "error": {
                        "code": exc.code,
                        "message": str(exc),
                        "recoverable": True,
                    }
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1
    except WorkingContextCurationError as exc:
        print(
            json.dumps(
                {
                    "error": {
                        "code": exc.code,
                        "message": str(exc),
                        "recoverable": True,
                        "source_id": exc.source_id,
                    }
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1
    except EvidenceValidationError as exc:
        print(
            json.dumps(
                {
                    "error": {
                        "code": "evidence-incomplete",
                        "message": str(exc),
                        "recoverable": True,
                    }
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1
    except (AlbertError, AgentConfigError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1


def _run(args: argparse.Namespace) -> int:
    mission = AlbertMission(
        target_repo=Path(args.target_repo),
        tracker_dir=Path(args.tracker_dir),
        runtime_root=Path(args.runtime_root),
        mission_id=args.mission_id,
        agent_config_path=Path(args.agent_config) if args.agent_config else None,
        allow_empty_tracker=args.command
        in {
            "agents",
            "headless-run",
            "headless-review",
            "workspace-snapshot",
            "workspace-action",
            "workspace-updates",
            "workspace-scope",
            "workspace-mission-switch",
            "agent-console-message",
            "agent-console-history",
            "working-context",
            "working-context-curate",
            "review-workspace",
            "activity-journal",
            "shell-terminal",
            "shell-terminal-submit",
            "additional-path-grant-create",
            "shell-terminal-decision",
            "review-decision",
            "workspace-queue",
            "ad-hoc-delegation-proposal",
            "workspace-queue-decision",
            "mission-drafts",
            "mission-draft-create",
            "mission-draft-update",
            "mission-draft-confirm",
            "mission-draft-abandon",
        },
        issues_dir=Path(args.issues_dir) if args.issues_dir else None,
    ).load()
    workspace_commands = {
        "workspace-snapshot",
        "workspace-action",
        "workspace-updates",
        "workspace-scope",
        "workspace-mission-switch",
        "agent-console-message",
        "agent-console-history",
        "working-context",
        "working-context-curate",
        "review-workspace",
        "activity-journal",
        "shell-terminal",
        "shell-terminal-submit",
        "additional-path-grant-create",
        "shell-terminal-decision",
        "review-decision",
        "workspace-queue",
        "ad-hoc-delegation-proposal",
        "workspace-queue-decision",
        "mission-drafts",
        "mission-draft-create",
        "mission-draft-update",
        "mission-draft-confirm",
        "mission-draft-abandon",
    }
    snapshots = (
        _load_workspace_service(args, mission) if args.command in workspace_commands else None
    )
    if args.command == "board":
        summary = mission.board_summary()
        print(summary["prd_title"])
        print(f"Issues: {summary['issue_count']}")
        print(f"Ordered: {', '.join(summary['ordered_issue_ids']) or 'None'}")
        print(f"Ready: {', '.join(summary['ready_issue_ids']) or 'None'}")
        for issue_id in summary["ordered_issue_ids"]:
            issue = mission.issues[issue_id]
            blockers = ", ".join(issue.blocked_by) if issue.blocked_by else "None"
            print(f"- {issue.id}: {issue.title} [{issue.review_state}] blockers: {blockers}")
        return 0
    if args.command == "agents":
        agents = mission.list_agents()
        if not agents:
            print("No agent registry configured; issue metadata assignments are allowed.")
            return 0
        for agent in agents:
            print(f"{agent.id}\t{agent.role}\t{agent.runner}\t{agent.summary()}")
        return 0
    if args.command == "headless-run":
        session = mission.launch_headless_work(
            work_kind="run",
            agent_id=args.agent,
            prompt=args.prompt,
            allowed_paths=args.allowed_path,
        )
        print(json.dumps(_headless_session_payload(mission, session, launch="headless-run"), sort_keys=True))
        return 0
    if args.command == "headless-review":
        session = mission.launch_headless_work(
            work_kind="review",
            agent_id=args.agent,
            review_session_id=args.session_id,
            allowed_paths=args.allowed_path,
        )
        print(json.dumps(_headless_session_payload(mission, session, launch="headless-review"), sort_keys=True))
        return 0
    if args.command == "tui":
        print(render_tui_state(build_tui_state(mission, selected_issue_id=args.select)), end="")
        return 0
    if args.command == "workspace-snapshot":
        snapshot = snapshots.snapshot()
        print(json.dumps(snapshot.to_dict(), sort_keys=True))
        return 0
    if args.command == "workspace-action":
        current = snapshots.snapshot()
        acknowledgement = WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id=args.correlation_id,
                expected_revision=args.expected_revision,
                active_mission_id=mission.mission_id,
                conversation_scope=current.conversation_scope,
                operations_view=args.operations_view,
            )
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "workspace-updates":
        batch = WorkspaceSyncService(snapshots).updates_after(args.after_revision)
        print(json.dumps(asdict(batch), sort_keys=True))
        return 0
    if args.command == "workspace-scope":
        current = snapshots.snapshot()
        acknowledgement = WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id=args.correlation_id,
                expected_revision=args.expected_revision,
                active_mission_id=mission.mission_id,
                conversation_scope=ConversationScope(
                    kind=args.scope_kind,
                    target_id=args.scope_target,
                    label=args.scope_label,
                ),
                operations_view=current.operations_view,
            )
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "workspace-mission-switch":
        current = snapshots.snapshot()
        acknowledgement = WorkspaceSyncService(snapshots).submit_action(
            WorkspaceAction(
                correlation_id=args.correlation_id,
                expected_revision=args.expected_revision,
                active_mission_id=args.active_mission_id,
                conversation_scope=current.conversation_scope,
                operations_view=current.operations_view,
            )
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "agent-console-message":
        message = AgentConsoleHistoryService(snapshots).append(
            role=args.role,
            content=args.content,
            outcome=args.outcome,
            source=args.source,
            expected_revision=args.expected_revision,
            expected_scope=ConversationScope(
                kind=args.scope_kind,
                target_id=args.scope_target,
                label=args.scope_label,
            ),
        )
        print(json.dumps(asdict(message), sort_keys=True))
        return 0
    if args.command == "agent-console-history":
        messages = AgentConsoleHistoryService(snapshots).history()
        print(
            json.dumps(
                {"schema_version": 1, "messages": [asdict(message) for message in messages]},
                sort_keys=True,
            )
        )
        return 0
    if args.command == "working-context":
        projection = WorkingContextService(snapshots).inspect()
        print(json.dumps(asdict(projection), sort_keys=True))
        return 0
    if args.command == "working-context-curate":
        acknowledgement = WorkingContextService(snapshots).curate(
            source_id=args.source_id,
            disposition=args.disposition,
            expected_revision=args.expected_context_revision,
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "review-workspace":
        projection = ReviewWorkspaceService(snapshots).inspect()
        print(json.dumps(asdict(projection), sort_keys=True))
        return 0
    if args.command == "activity-journal":
        projection = ActivityJournalService(snapshots).inspect(
            search=args.search,
            mission_id=args.activity_mission_id,
            actor=args.actor,
            action_type=args.action_type,
            started_at=args.started_at,
            ended_at=args.ended_at,
        )
        print(json.dumps(asdict(projection), sort_keys=True))
        return 0
    if args.command == "shell-terminal":
        projection = ShellTerminalService(snapshots).inspect()
        print(json.dumps(asdict(projection), sort_keys=True))
        return 0
    if args.command == "shell-terminal-submit":
        result = ShellTerminalService(snapshots).submit(
            correlation_id=args.correlation_id,
            command=args.command_text,
            working_directory=args.working_directory,
            requested_paths=args.requested_path,
            requester=args.requester,
            access_level=args.access_level,
        )
        print(json.dumps(asdict(result), sort_keys=True))
        return 0
    if args.command == "additional-path-grant-create":
        grant = ShellTerminalService(snapshots).create_path_grant(
            correlation_id=args.correlation_id,
            expected_revision=args.expected_terminal_revision,
            path=args.path,
            access_level=args.access_level,
            duration_seconds=args.duration_seconds,
            requester=args.requester,
        )
        print(json.dumps(asdict(grant), sort_keys=True))
        return 0
    if args.command == "shell-terminal-decision":
        terminal = ShellTerminalService(snapshots)
        result = (
            terminal.approve(command_id=args.command_id, approver=args.actor)
            if args.decision == "approve"
            else terminal.deny(
                command_id=args.command_id,
                decider=args.actor,
                reason=args.reason,
            )
        )
        print(json.dumps(asdict(result), sort_keys=True))
        return 0
    if args.command == "review-decision":
        acknowledgement = ReviewWorkspaceService(snapshots).decide(
            correlation_id=args.correlation_id,
            expected_revision=args.expected_revision,
            session_id=args.session_id,
            decision=args.decision,
            reason=args.reason,
            failure_type=args.failure_type,
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "workspace-queue":
        projection = WorkspaceQueueService(snapshots).inspect(
            item_type=args.item_type or None,
            mission_id=args.queue_mission_id or None,
        )
        print(json.dumps(asdict(projection), sort_keys=True))
        return 0
    if args.command == "ad-hoc-delegation-proposal":
        acknowledgement = WorkspaceQueueService(snapshots).propose_ad_hoc_delegation(
            correlation_id=args.correlation_id,
            expected_revision=args.expected_revision,
            source=args.source,
            scope=ConversationScope(
                kind=args.scope_kind,
                target_id=args.scope_target,
                label=args.scope_label,
            ),
            acceptance_criteria=args.acceptance_criterion,
            allowed_paths=args.allowed_path,
            command_policy=_parse_command_policy(args.command_policy),
            proposed_agent=args.proposed_agent,
            originating_message_id=args.originating_message_id,
            mission_id=args.queue_mission_id or None,
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "workspace-queue-decision":
        acknowledgement = WorkspaceQueueService(snapshots).decide(
            correlation_id=args.correlation_id,
            expected_revision=args.expected_queue_revision,
            item_id=args.item_id,
            decision=args.decision,
            reason=args.reason,
            action_type=args.action_type,
            actor=args.actor,
            target_kind=args.target_kind,
            target_id=args.target_id,
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "mission-drafts":
        projection = MissionDraftService(snapshots).inspect(
            mission_id=args.draft_mission_id or None,
        )
        print(json.dumps(asdict(projection), sort_keys=True))
        return 0
    if args.command == "mission-draft-create":
        acknowledgement = MissionDraftService(snapshots).create_draft(
            correlation_id=args.correlation_id,
            expected_revision=args.expected_revision,
            proposed_goal=args.proposed_goal,
            selected_ad_hoc_ids=args.selected_ad_hoc_id,
            excluded_ad_hoc_ids=args.excluded_ad_hoc_id,
            new_work_items=args.new_work_item,
            dependencies=args.dependency,
            unresolved_decisions=args.unresolved_decision,
            mission_id=args.draft_mission_id or None,
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "mission-draft-update":
        acknowledgement = MissionDraftService(snapshots).update_draft(
            correlation_id=args.correlation_id,
            expected_revision=args.expected_draft_revision,
            draft_id=args.draft_id,
            proposed_goal=args.proposed_goal,
            selected_ad_hoc_ids=args.selected_ad_hoc_id,
            excluded_ad_hoc_ids=args.excluded_ad_hoc_id,
            new_work_items=args.new_work_item,
            dependencies=args.dependency,
            unresolved_decisions=args.unresolved_decision,
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "mission-draft-confirm":
        acknowledgement = MissionDraftService(snapshots).confirm_draft(
            correlation_id=args.correlation_id,
            expected_revision=args.expected_draft_revision,
            draft_id=args.draft_id,
            reason=args.reason,
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "mission-draft-abandon":
        acknowledgement = MissionDraftService(snapshots).abandon_draft(
            correlation_id=args.correlation_id,
            expected_revision=args.expected_draft_revision,
            draft_id=args.draft_id,
            reason=args.reason,
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "tui-action":
        result = perform_tui_action(
            mission,
            args.action,
            args.issue_id,
            agent_id=args.agent,
            notes=args.notes,
            allowed_paths=args.allowed_path,
            session_id=args.session,
            outcome=args.outcome,
            reason=args.reason,
            failure_type=args.failure_type,
            gh_available=args.gh_available,
        )
        print(result.message)
        if result.session_id:
            print(result.session_id)
        if result.create_command:
            print(result.create_command)
        if result.body:
            print(result.body)
        return 0
    if args.command == "show":
        detail = mission.issue_detail(args.issue_id)
        print(f"{detail['issue_id']}: {detail['title']}")
        print(f"Tracker status: {detail['tracker_status']}")
        print(f"Runtime status: {detail['runtime_status']}")
        print(f"Review state: {detail['review_state']}")
        print(f"Assigned agent: {detail['assigned_agent']}")
        if detail["notes"]:
            print(f"Notes: {detail['notes']}")
        if detail["blockers"]:
            print("Blockers:")
            for blocker in detail["blockers"]:
                marker = "satisfied" if blocker["satisfied"] else "waiting"
                print(f"- {blocker['issue_id']}: {blocker['review_state']} ({marker})")
        else:
            print("Blockers: None")
        print(f"Next actions: {', '.join(detail['next_actions'])}")
        return 0
    if args.command == "approve":
        mission.approve_issue(args.issue_id)
        print(f"{args.issue_id} approved and locked.")
        return 0
    if args.command == "assign":
        mission.assign_issue(args.issue_id, args.agent, notes=args.notes)
        print(f"{args.issue_id} assigned to {args.agent}.")
        return 0
    if args.command == "reopen":
        mission.reopen_issue(args.issue_id, reason=args.reason)
        print(f"{args.issue_id} reopened for re-review.")
        return 0
    if args.command == "launch":
        session = mission.launch_issue(args.issue_id, allowed_paths=args.allowed_path)
        print(f"Launched {session.issue_id} as {session.session_id}")
        print(f"Worktree: {session.worktree_path}")
        return 0
    if args.command == "route":
        decision = mission.route_issue(args.issue_id)
        suffix = " (delegation approval required)" if decision.requires_approval and not decision.approved else ""
        print(
            f"{decision.issue_id} routed by {decision.router_agent} to {decision.recommended_agent} "
            f"[{decision.complexity}]: {decision.reason}{suffix}"
        )
        return 0
    if args.command == "approve-delegation":
        decision = mission.approve_delegation(args.issue_id)
        print(f"{decision.issue_id} delegation approved for {decision.recommended_agent}.")
        return 0
    if args.command == "evidence":
        evidence = EvidencePackage(
            changed_files=args.changed_file,
            diff_summary=args.diff_summary,
            commands_run=args.command_run,
            test_results=args.test_results,
            known_risks=args.known_risks,
            proposed_context_updates=args.context_updates,
            artifact_links=args.artifact_link,
        )
        mission.record_evidence(args.session_id, evidence)
        print(f"Evidence Package validated for {args.session_id}.")
        return 0
    if args.command == "review":
        decision = mission.record_frontier_review(
            args.session_id,
            args.outcome,
            reason=args.reason,
            failure_type=args.failure_type,
        )
        print(f"{decision.issue_id} review: {decision.outcome}; next action {decision.next_action}.")
        return 0
    if args.command == "records":
        records = mission.generate_mission_records()
        print(f"Mission records written to {records}")
        return 0
    if args.command == "pr":
        pr = mission.prepare_pr(args.issue_id, gh_available=args.gh_available)
        print(f"Branch: {pr.branch_name}")
        if pr.create_command:
            print(pr.create_command)
        print(pr.body)
        return 0
    return 1


def _parse_command_policy(entries: list[str]) -> dict[str, str]:
    policy: dict[str, str] = {}
    for entry in entries:
        command, separator, level = entry.partition("=")
        if not separator or not command.strip() or not level.strip():
            raise AlbertError("Command policy entries must use command=policy format.")
        policy[command.strip()] = level.strip()
    return policy


def _headless_session_payload(
    mission: AlbertMission,
    session: Any,
    *,
    launch: str,
) -> dict[str, Any]:
    agent = mission.agent_registry.require(session.assigned_agent)
    runner_command = mission._runner_command(agent)
    command_policy = mission.classify_command(runner_command) if runner_command else "not_applicable"
    evidence = session.evidence.to_dict() if session.evidence else None
    return {
        "product": "Alfredo",
        "launch": launch,
        "session_id": session.session_id,
        "work_id": session.issue_id,
        "status": session.status,
        "selected_agent": agent.id,
        "selected_model": agent.model,
        "agent": agent.to_dict(),
        "prompt": session.task_packet.get("prompt", ""),
        "review_session_id": session.task_packet.get("review_session_id", ""),
        "worktree_path": str(session.worktree_path),
        "allowed_paths": list(session.task_packet.get("allowed_paths", [])),
        "review_context": session.task_packet.get("review_context"),
        "governance": {
            "orchestrator": "AlbertMission",
            "command_policy": command_policy,
            "path_boundary": "allowed_paths",
            "model_assignment": agent.id,
            "evidence_package": "valid" if session.evidence_valid else "pending",
        },
        "lifecycle": [
            "agent-resolved",
            f"command-policy:{command_policy}",
            "session-created",
            f"runner-status:{session.status}",
            "evidence-package:valid" if session.evidence_valid else "evidence-package:pending",
        ],
        "evidence": evidence,
    }


def _load_workspace_service(
    args: argparse.Namespace, primary: AlbertMission
) -> WorkspaceSnapshotService:
    if not args.mission_catalog:
        return WorkspaceSnapshotService(primary)
    catalog_path = Path(args.mission_catalog).resolve()
    try:
        payload = json.loads(catalog_path.read_text(encoding="utf-8"))
        if payload["schema_version"] != 1 or not isinstance(payload["missions"], list):
            raise ValueError("unsupported mission catalog schema")
        missions = []
        for item in payload["missions"]:
            mission_id = item["mission_id"]
            tracker_dir = Path(item["tracker_dir"])
            if not tracker_dir.is_absolute():
                tracker_dir = catalog_path.parent / tracker_dir
            issues_value = item.get("issues_dir", "")
            issues_dir = Path(issues_value) if issues_value else None
            if issues_dir is not None and not issues_dir.is_absolute():
                issues_dir = catalog_path.parent / issues_dir
            if not isinstance(mission_id, str) or not mission_id.strip():
                raise ValueError("mission id must not be empty")
            missions.append(
                AlbertMission(
                    target_repo=primary.target_repo,
                    tracker_dir=tracker_dir,
                    issues_dir=issues_dir,
                    runtime_root=primary.runtime_root,
                    mission_id=mission_id,
                    agent_config_path=primary.agent_config_path,
                    allow_empty_tracker=True,
                ).load()
            )
        return WorkspaceSnapshotService(primary, missions=tuple(missions))
    except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
        raise AlbertError(f"Mission catalog could not be loaded: {exc}") from exc
