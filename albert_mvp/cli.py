from __future__ import annotations

import argparse
from dataclasses import asdict
import json
import os
import shlex
import sys
import time
from pathlib import Path
from typing import Any

from .agents import AgentConfigError, load_agent_registry
from .capabilities import (
    CapabilityCatalogService,
    OllamaHealthProbe,
    OllamaHealthSnapshot,
)
from .core import (
    AlbertError,
    AlbertMission,
    EvidencePackage,
    EvidenceValidationError,
    RunnerObservation,
    SharedUnderstandingGateError,
    WayfinderStatePersistenceError,
)
from .tui import build_tui_state, perform_tui_action, render_tui_state
from .workspace import (
    AgentConsoleHistoryService,
    AgentConsoleResponseService,
    ActivityJournalService,
    ConversationScope,
    MissionDraftService,
    ReviewWorkspaceService,
    SessionArtifactReadError,
    SessionArtifactService,
    SessionOutputReadError,
    SessionOutputService,
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
from .workspace_selection import (
    CodingWorkspaceSelectionError,
    CodingWorkspaceSelectionService,
    MissionChoiceError,
    WorkspaceJourneyStore,
)


_OLLAMA_HEALTH_CACHE_SECONDS = 2.0
_ollama_health_cache: dict[str, tuple[float, OllamaHealthSnapshot]] = {}


def _cached_ollama_health() -> OllamaHealthSnapshot:
    cache_key = os.environ.get("OLLAMA_HOST", "").strip()
    now = time.monotonic()
    cached = _ollama_health_cache.get(cache_key)
    if cached is not None and now - cached[0] < _OLLAMA_HEALTH_CACHE_SECONDS:
        return cached[1]
    health = OllamaHealthProbe()()
    _ollama_health_cache[cache_key] = (now, health)
    return health


def _live_agent_availability(
    *,
    workspace_root: Path,
    registry_path: Path,
) -> dict[str, tuple[str, str]]:
    registry = load_agent_registry(registry_path)
    agents = CapabilityCatalogService(
        workspace_root=workspace_root,
        agent_registry=registry,
        ollama_probe=_cached_ollama_health,
    ).agent_availability()
    return {
        agent.id: (agent.availability, agent.availability_reason)
        for agent in agents
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="albert", description="Local coding-agent MVP command surface.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    board = subparsers.add_parser("board", help="Render the current mission board summary.")
    _add_common_args(board)

    agents = subparsers.add_parser("agents", help="List configured Frontier and Local Agent models.")
    _add_common_args(agents)

    agent_capabilities = subparsers.add_parser(
        "agent-capabilities",
        help="Return installed skills, slash commands, and configured agents as JSON.",
    )
    _add_common_args(agent_capabilities, require_mission_state=False)
    agent_capabilities.add_argument("--skill-root", action="append", default=[])
    agent_capabilities.add_argument("--global-skill-root", action="append", default=None)

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

    coding_workspace_select = subparsers.add_parser(
        "coding-workspace-select",
        help="Validate and acknowledge an exact Coding Workspace repository.",
    )
    coding_workspace_select.add_argument("--starting-location", required=True)
    coding_workspace_select.add_argument("--workspace-path", required=True)
    coding_workspace_select.add_argument(
        "--selection-mode", required=True, choices=["existing", "create"]
    )
    coding_workspace_select.add_argument("--runtime-root", required=True)
    coding_workspace_select.add_argument("--correlation-id", required=True)
    coding_workspace_select.add_argument("--forbidden-root", action="append", default=[])

    mission_options = subparsers.add_parser(
        "mission-options",
        help="Return explicit Resume Mission and Start New Mission choices as JSON.",
    )
    mission_options.add_argument("--starting-location", required=True)
    mission_options.add_argument("--coding-workspace", required=True)
    mission_options.add_argument("--runtime-root", required=True)

    mission_choice = subparsers.add_parser(
        "mission-choice",
        help="Resume an exact Mission or create a distinct new Mission as JSON.",
    )
    mission_choice.add_argument("--starting-location", required=True)
    mission_choice.add_argument("--coding-workspace", required=True)
    mission_choice.add_argument("--runtime-root", required=True)
    mission_choice.add_argument("--correlation-id", required=True)
    mission_choice.add_argument("--expected-revision", required=True, type=int)
    mission_choice.add_argument("--choice", required=True, choices=["resume", "new"])
    mission_choice.add_argument("--mission-id", required=True)
    mission_choice.add_argument("--mission-title", default="")

    workspace_context = subparsers.add_parser(
        "workspace-context",
        help="Restore the canonical Workspace and Mission journey as JSON.",
    )
    workspace_context.add_argument("--starting-location", required=True)
    workspace_context.add_argument("--runtime-root", required=True)

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
        "--action-type",
        default="conversation-scope-change",
        choices=["conversation-scope-change"],
    )
    workspace_scope.add_argument(
        "--actor",
        default="mission-commander",
        choices=["mission-commander"],
    )
    workspace_scope.add_argument(
        "--target-kind",
        default="conversation-scope",
        choices=["conversation-scope"],
    )
    workspace_scope.add_argument("--target-id", default="")
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
    agent_console_message.add_argument("--scope-mission-id", default="")

    agent_console_history = subparsers.add_parser(
        "agent-console-history",
        help="Return the continuous Agent Console history as JSON.",
    )
    _add_common_args(agent_console_history)

    agent_console_response = subparsers.add_parser(
        "agent-console-response",
        help="Generate and append a controller response to one correlated Agent Console prompt.",
    )
    _add_common_args(agent_console_response)
    agent_console_response.add_argument("--message-id", required=True)
    agent_console_response.add_argument("--expected-revision", required=True, type=int)
    agent_console_response.add_argument(
        "--scope-kind",
        required=True,
        choices=["working-directory", "mission", "issue-slice"],
    )
    agent_console_response.add_argument("--scope-target", required=True)
    agent_console_response.add_argument("--scope-label", required=True)
    agent_console_response.add_argument("--scope-mission-id", default="")
    agent_console_response.add_argument("--agent-id", default="")

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

    session_artifact = subparsers.add_parser(
        "session-artifact",
        help="Read one registered review-safe Local Agent artifact as bounded JSON text.",
    )
    _add_common_args(session_artifact)
    session_artifact.add_argument("--artifact-mission-id", required=True)
    session_artifact.add_argument("--session-id", required=True)
    session_artifact.add_argument("--artifact-ref", required=True)

    session_output = subparsers.add_parser(
        "session-output",
        help="Read bounded live output for one exact Local Agent session.",
    )
    _add_common_args(session_output)
    session_output.add_argument("--output-mission-id", required=True)
    session_output.add_argument("--session-id", required=True)
    session_output.add_argument("--after-sequence", type=int, default=0)

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
    path_grant_create.add_argument("--request-id", default="")
    path_grant_create.add_argument("--expected-terminal-revision", required=True, type=int)
    path_grant_create.add_argument("--path", required=True)
    path_grant_create.add_argument(
        "--access-level", required=True, choices=["read", "write"]
    )
    path_grant_create.add_argument("--duration-seconds", required=True, type=int)
    path_grant_create.add_argument("--requester", required=True)

    path_grant_deny = subparsers.add_parser(
        "additional-path-grant-deny",
        help="Deny one contextual Additional Path Grant request as JSON.",
    )
    _add_common_args(path_grant_deny)
    path_grant_deny.add_argument("--correlation-id", required=True)
    path_grant_deny.add_argument("--request-id", required=True)
    path_grant_deny.add_argument("--expected-terminal-revision", required=True, type=int)
    path_grant_deny.add_argument("--path", required=True)
    path_grant_deny.add_argument(
        "--access-level", required=True, choices=["read", "write"]
    )
    path_grant_deny.add_argument("--duration-seconds", required=True, type=int)
    path_grant_deny.add_argument("--requester", required=True)
    path_grant_deny.add_argument("--reason", required=True)
    path_grant_deny.add_argument("--affected-action", required=True)

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
    review_decision.add_argument(
        "--action-type", required=True, choices=["review-decision"]
    )
    review_decision.add_argument(
        "--actor", required=True, choices=["mission-commander"]
    )
    review_decision.add_argument(
        "--target-kind", required=True, choices=["agent-session"]
    )
    review_decision.add_argument("--target-id", required=True)
    review_decision.add_argument("--session-id", required=True)
    review_decision.add_argument("--review-mission-id", default="")
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

    workstation_action = subparsers.add_parser(
        "workstation-action",
        help="Submit a typed Agent Workstation action as JSON.",
    )
    _add_common_args(workstation_action)
    workstation_action.add_argument("--correlation-id", required=True)
    workstation_action.add_argument("--expected-revision", required=True, type=int)
    workstation_action.add_argument(
        "--action-type",
        required=True,
        choices=[
            "issue-approve",
            "issue-launch",
            "issue-retry",
            "session-cancel",
            "model-assignment-change",
            "issue-archive",
            "issue-restore",
        ],
    )
    workstation_action.add_argument(
        "--actor", required=True, choices=["mission-commander"]
    )
    workstation_action.add_argument(
        "--target-kind", required=True, choices=["issue-slice", "agent-session"]
    )
    workstation_action.add_argument("--target-id", required=True)
    workstation_action.add_argument("--action-mission-id", default="")
    workstation_action.add_argument("--issue-id", default="")
    workstation_action.add_argument("--session-id", default="")
    workstation_action.add_argument("--agent", default="")
    workstation_action.add_argument("--reason", default="")
    workstation_action.add_argument("--allowed-path", action="append", default=[])
    workstation_action.add_argument("--command-policy", action="append", default=[])

    workstation_session_run = subparsers.add_parser(
        "workstation-session-run",
        help="Execute one persisted queued Local Agent session and return its lifecycle as JSON.",
    )
    _add_common_args(workstation_session_run)
    workstation_session_run.add_argument("--session-id", required=True)
    workstation_session_run.add_argument("--session-mission-id", default="")

    runner_observe = subparsers.add_parser(
        "runner-observe",
        help="Reconcile one advisory Local Agent runner observation and return its receipt.",
    )
    _add_common_args(runner_observe)
    runner_observe.add_argument("--source-id", required=True)
    runner_observe.add_argument("--source-incarnation", required=True)
    runner_observe.add_argument("--sequence", required=True, type=int)
    runner_observe.add_argument("--observation-mission-id", required=True)
    runner_observe.add_argument("--session-id", required=True)
    runner_observe.add_argument("--session-revision", required=True, type=int)
    runner_observe.add_argument("--runner-operation-id", required=True)
    runner_observe.add_argument(
        "--owner-signal",
        required=True,
        choices=["live-exact", "absent", "reused", "unavailable"],
    )
    runner_observe.add_argument(
        "--process-group-signal",
        required=True,
        choices=["live-exact", "absent", "reused", "unavailable"],
    )
    runner_observe.add_argument("--worktree-identity", required=True)
    runner_observe.add_argument(
        "--result-signal",
        required=True,
        choices=["absent", "exact-valid", "invalid", "unavailable"],
    )
    runner_observe.add_argument("--result-digest", default="")

    retirement_preserve = subparsers.add_parser(
        "retirement-preserve",
        help="Capture and verify one terminal Retirement Unit without removing its worktree.",
    )
    _add_common_args(retirement_preserve)
    retirement_preserve.add_argument("--session-id", required=True)
    retirement_preserve.add_argument("--session-mission-id", default="")
    retirement_preserve.add_argument("--expected-revision", required=True, type=int)
    retirement_preserve.add_argument("--correlation-id", required=True)

    retirement_verify = subparsers.add_parser(
        "retirement-verify",
        help="Re-read and reconstruct one verified Retirement Snapshot.",
    )
    _add_common_args(retirement_verify)
    retirement_verify.add_argument("--session-id", required=True)
    retirement_verify.add_argument("--session-mission-id", default="")

    retirement_storage = subparsers.add_parser(
        "retirement-storage",
        help="Return deterministic Snapshot Payload usage and policy inspection as JSON.",
    )
    _add_common_args(retirement_storage)

    retirement_inspect = subparsers.add_parser(
        "retirement-inspect",
        help="Inspect one Retirement Unit and its available governed actions as JSON.",
    )
    _add_common_args(retirement_inspect)
    retirement_inspect.add_argument("--session-id", required=True)
    retirement_inspect.add_argument("--session-mission-id", default="")

    retirement_pin = subparsers.add_parser(
        "retirement-pin",
        help="Pin or unpin one retained Snapshot Payload.",
    )
    _add_common_args(retirement_pin)
    retirement_pin.add_argument("--session-id", required=True)
    retirement_pin.add_argument("--session-mission-id", default="")
    retirement_pin.add_argument(
        "--pin-state", required=True, choices=["pinned", "unpinned"]
    )
    retirement_pin.add_argument("--expected-revision", required=True, type=int)
    retirement_pin.add_argument("--correlation-id", required=True)

    retirement_retry = subparsers.add_parser(
        "retirement-retry",
        help="Authorize one fresh bounded attempt for a blocked Retirement Unit.",
    )
    _add_common_args(retirement_retry)
    retirement_retry.add_argument("--session-id", required=True)
    retirement_retry.add_argument("--session-mission-id", default="")
    retirement_retry.add_argument("--expected-revision", required=True, type=int)
    retirement_retry.add_argument("--correlation-id", required=True)

    retirement_export = subparsers.add_parser(
        "retirement-export",
        help="Export one blocked Retirement Unit from verified preserved material.",
    )
    _add_common_args(retirement_export)
    retirement_export.add_argument("--session-id", required=True)
    retirement_export.add_argument("--session-mission-id", default="")
    retirement_export.add_argument("--destination", required=True)
    retirement_export.add_argument("--expected-revision", required=True, type=int)
    retirement_export.add_argument("--correlation-id", required=True)

    retirement_discard = subparsers.add_parser(
        "retirement-discard",
        help="Irreversibly discard one exact blocked and quiesced retained worktree.",
    )
    _add_common_args(retirement_discard)
    retirement_discard.add_argument("--session-id", required=True)
    retirement_discard.add_argument("--session-mission-id", default="")
    retirement_discard.add_argument("--expected-revision", required=True, type=int)
    retirement_discard.add_argument("--correlation-id", required=True)
    retirement_discard.add_argument("--confirmation", required=True)
    retirement_discard.add_argument("--reason", required=True)

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
    tui_action.add_argument("--expected-session-revision", type=int)
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
    evidence.add_argument("--expected-revision", required=True, type=int)
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
    review.add_argument("--expected-revision", required=True, type=int)
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


def _add_common_args(
    parser: argparse.ArgumentParser, *, require_mission_state: bool = True
) -> None:
    parser.add_argument("--target-repo", required=True)
    parser.add_argument("--tracker-dir", required=require_mission_state, default="")
    parser.add_argument("--issues-dir", default="")
    parser.add_argument("--runtime-root", required=require_mission_state, default="")
    parser.add_argument("--mission-id", default="mission-001")
    parser.add_argument("--agent-config", default="")
    parser.add_argument("--mission-catalog", default="")
    parser.add_argument(
        "--retention-grace-seconds",
        type=int,
        default=72 * 60 * 60,
    )
    parser.add_argument(
        "--snapshot-storage-retention-seconds",
        type=int,
        default=30 * 24 * 60 * 60,
    )
    parser.add_argument(
        "--snapshot-storage-budget-bytes",
        type=int,
        default=5 * 1024 * 1024 * 1024,
    )


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
    except SessionArtifactReadError as exc:
        print(
            json.dumps(
                {
                    "error": {
                        "code": exc.code,
                        "message": str(exc),
                        "recoverable": exc.recoverable,
                    }
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1
    except SessionOutputReadError as exc:
        print(
            json.dumps(
                {
                    "error": {
                        "code": exc.code,
                        "message": str(exc),
                        "recoverable": exc.recoverable,
                    }
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1
    except SharedUnderstandingGateError as exc:
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
    except WayfinderStatePersistenceError as exc:
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
    except CodingWorkspaceSelectionError as exc:
        print(
            json.dumps(
                {
                    "error": {
                        "code": exc.code,
                        "message": str(exc),
                        "recoverable": exc.recoverable,
                    }
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1
    except MissionChoiceError as exc:
        error_payload: dict[str, object] = {
            "code": exc.code,
            "message": str(exc),
            "recoverable": exc.recoverable,
        }
        if exc.expected_revision is not None:
            error_payload["expected_revision"] = exc.expected_revision
        if exc.current_revision is not None:
            error_payload["current_revision"] = exc.current_revision
        print(json.dumps({"error": error_payload}, sort_keys=True), file=sys.stderr)
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
    if args.command == "coding-workspace-select":
        acknowledgement = CodingWorkspaceSelectionService(
            starting_location=Path(args.starting_location),
            runtime_root=Path(args.runtime_root),
            forbidden_roots=tuple(Path(root) for root in args.forbidden_root),
        ).select(
            correlation_id=args.correlation_id,
            workspace_path=Path(args.workspace_path),
            selection_mode=args.selection_mode,
        )
        print(json.dumps(acknowledgement.to_dict(), sort_keys=True))
        return 0
    if args.command in {"mission-options", "mission-choice", "workspace-context"}:
        return _run_mission_journey(args)
    if args.command == "agent-capabilities":
        workspace_root = Path(args.target_repo)
        registry_path = (
            Path(args.agent_config)
            if args.agent_config
            else workspace_root / ".albert" / "agents.json"
        )
        registry = load_agent_registry(registry_path)
        projection = CapabilityCatalogService(
            workspace_root=workspace_root,
            agent_registry=registry,
            skill_roots=[Path(root) for root in args.skill_root],
            global_skill_roots=(
                [Path(root) for root in args.global_skill_root]
                if args.global_skill_root is not None
                else None
            ),
            ollama_probe=_cached_ollama_health,
        ).inspect()
        print(json.dumps(projection.to_dict(), sort_keys=True))
        return 0
    target_repo = Path(args.target_repo)
    registry_path = (
        Path(args.agent_config)
        if args.agent_config
        else target_repo / ".albert" / "agents.json"
    )
    availability_snapshot = _live_agent_availability(
        workspace_root=target_repo,
        registry_path=registry_path,
    )
    mission = AlbertMission(
        target_repo=target_repo,
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
            "agent-console-response",
            "working-context",
            "working-context-curate",
            "review-workspace",
            "session-artifact",
            "session-output",
            "activity-journal",
            "shell-terminal",
            "shell-terminal-submit",
            "additional-path-grant-create",
            "additional-path-grant-deny",
            "shell-terminal-decision",
            "review-decision",
            "workspace-queue",
            "ad-hoc-delegation-proposal",
            "workspace-queue-decision",
            "workstation-action",
            "workstation-session-run",
            "runner-observe",
            "mission-drafts",
            "mission-draft-create",
            "mission-draft-update",
            "mission-draft-confirm",
            "mission-draft-abandon",
        },
        issues_dir=Path(args.issues_dir) if args.issues_dir else None,
        agent_availability_snapshot=availability_snapshot,
        retention_grace_seconds=args.retention_grace_seconds,
        snapshot_storage_retention_seconds=args.snapshot_storage_retention_seconds,
        snapshot_storage_budget_bytes=args.snapshot_storage_budget_bytes,
    ).load()
    workspace_commands = {
        "workspace-snapshot",
        "workspace-action",
        "workspace-updates",
        "workspace-scope",
        "workspace-mission-switch",
        "agent-console-message",
        "agent-console-history",
        "agent-console-response",
        "working-context",
        "working-context-curate",
        "review-workspace",
        "session-artifact",
        "session-output",
        "activity-journal",
        "shell-terminal",
        "shell-terminal-submit",
        "additional-path-grant-create",
        "additional-path-grant-deny",
        "shell-terminal-decision",
        "review-decision",
        "workspace-queue",
        "ad-hoc-delegation-proposal",
        "workspace-queue-decision",
        "workstation-action",
        "workstation-session-run",
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
        target_id = args.target_id or args.scope_target
        if args.action_type != "conversation-scope-change":
            raise AlbertError("Conversation Scope action type must be conversation-scope-change")
        if args.actor != "mission-commander":
            raise AlbertError("Conversation Scope action actor must be mission-commander")
        if args.target_kind != "conversation-scope":
            raise AlbertError("Conversation Scope action target kind must be conversation-scope")
        if target_id != args.scope_target:
            raise AlbertError("Conversation Scope action target id must match scope target")
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
                mission_id=args.scope_mission_id or None,
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
    if args.command == "agent-console-response":
        response = AgentConsoleResponseService(snapshots).respond(
            message_id=args.message_id,
            expected_revision=args.expected_revision,
            expected_scope=ConversationScope(
                kind=args.scope_kind,
                target_id=args.scope_target,
                label=args.scope_label,
                mission_id=args.scope_mission_id or None,
            ),
            agent_id=args.agent_id,
        )
        print(json.dumps(asdict(response), sort_keys=True))
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
    if args.command == "session-artifact":
        projection = SessionArtifactService(snapshots).read(
            mission_id=args.artifact_mission_id,
            session_id=args.session_id,
            artifact_ref=args.artifact_ref,
        )
        print(json.dumps(asdict(projection), sort_keys=True))
        return 0
    if args.command == "session-output":
        projection = SessionOutputService(snapshots).read(
            mission_id=args.output_mission_id,
            session_id=args.session_id,
            after_sequence=args.after_sequence,
        )
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
            request_id=args.request_id,
            expected_revision=args.expected_terminal_revision,
            path=args.path,
            access_level=args.access_level,
            duration_seconds=args.duration_seconds,
            requester=args.requester,
        )
        print(json.dumps(asdict(grant), sort_keys=True))
        return 0
    if args.command == "additional-path-grant-deny":
        denial = ShellTerminalService(snapshots).deny_path_grant_request(
            correlation_id=args.correlation_id,
            request_id=args.request_id,
            expected_revision=args.expected_terminal_revision,
            path=args.path,
            access_level=args.access_level,
            duration_seconds=args.duration_seconds,
            requester=args.requester,
            reason=args.reason,
            affected_action=args.affected_action,
        )
        print(json.dumps(asdict(denial), sort_keys=True))
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
        if args.action_type != "review-decision":
            raise AlbertError("Review decision action type must be review-decision")
        if args.actor != "mission-commander":
            raise AlbertError("Review decision action actor must be mission-commander")
        if args.target_kind != "agent-session":
            raise AlbertError("Review decision action target kind must be agent-session")
        if args.target_id != args.session_id:
            raise AlbertError("Review decision action target id must match session id")
        acknowledgement = ReviewWorkspaceService(snapshots).decide(
            correlation_id=args.correlation_id,
            expected_revision=args.expected_revision,
            session_id=args.session_id,
            mission_id=args.review_mission_id or None,
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
    if args.command == "workstation-action":
        acknowledgement = WorkstationActionService(snapshots).submit(
            correlation_id=args.correlation_id,
            action_type=args.action_type,
            actor=args.actor,
            expected_revision=args.expected_revision,
            target_kind=args.target_kind,
            target_id=args.target_id,
            mission_id=args.action_mission_id or None,
            issue_id=args.issue_id,
            session_id=args.session_id,
            agent_id=args.agent,
            reason=args.reason,
            allowed_paths=args.allowed_path,
            command_policy=_parse_command_policy(args.command_policy),
        )
        print(json.dumps(asdict(acknowledgement), sort_keys=True))
        return 0
    if args.command == "workstation-session-run":
        mission_for_session = _mission_containing_session(
            snapshots,
            session_id=args.session_id,
            mission_id=args.session_mission_id,
        )
        session = mission_for_session.run_session(
            args.session_id,
            expected_revision=mission_for_session.sessions[args.session_id].revision,
        )
        print(
            json.dumps(
                _workstation_session_payload(mission_for_session, session),
                sort_keys=True,
            )
        )
        return 0
    if args.command == "runner-observe":
        if args.observation_mission_id != mission.mission_id:
            raise AlbertError("Runner observation Mission identity does not match.")
        receipt = mission.observe_runner(
            RunnerObservation(
                source_id=args.source_id,
                source_incarnation=args.source_incarnation,
                sequence=args.sequence,
                mission_id=args.observation_mission_id,
                session_id=args.session_id,
                session_revision=args.session_revision,
                runner_operation_id=args.runner_operation_id,
                owner_signal=args.owner_signal,
                process_group_signal=args.process_group_signal,
                worktree_identity=args.worktree_identity,
                result_signal=args.result_signal,
                result_digest=args.result_digest,
            )
        )
        print(json.dumps(asdict(receipt), sort_keys=True))
        return 0
    if args.command in {"retirement-preserve", "retirement-verify"}:
        if args.session_mission_id and args.session_mission_id != mission.mission_id:
            raise AlbertError("Retirement Unit Mission identity does not match.")
        mission_for_session = mission
        if args.session_id not in mission_for_session.sessions:
            raise AlbertError(f"Unknown Local Agent session: {args.session_id}")
        if args.command == "retirement-preserve":
            session = mission_for_session.preserve_retirement_unit(
                args.session_id,
                expected_revision=args.expected_revision,
                correlation_id=args.correlation_id,
            )
            verified = True
        else:
            session = mission_for_session.sessions[args.session_id]
            verified = mission_for_session.verify_retirement_snapshot(
                args.session_id
            )
        print(
            json.dumps(
                {
                    "schema_version": 1,
                    "mission_id": mission_for_session.mission_id,
                    "session_id": session.session_id,
                    "session_revision": session.revision,
                    "preservation_budget": session.preservation_budget,
                    "retirement": session.retirement,
                    "verified": verified,
                },
                sort_keys=True,
            )
        )
        return 0
    if args.command == "retirement-storage":
        print(json.dumps(mission.retirement_storage_inspection(), sort_keys=True))
        return 0
    if args.command in {
        "retirement-inspect",
        "retirement-pin",
        "retirement-retry",
        "retirement-export",
        "retirement-discard",
    }:
        if args.session_mission_id and args.session_mission_id != mission.mission_id:
            raise AlbertError("Retirement Unit Mission identity does not match.")
        if args.session_id not in mission.sessions:
            raise AlbertError(f"Unknown Local Agent session: {args.session_id}")
        if args.command == "retirement-inspect":
            result = mission.inspect_retirement_unit(args.session_id)
        elif args.command == "retirement-pin":
            mission.set_retirement_snapshot_pin(
                args.session_id,
                pinned=args.pin_state == "pinned",
                expected_revision=args.expected_revision,
                correlation_id=args.correlation_id,
            )
            result = mission.inspect_retirement_unit(args.session_id)
        elif args.command == "retirement-retry":
            mission.retry_retirement_unit(
                args.session_id,
                expected_revision=args.expected_revision,
                correlation_id=args.correlation_id,
            )
            result = mission.inspect_retirement_unit(args.session_id)
        elif args.command == "retirement-export":
            result = mission.export_retirement_unit(
                args.session_id,
                destination=Path(args.destination),
                expected_revision=args.expected_revision,
                correlation_id=args.correlation_id,
            )
        else:
            mission.discard_retained_worktree(
                args.session_id,
                expected_revision=args.expected_revision,
                correlation_id=args.correlation_id,
                confirmation=args.confirmation,
                reason=args.reason,
            )
            result = mission.inspect_retirement_unit(args.session_id)
        print(json.dumps(result, sort_keys=True))
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
            expected_revision=args.expected_session_revision,
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
        print(f"Queued {session.issue_id} as {session.session_id}")
        print(f"Worktree: {session.worktree_path}")
        runner_args = [
            sys.executable,
            "-m",
            "albert_mvp",
            "workstation-session-run",
            "--target-repo",
            str(mission.target_repo),
            "--tracker-dir",
            str(mission.tracker_dir),
            "--issues-dir",
            str(mission.issues_dir),
            "--runtime-root",
            str(mission.runtime_root),
            "--mission-id",
            mission.mission_id,
            "--agent-config",
            str(mission.agent_config_path),
            "--session-id",
            session.session_id,
            "--session-mission-id",
            mission.mission_id,
        ]
        print(f"Run: {shlex.join(runner_args)}")
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
        mission.record_evidence(
            args.session_id,
            evidence,
            expected_revision=args.expected_revision,
        )
        print(f"Evidence Package validated for {args.session_id}.")
        return 0
    if args.command == "review":
        decision = mission.record_frontier_review(
            args.session_id,
            args.outcome,
            reason=args.reason,
            failure_type=args.failure_type,
            expected_revision=args.expected_revision,
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


def _run_mission_journey(args: argparse.Namespace) -> int:
    journey = WorkspaceJourneyStore(Path(args.runtime_root))
    if args.command == "mission-options":
        projection = journey.mission_options(
            starting_location=Path(args.starting_location),
            coding_workspace=Path(args.coding_workspace),
        )
        print(json.dumps(projection, sort_keys=True))
        return 0
    if args.command == "mission-choice":
        acknowledgement = journey.choose_mission(
            starting_location=Path(args.starting_location),
            coding_workspace=Path(args.coding_workspace),
            correlation_id=args.correlation_id,
            expected_revision=args.expected_revision,
            choice=args.choice,
            mission_id=args.mission_id,
            mission_title=args.mission_title,
        )
        print(json.dumps(acknowledgement, sort_keys=True))
        return 0
    projection = journey.workspace_context(starting_location=Path(args.starting_location))
    print(json.dumps(projection, sort_keys=True))
    return 0


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


def _mission_containing_session(
    snapshots: WorkspaceSnapshotService,
    *,
    session_id: str,
    mission_id: str,
) -> AlbertMission:
    if mission_id:
        mission = snapshots._missions.get(mission_id)
        if mission is None:
            raise AlbertError(f"Unknown Mission for workstation session: {mission_id}")
        if session_id not in mission.sessions:
            raise AlbertError(f"Unknown Local Agent session in {mission_id}: {session_id}")
        return mission
    matches = [
        mission for mission in snapshots._missions.values() if session_id in mission.sessions
    ]
    if not matches:
        raise AlbertError(f"Unknown Local Agent session: {session_id}")
    if len(matches) > 1:
        raise AlbertError(
            f"Local Agent session {session_id} is ambiguous; provide --session-mission-id."
        )
    return matches[0]


def _workstation_session_payload(
    mission: AlbertMission,
    session: Any,
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "mission_id": mission.mission_id,
        "session_id": session.session_id,
        "issue_id": session.issue_id,
        "status": session.status,
        "runner_started_at": session.runner_started_at,
        "runner_ended_at": session.runner_ended_at,
        "runner_exit_status": session.runner_exit_status,
        "evidence_valid": session.evidence_valid,
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
        catalog_mission_ids: set[str] = set()
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
            if mission_id in catalog_mission_ids:
                raise ValueError(f"duplicate mission id: {mission_id}")
            catalog_mission_ids.add(mission_id)
            if mission_id == primary.mission_id:
                catalog_issues_dir = issues_dir or (tracker_dir / "issues")
                if (
                    tracker_dir.resolve() != primary.tracker_dir
                    or catalog_issues_dir.resolve() != primary.issues_dir
                ):
                    raise ValueError("primary Mission paths do not match the active Mission")
                continue
            missions.append(
                AlbertMission(
                    target_repo=primary.target_repo,
                    tracker_dir=tracker_dir,
                    issues_dir=issues_dir,
                    runtime_root=primary.runtime_root,
                    mission_id=mission_id,
                    agent_config_path=primary.agent_config_path,
                    allow_empty_tracker=True,
                    agent_availability_snapshot=primary.agent_availability_snapshot,
                    retention_grace_seconds=primary.retention_grace_seconds,
                    snapshot_storage_retention_seconds=(
                        primary.snapshot_storage_retention_seconds
                    ),
                    snapshot_storage_budget_bytes=(
                        primary.snapshot_storage_budget_bytes
                    ),
                ).load()
            )
        return WorkspaceSnapshotService(primary, missions=tuple(missions))
    except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
        raise AlbertError(f"Mission catalog could not be loaded: {exc}") from exc
