from __future__ import annotations

import codecs
from contextlib import contextmanager
from dataclasses import asdict, dataclass, field, replace
import datetime as datetime_module
from datetime import datetime, timedelta, timezone
import fcntl
from functools import wraps
from inspect import signature
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tempfile
import threading
from typing import Any, Literal

from .agents import AgentConfig, is_cloud_model, is_eligible_controller_agent
from .capabilities import CapabilityCatalogService
from .core import (
    AlbertError,
    AlbertMission,
    EvidenceValidationError,
    IssueSlice,
    LocalAgentSession,
    ReviewDecision,
    SharedUnderstandingGateError,
    WayfinderStatePersistenceError,
    _process_identity,
    _process_identity_is_live,
    _run_bounded_process,
    _trusted_system_executable,
    ensure_wayfinder_gate_open,
    load_wayfinder_state,
    sandboxed_process_argv,
    sanitized_process_environment,
    wayfinder_state_path,
)
from .execution import (
    ExecutionCoordinator,
    ExecutionJournal,
    ExecutionLimits,
    ExecutionReceipt,
    ExecutionRequest,
    ExecutionSandbox,
    PythonExecutionProvider,
    ShellExecutionAuthority,
)
from .performance import measured_stage


_SESSION_ARTIFACT_CONTENT_BYTES_LIMIT = 128_000
_SESSION_OUTPUT_JOURNAL_BYTES_LIMIT = 128_000
_SESSION_OUTPUT_EVENT_CONTENT_BYTES_LIMIT = 16_000
_SESSION_OUTPUT_EVENT_COUNT_LIMIT = 256
_AGENT_CONSOLE_USER_CONTENT_CHARACTER_LIMIT = 16_000
_AGENT_CONSOLE_CONTENT_CHARACTER_LIMIT = 100_000
_CONTROLLER_RECENT_CONVERSATION_CHARACTER_LIMIT = 24_000
_CONTROLLER_MESSAGE_CHARACTER_LIMIT = 16_000
_CONTROLLER_INPUT_CHARACTER_LIMIT = 96_000


def _valid_session_activity_at(candidate: Any) -> str:
    if not isinstance(candidate, str) or not candidate.strip():
        return ""
    value = candidate.strip()
    try:
        parsed = datetime.fromisoformat(
            value.removesuffix("Z") + ("+00:00" if value.endswith("Z") else "")
        )
    except ValueError:
        return ""
    return value if parsed.tzinfo is not None else ""


def _latest_session_activity_at(session: Any) -> str:
    for candidate in (
        session.runner_ended_at,
        session.cancel_requested_at,
        session.runner_started_at,
    ):
        value = _valid_session_activity_at(candidate)
        if value:
            return value
    return ""


_CHRONOLOGY_LOCKS_GUARD = threading.Lock()
_CHRONOLOGY_LOCKS: dict[str, threading.RLock] = {}
_CHRONOLOGY_LOCK_DEPTH = threading.local()


@contextmanager
def _chronology_order_lock(path: Path):
    key = str(path.resolve())
    with _CHRONOLOGY_LOCKS_GUARD:
        local_lock = _CHRONOLOGY_LOCKS.setdefault(key, threading.RLock())
    with local_lock:
        depths = getattr(_CHRONOLOGY_LOCK_DEPTH, "depths", None)
        if depths is None:
            depths = {}
            _CHRONOLOGY_LOCK_DEPTH.depths = depths
        depth = depths.get(key, 0)
        depths[key] = depth + 1
        try:
            if depth:
                yield
                return
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("a+", encoding="utf-8") as lock_file:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
                try:
                    yield
                finally:
                    fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)
        finally:
            remaining_depth = depths[key] - 1
            if remaining_depth:
                depths[key] = remaining_depth
            else:
                depths.pop(key, None)


def _causal_chronology(method):
    """Linearize durable effects with Console and Activity audit phases."""

    @wraps(method)
    def ordered(self, *args, **kwargs):
        lock_path = (
            self._snapshots.preferences_path.parent / ".chronology-order.lock"
        )
        with _chronology_order_lock(lock_path):
            return method(self, *args, **kwargs)

    return ordered


def _atomic_workspace_action(store_attribute: str | None = None):
    """Serialize one revision check and its authoritative store mutation."""

    def decorate(method):
        @wraps(method)
        def atomic(self, *args, **kwargs):
            store_path = (
                self._snapshots.preferences_path
                if store_attribute is None
                else getattr(self, store_attribute)
            )
            with self._snapshots._action_store_lock(store_path):
                return method(self, *args, **kwargs)

        return atomic

    return decorate


def _audit_rejected_workstation_action(method):
    """Persist valid Mission Commander action attempts that fail validation."""

    method_signature = signature(method)

    @wraps(method)
    def audited(self, *args, **kwargs):
        try:
            return method(self, *args, **kwargs)
        except AlbertError as error:
            if not isinstance(error, WorkspacePersistenceError):
                bound = method_signature.bind(self, *args, **kwargs)
                bound.apply_defaults()
                self._record_rejected_attempt(
                    error=error,
                    **{
                        key: value
                        for key, value in bound.arguments.items()
                        if key != "self"
                    },
                )
            raise

    return audited


class WorkspacePersistenceError(AlbertError):
    code = "persistence-read-failure"


class WorkspaceStaleActionError(AlbertError):
    code = "stale-action"

    def __init__(self, *, expected_revision: int, current_revision: int):
        self.expected_revision = expected_revision
        self.current_revision = current_revision
        super().__init__(
            f"Workspace action expected revision {expected_revision}, "
            f"but the current revision is {current_revision}."
        )


class WorkspaceRevisionGapError(AlbertError):
    code = "revision-gap"

    def __init__(self, *, requested_revision: int, current_revision: int):
        self.requested_revision = requested_revision
        self.current_revision = current_revision
        super().__init__(
            f"Workspace updates requested after revision {requested_revision}, "
            f"but the current revision is {current_revision}."
        )


class WorkspaceScopeMismatchError(AlbertError):
    code = "scope-mismatch"

    def __init__(self, *, expected_scope: ConversationScope, current_scope: ConversationScope):
        self.expected_scope = expected_scope
        self.current_scope = current_scope
        super().__init__(
            "Agent Console message scope does not match the acknowledged Conversation Scope."
        )


class WorkingContextCurationError(AlbertError):
    code = "context-source-ineligible"

    def __init__(self, source_id: str):
        self.source_id = source_id
        super().__init__(f"Working Context source is not eligible for curation: {source_id}")


class SessionArtifactReadError(AlbertError):
    """A safe, structured failure from the bounded session artifact boundary."""

    def __init__(
        self,
        message: str,
        *,
        code: str = "session-artifact-unavailable",
        recoverable: bool = True,
    ):
        self.code = code
        self.recoverable = recoverable
        super().__init__(message)


class SessionOutputReadError(AlbertError):
    """A safe, structured failure from the live session-output boundary."""

    def __init__(
        self,
        message: str,
        *,
        code: str = "session-output-unavailable",
        recoverable: bool = True,
    ):
        self.code = code
        self.recoverable = recoverable
        super().__init__(message)


@dataclass(frozen=True)
class ConversationScope:
    kind: Literal["working-directory", "mission", "issue-slice"]
    target_id: str
    label: str
    mission_id: str | None = None


@dataclass(frozen=True)
class MissionSummary:
    id: str
    title: str
    issue_count: int


@dataclass(frozen=True)
class WorkspaceSessionSummary:
    id: str
    workspace_path: str
    status: Literal["ready", "empty"]


@dataclass(frozen=True)
class RepairTaskPacketPreview:
    issue_id: str
    goal: str
    acceptance_criteria: tuple[str, ...]
    allowed_paths: tuple[str, ...]
    command_policy: dict[str, str]
    evidence_requirements: tuple[str, ...]
    assigned_agent: str
    review_reason: str


@dataclass(frozen=True)
class MissionSessionSummary:
    session_id: str
    issue_id: str
    assigned_agent: str
    status: str
    last_activity_at: str
    runner_started_at: str
    role: str
    provider: str
    model: str
    task_title: str
    operation_status: str
    failure: str
    changed_files: tuple[str, ...]
    commands_run: tuple[str, ...]
    test_results: str
    risks: str
    artifact_links: tuple[str, ...]
    launch_correlation_id: str
    evidence_correlation_id: str
    review_correlation_id: str
    review_outcome: str
    review_next_action: str
    repair_action_available: bool
    supervision_receipt_id: str = ""
    supervision_outcome: str = ""
    automatic_recovery_count: int = 0
    repair_task_packet: RepairTaskPacketPreview | None = None
    work_kind: str = ""
    parent_session_id: str = ""
    session_revision: int = 0
    retirement_phase: str = "active"
    retirement_blocked_reason: str = ""
    retirement_runner_boundary: dict[str, Any] = field(default_factory=dict)
    preservation_budget: dict[str, Any] = field(default_factory=dict)
    retirement_record: dict[str, Any] | None = None
    retirement_actions: dict[str, bool] = field(default_factory=dict)


@dataclass(frozen=True)
class WorkspaceQueueAttention:
    attention_id: str
    mission_id: str
    kind: Literal[
        "delegation-approval",
        "clarification",
        "issue-change-proposal",
        "frontier-confirmation",
        "ad-hoc-delegation",
        "runner-supervision",
        "retirement-storage",
    ]
    label: str
    queue_link: str
    entity_id: str = ""
    queue_item_id: str = ""


@dataclass(frozen=True)
class WorkspaceMissionSummary:
    id: str
    title: str
    issue_count: int
    is_active: bool
    sessions: tuple[MissionSessionSummary, ...]
    attention: tuple[WorkspaceQueueAttention, ...]
    archived_issue_ids: tuple[str, ...] = ()


@dataclass(frozen=True)
class WorkspaceSnapshot:
    schema_version: int
    revision: int
    workspace_session: WorkspaceSessionSummary
    active_mission: MissionSummary | None
    conversation_scope: ConversationScope
    operations_view: str
    mission_board: dict[str, Any]
    missions: tuple[WorkspaceMissionSummary, ...] = ()

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class WorkspaceAction:
    correlation_id: str
    expected_revision: int
    active_mission_id: str
    conversation_scope: ConversationScope
    operations_view: str


@dataclass(frozen=True)
class WorkspaceActionAcknowledgement:
    correlation_id: str
    outcome: Literal["acknowledged"]
    revision: int


@dataclass(frozen=True)
class WorkspaceEvent:
    event_id: str
    correlation_id: str
    revision: int
    kind: Literal["workspace-preferences-updated"]
    active_mission_id: str
    conversation_scope: ConversationScope
    operations_view: str


@dataclass(frozen=True)
class WorkspaceUpdateBatch:
    after_revision: int
    current_revision: int
    events: tuple[WorkspaceEvent, ...]


ActivityActor = Literal["mission-commander", "orchestrator", "frontier-model", "local-agent"]


@dataclass(frozen=True)
class ActivityAffectedEntity:
    entity_type: str
    entity_id: str
    label: str
    href: str = ""


@dataclass(frozen=True)
class ActivityJournalEntry:
    entry_id: str
    sequence: int
    recorded_at: str
    actor: ActivityActor
    action_type: str
    summary: str
    affected_entities: tuple[ActivityAffectedEntity, ...]
    evidence_links: tuple[str, ...]
    correlation_id: str


@dataclass(frozen=True)
class ActivityJournalProjection:
    schema_version: int
    revision: int
    entries: tuple[ActivityJournalEntry, ...]


@dataclass(frozen=True)
class ShellTerminalCommandResult:
    command_id: str
    correlation_id: str
    classification: Literal["auto-allowed", "frontier-approvable", "human-required"]
    status: Literal[
        "pending-approval",
        "executing",
        "outcome-unknown",
        "completed",
        "failed",
        "denied",
    ]
    exit_code: int | None
    stdout: str
    stderr: str


@dataclass(frozen=True)
class AdditionalPathGrant:
    grant_id: str
    correlation_id: str
    path: str
    access_level: Literal["read", "write"]
    duration_seconds: int
    granted_by: Literal["mission-commander"]
    granted_at: str
    expires_at: str
    request_id: str = ""


@dataclass(frozen=True)
class AdditionalPathGrantRequestRecord:
    request_id: str
    correlation_id: str
    mission_id: str
    path: str
    access_level: Literal["read", "write"]
    duration_seconds: int
    requester: str
    requested_at: str
    reason: str
    affected_action: str
    status: Literal["pending", "granted", "denied"] = "pending"


@dataclass(frozen=True)
class AdditionalPathGrantDenial:
    denial_id: str
    correlation_id: str
    request_id: str
    path: str
    access_level: Literal["read", "write"]
    duration_seconds: int
    denied_by: Literal["mission-commander"]
    denied_at: str
    reason: str
    affected_action: str


@dataclass(frozen=True)
class ShellTerminalCommandRecord:
    command_id: str
    correlation_id: str
    mission_id: str
    command: str
    classification: Literal["auto-allowed", "frontier-approvable", "human-required"]
    status: Literal[
        "pending-approval",
        "executing",
        "outcome-unknown",
        "completed",
        "failed",
        "denied",
    ]
    exit_code: int | None
    working_directory: str
    requested_paths: tuple[str, ...]
    access_level: Literal["read", "write"]
    requester: str
    approver: str = ""
    decider: str = ""
    reason: str = ""


@dataclass(frozen=True)
class ShellTerminalProjection:
    schema_version: int
    revision: int
    commands: tuple[ShellTerminalCommandRecord, ...]
    grants: tuple[AdditionalPathGrant, ...]
    grant_denials: tuple[AdditionalPathGrantDenial, ...]
    path_grant_requests: tuple[AdditionalPathGrantRequestRecord, ...]


class ShellTerminalService:
    """Executes governed commands while keeping terminal bytes transient."""

    _COMMAND_TIMEOUT_SECONDS = 30
    _SANDBOX_UNAVAILABLE_EXIT_CODE = 126
    _OUTPUT_BYTES_LIMIT = 1_000_000

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots
        self._terminal_path = snapshots.preferences_path.parent / "shell-terminal.json"
        self._path_grant_requests_path = (
            snapshots.preferences_path.parent / "path-grant-requests.json"
        )

    @property
    def terminal_path(self) -> Path:
        return self._terminal_path

    @property
    def path_grant_requests_path(self) -> Path:
        return self._path_grant_requests_path

    def _execution_journal_path_for_mission(self, mission_id: str) -> Path:
        mission = self._snapshots._missions.get(mission_id)
        if mission is None:
            raise WorkspacePersistenceError(
                f"Shell Terminal command references unknown Mission: {mission_id}"
            )
        return mission.runtime_dir / "execution-receipts.json"

    @contextmanager
    def _chronology_then_terminal_lock(self):
        chronology_path = (
            self._snapshots.preferences_path.parent / ".chronology-order.lock"
        )
        with _chronology_order_lock(chronology_path):
            with WorkspaceSnapshotService._json_store_lock(self._terminal_path):
                yield

    def inspect(self) -> ShellTerminalProjection:
        with self._chronology_then_terminal_lock():
            return self._inspect_locked()

    def _inspect_locked(self) -> ShellTerminalProjection:
        terminal = self._reconcile_execution_ledger(
            self._load_terminal(), terminal_lock_held=True
        )
        path_grant_requests = self._load_path_grant_requests()
        if any(
            record.get("status") == "executing"
            and self._projected_command_status(record) == "outcome-unknown"
            for record in terminal["commands"]
        ):
            terminal = self._try_persist_orphaned_executions(
                terminal, terminal_lock_held=True
            )
        self._reconcile_terminal_audit(terminal)
        return ShellTerminalProjection(
            schema_version=1,
            revision=terminal["revision"],
            commands=tuple(
                ShellTerminalCommandRecord(
                    command_id=item["command_id"],
                    correlation_id=item["correlation_id"],
                    mission_id=item.get("mission_id", ""),
                    command=item["command"],
                    classification=item["classification"],
                    status=self._projected_command_status(item),
                    exit_code=item["exit_code"],
                    working_directory=item["working_directory"],
                    requested_paths=tuple(item["requested_paths"]),
                    access_level=item.get("access_level", "read"),
                    requester=item["requester"],
                    approver=item.get("approver", ""),
                    decider=item.get("decider", ""),
                    reason=item.get("reason", ""),
                )
                for item in terminal["commands"]
            ),
            grants=tuple(AdditionalPathGrant(**item) for item in terminal["grants"]),
            grant_denials=tuple(
                AdditionalPathGrantDenial(**item) for item in terminal["grant_denials"]
            ),
            path_grant_requests=tuple(
                self._projected_path_grant_request(item, terminal=terminal)
                for item in path_grant_requests["requests"]
            ),
        )

    def reconcile_audit(self) -> None:
        """Repair missing command audit phases before unrelated chronology advances."""
        with self._chronology_then_terminal_lock():
            return self._reconcile_audit_locked()

    def _reconcile_audit_locked(self) -> None:
        terminal = self._reconcile_execution_ledger(
            self._load_terminal(), terminal_lock_held=True
        )
        if any(
            record.get("status") == "executing"
            and self._projected_command_status(record) == "outcome-unknown"
            for record in terminal["commands"]
        ):
            terminal = self._try_persist_orphaned_executions(
                terminal, terminal_lock_held=True
            )
        if any(
            record.get("status") == "executing"
            and self._projected_command_status(record) == "outcome-unknown"
            for record in terminal["commands"]
        ):
            raise WorkspacePersistenceError(
                "Shell Terminal outcome recovery is waiting for the command store; "
                "retry the later action after the active command finishes."
            )
        self._reconcile_terminal_audit(terminal)

    def submit(
        self,
        *,
        correlation_id: str,
        command: str,
        working_directory: str,
        requested_paths: list[str],
        requester: str,
        access_level: Literal["read", "write"] = "read",
    ) -> ShellTerminalCommandResult:
        if not correlation_id.strip():
            raise AlbertError("Shell Terminal correlation id must not be empty")
        if not command.strip():
            raise AlbertError("Shell Terminal command must not be empty")
        if not requester.strip():
            raise AlbertError("Shell Terminal requester must not be empty")
        if access_level not in {"read", "write"}:
            raise AlbertError(f"Unknown Shell Terminal access level: {access_level}")
        with self._chronology_then_terminal_lock():
            return self._submit_locked(
                correlation_id=correlation_id,
                command=command,
                working_directory=working_directory,
                requested_paths=requested_paths,
                requester=requester,
                access_level=access_level,
            )

    def _submit_locked(
        self,
        *,
        correlation_id: str,
        command: str,
        working_directory: str,
        requested_paths: list[str],
        requester: str,
        access_level: Literal["read", "write"],
    ) -> ShellTerminalCommandResult:
        terminal = self._reconcile_execution_ledger(
            self._load_terminal(), terminal_lock_held=True
        )
        normalized_correlation_id = correlation_id.strip()
        normalized_working_directory = str(Path(working_directory).resolve())
        normalized_requested_paths = [
            str(Path(path).resolve()) for path in requested_paths
        ]
        request_payload = {
            "command": command,
            "working_directory": normalized_working_directory,
            "requested_paths": normalized_requested_paths,
            "requester": requester,
            "access_level": access_level,
        }
        persisted = self._submission_for_correlation(
            terminal,
            correlation_id=normalized_correlation_id,
        )
        if persisted is not None:
            if self._submission_request(persisted) != request_payload:
                raise AlbertError(
                    f"Shell Terminal correlation id {normalized_correlation_id} was already "
                    "used for a different request."
                )
            if persisted.get("status") == "executing":
                persisted = self._persist_orphaned_execution(terminal, persisted)
            self._reconcile_submission_audit(persisted)
            return self._result_from_record(persisted)

        snapshot = self._snapshots.snapshot()
        if snapshot.active_mission is None:
            raise AlbertError("Shell Terminal requires an Active Mission")
        mission = self._snapshots._missions[snapshot.active_mission.id]
        working_path = Path(normalized_working_directory)
        if not self._path_authorized(
            working_path,
            access_level=access_level,
            workspace=mission.target_repo,
            grants=terminal["grants"],
        ):
            if self._matching_grant_expired(
                working_path,
                access_level=access_level,
                grants=terminal["grants"],
            ):
                raise AlbertError(
                    "Shell Terminal working directory Additional Path Grant is expired."
                )
            reason = (
                "Shell Terminal working directory is outside the workspace and has no "
                f"active {access_level} Additional Path Grant."
            )
            self._record_path_grant_request(
                correlation_id=normalized_correlation_id,
                mission_id=mission.mission_id,
                path=normalized_working_directory,
                access_level=access_level,
                requester=requester,
                reason=reason,
                affected_action=command,
            )
            raise AlbertError(reason)
        outside_paths = [
            path
            for path in normalized_requested_paths
            if not self._path_authorized(
                Path(path),
                access_level=access_level,
                workspace=mission.target_repo,
                grants=terminal["grants"],
            )
        ]
        if outside_paths:
            reason = (
                "Shell Terminal requested path is outside the workspace and has no "
                f"active {access_level} Additional Path Grant: {outside_paths[0]}"
            )
            self._record_path_grant_request(
                correlation_id=normalized_correlation_id,
                mission_id=mission.mission_id,
                path=outside_paths[0],
                access_level=access_level,
                requester=requester,
                reason=reason,
                affected_action=command,
            )
            raise AlbertError(reason)
        classification = mission.classify_command(command)
        command_id = f"terminal-command-{len(terminal['commands']) + 1:06d}"
        record = {
            "command_id": command_id,
            "correlation_id": normalized_correlation_id,
            "mission_id": mission.mission_id,
            "command": command,
            "classification": classification,
            "status": "pending-approval",
            "exit_code": None,
            "working_directory": str(working_path),
            "requested_paths": normalized_requested_paths,
            "access_level": access_level,
            "requester": requester,
        }
        if classification != "auto-allowed":
            chronology_path = (
                self._snapshots.preferences_path.parent
                / ".chronology-order.lock"
            )
            with _chronology_order_lock(chronology_path):
                self._persist_terminal(
                    revision=terminal["revision"] + 1,
                    commands=[*terminal["commands"], record],
                    grants=terminal["grants"],
                    grant_denials=terminal["grant_denials"],
                )
                self._reconcile_submission_audit(record)
                return ShellTerminalCommandResult(
                    command_id=command_id,
                    correlation_id=normalized_correlation_id,
                    classification=classification,
                    status="pending-approval",
                    exit_code=None,
                    stdout="",
                    stderr="",
                )
        return self._execute(
            terminal=terminal,
            record=record,
            record_index=None,
        )

    def _record_path_grant_request(
        self,
        *,
        correlation_id: str,
        mission_id: str,
        path: str,
        access_level: Literal["read", "write"],
        requester: str,
        reason: str,
        affected_action: str,
    ) -> AdditionalPathGrantRequestRecord:
        normalized_path = str(Path(path).resolve())
        with WorkspaceSnapshotService._json_store_lock(
            self._path_grant_requests_path
        ):
            store = self._load_path_grant_requests()
            matches = [
                item
                for item in store["requests"]
                if item.get("correlation_id") == correlation_id
                and item.get("path") == normalized_path
            ]
            if len(matches) > 1:
                raise WorkspacePersistenceError(
                    "Additional Path Grant request boundary is not unique: "
                    f"{correlation_id}/{normalized_path}"
                )
            if matches:
                request = AdditionalPathGrantRequestRecord(**matches[0])
                if (
                    request.mission_id != mission_id
                    or request.access_level != access_level
                    or request.duration_seconds != 900
                    or request.requester != requester
                    or request.reason != reason
                    or request.affected_action != affected_action
                ):
                    raise AlbertError(
                        "Shell Terminal correlation id was already used for a different "
                        "Additional Path Grant request boundary."
                    )
            else:
                request = AdditionalPathGrantRequestRecord(
                    request_id=(
                        f"path-grant-request-{len(store['requests']) + 1:06d}"
                    ),
                    correlation_id=correlation_id,
                    mission_id=mission_id,
                    path=normalized_path,
                    access_level=access_level,
                    duration_seconds=900,
                    requester=requester,
                    requested_at=datetime.now(timezone.utc)
                    .isoformat(timespec="seconds")
                    .replace("+00:00", "Z"),
                    reason=reason,
                    affected_action=affected_action,
                )
                self._persist_path_grant_requests(
                    [*store["requests"], asdict(request)]
                )
        self._reconcile_path_grant_requested(request)
        return request

    def _reconcile_path_grant_requested(
        self,
        request: AdditionalPathGrantRequestRecord,
    ) -> None:
        AgentConsoleHistoryService(
            self._snapshots
        ).record_additional_path_grant_requested(request=request)

    def create_path_grant(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        path: str,
        access_level: Literal["read", "write"],
        duration_seconds: int,
        requester: str,
        request_id: str = "",
    ) -> AdditionalPathGrant:
        if requester != "mission-commander":
            raise AlbertError(
                "Only the Mission Commander can create an Additional Path Grant."
            )
        if not correlation_id.strip():
            raise AlbertError("Additional Path Grant correlation id must not be empty")
        if access_level not in {"read", "write"}:
            raise AlbertError(f"Unknown Additional Path Grant access level: {access_level}")
        if duration_seconds <= 0:
            raise AlbertError("Additional Path Grant duration must be positive")
        with self._chronology_then_terminal_lock():
            return self._create_path_grant_locked(
                correlation_id=correlation_id,
                expected_revision=expected_revision,
                path=path,
                access_level=access_level,
                duration_seconds=duration_seconds,
                request_id=request_id,
            )

    def _create_path_grant_locked(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        path: str,
        access_level: Literal["read", "write"],
        duration_seconds: int,
        request_id: str,
    ) -> AdditionalPathGrant:
        terminal = self._reconcile_execution_ledger(
            self._load_terminal(), terminal_lock_held=True
        )
        normalized_correlation_id = correlation_id.strip()
        normalized_path = str(Path(path).resolve())
        normalized_request_id = request_id.strip()
        matching_grants = [
            item
            for item in terminal["grants"]
            if item.get("correlation_id") == normalized_correlation_id
        ]
        if len(matching_grants) > 1:
            raise WorkspacePersistenceError(
                "Additional Path Grant correlation id is not unique: "
                f"{normalized_correlation_id}"
            )
        if matching_grants:
            grant = AdditionalPathGrant(**matching_grants[0])
            if (
                grant.path != normalized_path
                or grant.access_level != access_level
                or grant.duration_seconds != duration_seconds
                or grant.granted_by != "mission-commander"
                or grant.request_id != normalized_request_id
            ):
                raise AlbertError(
                    "Additional Path Grant correlation id was already used for a "
                    f"different request: {normalized_correlation_id}"
                )
            self._reconcile_path_grant_created(grant)
            return grant
        if expected_revision != terminal["revision"]:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=terminal["revision"],
            )
        if normalized_request_id:
            request = self._path_grant_request_by_id(normalized_request_id)
            if request is None:
                raise AlbertError(
                    f"Unknown Additional Path Grant request: {normalized_request_id}"
                )
            projected_request = self._projected_path_grant_request(
                asdict(request),
                terminal=terminal,
            )
            if (
                projected_request.status != "pending"
                or request.path != normalized_path
                or request.access_level != access_level
                or request.duration_seconds != duration_seconds
                or request.requester != "mission-commander"
            ):
                raise AlbertError(
                    "Additional Path Grant request does not match the pending typed "
                    f"boundary: {normalized_request_id}"
                )
        now = datetime.now(timezone.utc)
        granted_at = now.isoformat(timespec="seconds").replace("+00:00", "Z")
        expires_at = (now + timedelta(seconds=duration_seconds)).isoformat(
            timespec="seconds"
        ).replace("+00:00", "Z")
        grant = AdditionalPathGrant(
            grant_id=f"path-grant-{len(terminal['grants']) + 1:06d}",
            correlation_id=normalized_correlation_id,
            path=normalized_path,
            access_level=access_level,
            duration_seconds=duration_seconds,
            granted_by="mission-commander",
            granted_at=granted_at,
            expires_at=expires_at,
            request_id=normalized_request_id,
        )
        self._persist_terminal(
            revision=terminal["revision"] + 1,
            commands=terminal["commands"],
            grants=[*terminal["grants"], asdict(grant)],
            grant_denials=terminal["grant_denials"],
        )
        self._reconcile_path_grant_created(grant)
        return grant

    def _reconcile_path_grant_created(self, grant: AdditionalPathGrant) -> None:
        ActivityJournalService(self._snapshots).record_additional_path_grant_created(
            correlation_id=grant.correlation_id,
            snapshot=self._snapshots.snapshot(),
            grant=grant,
        )
        AgentConsoleHistoryService(self._snapshots).record_additional_path_grant_created(
            grant=grant,
        )

    def deny_path_grant_request(
        self,
        *,
        correlation_id: str,
        request_id: str,
        expected_revision: int,
        path: str,
        access_level: Literal["read", "write"],
        duration_seconds: int,
        requester: str,
        reason: str,
        affected_action: str,
    ) -> AdditionalPathGrantDenial:
        if requester != "mission-commander":
            raise AlbertError(
                "Only the Mission Commander can deny an Additional Path Grant request."
            )
        if not correlation_id.strip():
            raise AlbertError("Additional Path Grant denial correlation id must not be empty")
        if not request_id.strip():
            raise AlbertError("Additional Path Grant request id must not be empty")
        if access_level not in {"read", "write"}:
            raise AlbertError(f"Unknown Additional Path Grant access level: {access_level}")
        if duration_seconds <= 0:
            raise AlbertError("Additional Path Grant duration must be positive")
        if not reason.strip():
            raise AlbertError("Additional Path Grant denial reason must not be empty")
        if not affected_action.strip():
            raise AlbertError("Additional Path Grant affected action must not be empty")
        with self._chronology_then_terminal_lock():
            terminal = self._load_terminal()
            normalized_correlation_id = correlation_id.strip()
            normalized_request_id = request_id.strip()
            normalized_path = str(Path(path).resolve())
            normalized_reason = reason.strip()
            normalized_affected_action = affected_action.strip()
            matching_denials = [
                item
                for item in terminal["grant_denials"]
                if item.get("correlation_id") == normalized_correlation_id
            ]
            if len(matching_denials) > 1:
                raise WorkspacePersistenceError(
                    "Additional Path Grant denial correlation id is not unique: "
                    f"{normalized_correlation_id}"
                )
            if matching_denials:
                denial = AdditionalPathGrantDenial(**matching_denials[0])
                if (
                    denial.request_id != normalized_request_id
                    or denial.path != normalized_path
                    or denial.access_level != access_level
                    or denial.duration_seconds != duration_seconds
                    or denial.denied_by != "mission-commander"
                    or denial.reason != normalized_reason
                    or denial.affected_action != normalized_affected_action
                ):
                    raise AlbertError(
                        "Additional Path Grant denial correlation id was already used "
                        f"for a different request: {normalized_correlation_id}"
                    )
                self._reconcile_path_grant_denied(denial)
                return denial
            typed_request = self._path_grant_request_by_id(normalized_request_id)
            if typed_request is not None:
                projected_request = self._projected_path_grant_request(
                    asdict(typed_request),
                    terminal=terminal,
                )
                if (
                    projected_request.status != "pending"
                    or typed_request.path != normalized_path
                    or typed_request.access_level != access_level
                    or typed_request.duration_seconds != duration_seconds
                    or typed_request.requester != requester
                    or typed_request.reason != normalized_reason
                    or typed_request.affected_action != normalized_affected_action
                ):
                    raise AlbertError(
                        "Additional Path Grant denial does not match the pending typed "
                        f"boundary: {normalized_request_id}"
                    )
            if expected_revision != terminal["revision"]:
                raise WorkspaceStaleActionError(
                    expected_revision=expected_revision,
                    current_revision=terminal["revision"],
                )
            denial = AdditionalPathGrantDenial(
                denial_id=(
                    f"path-grant-denial-{len(terminal['grant_denials']) + 1:06d}"
                ),
                correlation_id=normalized_correlation_id,
                request_id=normalized_request_id,
                path=normalized_path,
                access_level=access_level,
                duration_seconds=duration_seconds,
                denied_by="mission-commander",
                denied_at=datetime.now(timezone.utc)
                .isoformat(timespec="seconds")
                .replace("+00:00", "Z"),
                reason=normalized_reason,
                affected_action=normalized_affected_action,
            )
            self._persist_terminal(
                revision=terminal["revision"] + 1,
                commands=terminal["commands"],
                grants=terminal["grants"],
                grant_denials=[*terminal["grant_denials"], asdict(denial)],
            )
            self._reconcile_path_grant_denied(denial)
            return denial

    def _reconcile_path_grant_denied(
        self,
        denial: AdditionalPathGrantDenial,
    ) -> None:
        snapshot = self._snapshots.snapshot()
        ActivityJournalService(
            self._snapshots
        ).record_additional_path_grant_denied(
            correlation_id=denial.correlation_id,
            snapshot=snapshot,
            denial=denial,
        )
        AgentConsoleHistoryService(
            self._snapshots
        ).record_additional_path_grant_denied(
            denial=denial,
            mission_id=snapshot.active_mission.id if snapshot.active_mission else "",
        )

    def change_path_grant(
        self,
        *,
        grant_id: str,
        path: str,
        access_level: Literal["read", "write"],
        duration_seconds: int,
        requester: str,
    ) -> None:
        if requester != "mission-commander":
            raise AlbertError(
                "Agents and skills cannot broaden, renew, or change an Additional Path Grant."
            )
        raise AlbertError(
            f"Additional Path Grant {grant_id} is immutable; create a new bounded grant instead."
        )

    def approve(
        self,
        *,
        command_id: str,
        approver: str,
    ) -> ShellTerminalCommandResult:
        with self._chronology_then_terminal_lock():
            return self._approve_locked(command_id=command_id, approver=approver)

    def _approve_locked(
        self,
        *,
        command_id: str,
        approver: str,
    ) -> ShellTerminalCommandResult:
        terminal = self._reconcile_execution_ledger(
            self._load_terminal(), terminal_lock_held=True
        )
        commands = list(terminal["commands"])
        index = next(
            (position for position, item in enumerate(commands) if item["command_id"] == command_id),
            None,
        )
        if index is None:
            raise AlbertError(f"Unknown Shell Terminal command: {command_id}")
        record = commands[index]
        if record["status"] == "executing":
            record = self._persist_orphaned_execution(terminal, record)
        if record["status"] in {"outcome-unknown", "completed", "failed"}:
            if record.get("approver") == approver:
                self._reconcile_submission_audit(record)
                return self._result_from_record(record)
            raise AlbertError(
                f"Shell Terminal command is already {record['status']}: {command_id}"
            )
        if record["status"] != "pending-approval":
            raise AlbertError(f"Shell Terminal command is already {record['status']}: {command_id}")
        required_approver = (
            "frontier-model"
            if record["classification"] == "frontier-approvable"
            else "mission-commander"
        )
        if approver != required_approver:
            raise AlbertError(
                f"Shell Terminal {record['classification']} command requires approval "
                f"from {required_approver}."
            )
        record = {**record, "approver": approver}
        return self._execute(
            terminal=terminal,
            record=record,
            record_index=index,
        )

    def deny(
        self,
        *,
        command_id: str,
        decider: str,
        reason: str,
    ) -> ShellTerminalCommandResult:
        if decider != "mission-commander":
            raise AlbertError("Only the Mission Commander can deny a Shell Terminal command.")
        if not reason.strip():
            raise AlbertError("Shell Terminal command denial requires a reason.")
        with self._chronology_then_terminal_lock():
            return self._deny_locked(
                command_id=command_id,
                decider=decider,
                reason=reason,
            )

    @_causal_chronology
    def _deny_locked(
        self,
        *,
        command_id: str,
        decider: str,
        reason: str,
    ) -> ShellTerminalCommandResult:
        terminal = self._load_terminal()
        commands = list(terminal["commands"])
        index = next(
            (position for position, item in enumerate(commands) if item["command_id"] == command_id),
            None,
        )
        if index is None:
            raise AlbertError(f"Unknown Shell Terminal command: {command_id}")
        record = commands[index]
        if record["status"] == "denied":
            if (
                record.get("decider") == decider
                and record.get("reason") == reason.strip()
            ):
                self._reconcile_submission_audit(record)
                return self._result_from_record(record)
            raise AlbertError(
                f"Shell Terminal command is already denied: {command_id}"
            )
        if record["status"] != "pending-approval":
            raise AlbertError(f"Shell Terminal command is already {record['status']}: {command_id}")
        denied = {
            **record,
            "status": "denied",
            "decider": decider,
            "reason": reason.strip(),
        }
        commands[index] = denied
        self._persist_terminal(
            revision=terminal["revision"] + 1,
            commands=commands,
            grants=terminal["grants"],
            grant_denials=terminal["grant_denials"],
        )
        self._reconcile_submission_audit(denied)
        return self._result_from_record(denied)

    def _execute(
        self,
        *,
        terminal: dict[str, Any],
        record: dict[str, Any],
        record_index: int | None,
    ) -> ShellTerminalCommandResult:
        bubblewrap = _trusted_system_executable("bwrap")
        if bubblewrap is None:
            return self._finish_execution(
                terminal=terminal,
                record=record,
                record_index=record_index,
                exit_code=self._SANDBOX_UNAVAILABLE_EXIT_CODE,
                stdout="",
                stderr=(
                    "Shell Terminal sandbox unavailable: bubblewrap (bwrap) is required "
                    "for governed command execution; the command was not executed."
                ),
            )
        sandbox_argv, mount_error = self._sandbox_argv(
            bubblewrap=bubblewrap,
            terminal=terminal,
            record=record,
        )
        if mount_error:
            return self._finish_execution(
                terminal=terminal,
                record=record,
                record_index=record_index,
                exit_code=self._SANDBOX_UNAVAILABLE_EXIT_CODE,
                stdout="",
                stderr=mount_error,
            )
        execution_request = ExecutionRequest(
            request_id=f"shell:{record['command_id']}",
            effect="shell",
            argv=tuple(sandbox_argv),
            working_directory=str(Path(record["working_directory"]).resolve()),
            authority=ShellExecutionAuthority(
                mission_id=str(record["mission_id"]),
                command_id=str(record["command_id"]),
                correlation_id=str(record["correlation_id"]),
                command=str(record["command"]),
                classification=record["classification"],
                requester=str(record["requester"]),
                working_directory=str(Path(record["working_directory"]).resolve()),
                requested_paths=tuple(record["requested_paths"]),
                access_level=record.get("access_level", "read"),
                approval_actor=str(record.get("approver", "")),
            ),
            limits=ExecutionLimits(
                timeout_seconds=self._COMMAND_TIMEOUT_SECONDS,
                output_limit_bytes=self._OUTPUT_BYTES_LIMIT,
            ),
            sandbox=self._execution_sandbox_for_record(
                terminal=terminal,
                record=record,
            ),
            environment=tuple(sorted(sanitized_process_environment().items())),
        )
        attempt_record = {
            **record,
            "status": "executing",
            "exit_code": None,
            "executor_pid": os.getpid(),
            "executor_identity": _process_identity(os.getpid()),
            "execution_request_id": execution_request.request_id,
            "execution_request_digest": execution_request.request_digest,
        }
        commands = list(terminal["commands"])
        if record_index is None:
            record_index = len(commands)
            commands.append(attempt_record)
        else:
            commands[record_index] = attempt_record
        attempt_terminal = {
            **terminal,
            "revision": terminal["revision"] + 1,
            "commands": commands,
        }
        chronology_path = (
            self._snapshots.preferences_path.parent / ".chronology-order.lock"
        )
        with _chronology_order_lock(chronology_path):
            self._persist_terminal(
                revision=attempt_terminal["revision"],
                commands=commands,
                grants=terminal["grants"],
                grant_denials=terminal["grant_denials"],
            )
        try:
            receipt = ExecutionCoordinator(
                ExecutionJournal(
                    self._execution_journal_path_for_mission(str(record["mission_id"]))
                ),
                PythonExecutionProvider(executor=_run_bounded_process),
            ).execute(
                execution_request,
                authorize=lambda request: self._authorize_execution_request(request),
            )
            if receipt.status == "executing":
                return ShellTerminalCommandResult(
                    command_id=record["command_id"],
                    correlation_id=record["correlation_id"],
                    classification=record["classification"],
                    status="executing",
                    exit_code=None,
                    stdout="",
                    stderr=(
                        "Shell Terminal execution is still owned by a live provider; "
                        "the command was not replayed."
                    ),
                )
            if receipt.reconciliation_required:
                result = self._finish_outcome_unknown(
                    terminal=attempt_terminal,
                    record=attempt_record,
                    record_index=record_index,
                    receipt=receipt,
                )
                # Preserve the existing Shell Terminal transport failure contract
                # while the durable canonical state and execution ledger retain the
                # stronger typed uncertainty boundary. Exact retry returns the
                # outcome-unknown record and never reaches this provider path again.
                if receipt.status == "outcome-unknown":
                    raise OSError(
                        receipt.error_message
                        or "Shell Terminal host effect outcome is unknown; reconciliation is required."
                    )
                return result
            return self._finish_execution(
                terminal=attempt_terminal,
                record=attempt_record,
                record_index=record_index,
                exit_code=receipt.exit_code if receipt.exit_code is not None else 1,
                stdout=receipt.stdout,
                stderr=receipt.stderr or receipt.error_message,
                receipt=receipt,
            )
        except BaseException:
            completion_is_durable = False
            try:
                current = self._load_terminal()
                durable_record = self._submission_for_correlation(
                    current,
                    correlation_id=str(record["correlation_id"]),
                )
                completion_is_durable = (
                    durable_record is not None
                    and durable_record.get("status") in {"completed", "failed"}
                )
            except Exception:
                completion_is_durable = False
            if not completion_is_durable:
                self._best_effort_mark_outcome_unknown(
                    terminal=attempt_terminal,
                    record=attempt_record,
                    record_index=record_index,
                )
            raise

    @staticmethod
    def _authorize_execution_request(request: ExecutionRequest) -> None:
        if request.effect != "shell" or request.authority.kind != "shell":
            raise AlbertError("Shell Terminal execution request authority is invalid.")
        if request.shell:
            raise AlbertError("Shell Terminal execution request cannot enable shell parsing.")

    @_causal_chronology
    def _finish_execution(
        self,
        *,
        terminal: dict[str, Any],
        record: dict[str, Any],
        record_index: int | None,
        exit_code: int,
        stdout: str,
        stderr: str,
        receipt: ExecutionReceipt | None = None,
    ) -> ShellTerminalCommandResult:
        status: Literal["completed", "failed"] = (
            "completed" if exit_code == 0 else "failed"
        )
        completed_record = {
            **record,
            "status": status,
            "exit_code": exit_code,
            "executor_pid": None,
            "executor_identity": "",
        }
        if receipt is not None:
            completed_record.update(
                {
                    "execution_receipt_id": receipt.receipt_id,
                    "execution_status": receipt.status,
                }
            )
        commands = list(terminal["commands"])
        if record_index is None:
            commands.append(completed_record)
        else:
            commands[record_index] = completed_record
        self._persist_terminal(
            revision=terminal["revision"] + 1,
            commands=commands,
            grants=terminal["grants"],
            grant_denials=terminal["grant_denials"],
        )
        self._reconcile_submission_audit(completed_record)
        return ShellTerminalCommandResult(
            command_id=record["command_id"],
            correlation_id=record["correlation_id"],
            classification=record["classification"],
            status=status,
            exit_code=exit_code,
            stdout=stdout,
            stderr=stderr,
        )

    @_causal_chronology
    def _finish_outcome_unknown(
        self,
        *,
        terminal: dict[str, Any],
        record: dict[str, Any],
        record_index: int,
        receipt: ExecutionReceipt,
    ) -> ShellTerminalCommandResult:
        unknown = {
            **record,
            "status": "outcome-unknown",
            "exit_code": None,
            "executor_pid": None,
            "executor_identity": "",
            "execution_receipt_id": receipt.receipt_id,
            "execution_status": receipt.status,
        }
        commands = list(terminal["commands"])
        commands[record_index] = unknown
        self._persist_terminal(
            revision=terminal["revision"] + 1,
            commands=commands,
            grants=terminal["grants"],
            grant_denials=terminal["grant_denials"],
        )
        self._reconcile_submission_audit(unknown)
        return self._result_from_record(unknown)

    @_causal_chronology
    def _best_effort_mark_outcome_unknown(
        self,
        *,
        terminal: dict[str, Any],
        record: dict[str, Any],
        record_index: int,
    ) -> None:
        unknown = {
            **record,
            "status": "outcome-unknown",
            "exit_code": None,
            "executor_pid": None,
            "executor_identity": "",
        }
        commands = list(terminal["commands"])
        commands[record_index] = unknown
        try:
            self._persist_terminal(
                revision=terminal["revision"] + 1,
                commands=commands,
                grants=terminal["grants"],
                grant_denials=terminal["grant_denials"],
            )
            self._reconcile_submission_audit(unknown)
        except Exception:
            # Preserve the original execution/storage failure. A future locked replay
            # still converts the durable executing marker and repairs its audit trail
            # without re-running it.
            pass

    @_causal_chronology
    def _persist_orphaned_execution(
        self,
        terminal: dict[str, Any],
        record: dict[str, Any],
    ) -> dict[str, Any]:
        commands = list(terminal["commands"])
        index = commands.index(record)
        unknown = {
            **record,
            "status": "outcome-unknown",
            "exit_code": None,
            "executor_pid": None,
            "executor_identity": "",
        }
        commands[index] = unknown
        self._persist_terminal(
            revision=terminal["revision"] + 1,
            commands=commands,
            grants=terminal["grants"],
            grant_denials=terminal["grant_denials"],
        )
        self._reconcile_submission_audit(unknown)
        return unknown

    def _persist_orphaned_executions_locked(
        self,
        terminal: dict[str, Any],
    ) -> dict[str, Any]:
        commands: list[dict[str, Any]] = []
        changed = False
        for record in terminal["commands"]:
            if (
                record.get("status") == "executing"
                and self._projected_command_status(record) == "outcome-unknown"
            ):
                commands.append(
                    {
                        **record,
                        "status": "outcome-unknown",
                        "exit_code": None,
                        "executor_pid": None,
                        "executor_identity": "",
                    }
                )
                changed = True
            else:
                commands.append(record)
        if not changed:
            return terminal
        reconciled = {
            **terminal,
            "revision": terminal["revision"] + 1,
            "commands": commands,
        }
        self._persist_terminal(
            revision=reconciled["revision"],
            commands=commands,
            grants=terminal["grants"],
            grant_denials=terminal["grant_denials"],
        )
        return reconciled

    def _try_persist_orphaned_executions(
        self,
        observed_terminal: dict[str, Any],
        *,
        terminal_lock_held: bool = False,
    ) -> dict[str, Any]:
        if terminal_lock_held:
            return self._persist_orphaned_executions_locked(self._load_terminal())
        lock_path = self._terminal_path.with_name(
            f".{self._terminal_path.name}.lock"
        )
        lock_path.parent.mkdir(parents=True, exist_ok=True)
        with lock_path.open("a+", encoding="utf-8") as lock_file:
            try:
                fcntl.flock(
                    lock_file.fileno(),
                    fcntl.LOCK_EX | fcntl.LOCK_NB,
                )
            except BlockingIOError:
                # A live submit/decision owns the command store while it executes.
                # Preserve non-blocking inspection and retry recovery on the next poll.
                return observed_terminal
            try:
                return self._persist_orphaned_executions_locked(
                    self._load_terminal()
                )
            finally:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)

    @staticmethod
    def _projected_command_status(record: dict[str, Any]) -> str:
        if record.get("status") != "executing":
            return str(record.get("status", "failed"))
        pid = record.get("executor_pid")
        identity = record.get("executor_identity")
        if (
            isinstance(pid, int)
            and not isinstance(pid, bool)
            and isinstance(identity, str)
            and _process_identity_is_live(pid, identity)
        ):
            return "executing"
        return "outcome-unknown"

    @staticmethod
    def _submission_request(record: dict[str, Any]) -> dict[str, Any]:
        try:
            return {
                "command": record["command"],
                "working_directory": record["working_directory"],
                "requested_paths": list(record["requested_paths"]),
                "requester": record["requester"],
                "access_level": record.get("access_level", "read"),
            }
        except (KeyError, TypeError) as exc:
            raise WorkspacePersistenceError(
                "Shell Terminal command record has an invalid request boundary."
            ) from exc

    @staticmethod
    def _submission_for_correlation(
        terminal: dict[str, Any],
        *,
        correlation_id: str,
    ) -> dict[str, Any] | None:
        matches = [
            item
            for item in terminal["commands"]
            if item.get("correlation_id") == correlation_id
        ]
        if len(matches) > 1:
            raise WorkspacePersistenceError(
                f"Shell Terminal correlation id is not unique: {correlation_id}"
            )
        return matches[0] if matches else None

    @_causal_chronology
    def _reconcile_submission_audit(self, record: dict[str, Any]) -> None:
        classification = str(record["classification"])
        correlation_id = str(record["correlation_id"])
        mission_id = str(record.get("mission_id", ""))
        if classification != "auto-allowed":
            required_approver = (
                "frontier-model"
                if classification == "frontier-approvable"
                else "mission-commander"
            )
            ActivityJournalService(
                self._snapshots
            ).record_shell_command_approval_requested(
                correlation_id=correlation_id,
                snapshot=self._snapshots.snapshot(),
                command_record=record,
                required_approver=required_approver,
            )
            AgentConsoleHistoryService(
                self._snapshots
            ).record_shell_command_approval_requested(
                correlation_id=correlation_id,
                command_id=str(record["command_id"]),
                classification=classification,
                required_approver=required_approver,
                mission_id=mission_id,
            )
        approver = str(record.get("approver", ""))
        if approver:
            ActivityJournalService(self._snapshots).record_shell_command_approved(
                correlation_id=correlation_id,
                snapshot=self._snapshots.snapshot(),
                command_record=record,
                approver=approver,
            )
            AgentConsoleHistoryService(
                self._snapshots
            ).record_shell_command_approved(
                correlation_id=correlation_id,
                command_id=str(record["command_id"]),
                approver=approver,
                mission_id=mission_id,
            )
        if record["status"] == "denied":
            reason = str(record.get("reason", ""))
            if record.get("decider") != "mission-commander" or not reason:
                raise WorkspacePersistenceError(
                    "Shell Terminal denied command has no valid decision boundary."
                )
            ActivityJournalService(self._snapshots).record_shell_command_denied(
                correlation_id=correlation_id,
                snapshot=self._snapshots.snapshot(),
                command_record=record,
                reason=reason,
            )
            AgentConsoleHistoryService(
                self._snapshots
            ).record_shell_command_denied(
                correlation_id=correlation_id,
                command_id=str(record["command_id"]),
                reason=reason,
                mission_id=mission_id,
            )
        elif record["status"] in {"completed", "failed"}:
            exit_code = record.get("exit_code")
            if not isinstance(exit_code, int):
                raise WorkspacePersistenceError(
                    "Shell Terminal completed command has no valid exit code."
                )
            ActivityJournalService(self._snapshots).record_shell_command_finished(
                correlation_id=correlation_id,
                snapshot=self._snapshots.snapshot(),
                command_record=record,
            )
            AgentConsoleHistoryService(self._snapshots).record_shell_command_finished(
                correlation_id=correlation_id,
                command_id=str(record["command_id"]),
                status=str(record["status"]),
                exit_code=exit_code,
                mission_id=mission_id,
            )
        elif record["status"] == "outcome-unknown":
            ActivityJournalService(
                self._snapshots
            ).record_shell_command_outcome_unknown(
                correlation_id=correlation_id,
                snapshot=self._snapshots.snapshot(),
                command_record=record,
            )
            AgentConsoleHistoryService(
                self._snapshots
            ).record_shell_command_outcome_unknown(
                correlation_id=correlation_id,
                command_id=str(record["command_id"]),
                mission_id=mission_id,
            )

    def _reconcile_terminal_audit(self, terminal: dict[str, Any]) -> None:
        console_markers = {
            (message.correlation_id, message.action_phase)
            for message in AgentConsoleHistoryService(self._snapshots).history()
            if message.correlation_id and message.action_phase
        }
        journal_markers = {
            (entry.correlation_id, entry.action_type)
            for entry in ActivityJournalService(self._snapshots).inspect().entries
        }
        for item in self._load_path_grant_requests()["requests"]:
            request = AdditionalPathGrantRequestRecord(**item)
            audit_correlation_id = f"{request.correlation_id}:{request.request_id}"
            if (
                audit_correlation_id,
                "shell-path-grant-requested",
            ) not in console_markers:
                self._reconcile_path_grant_requested(request)
                console_markers.add(
                    (audit_correlation_id, "shell-path-grant-requested")
                )
        for item in terminal["grants"]:
            grant = AdditionalPathGrant(**item)
            if (
                (grant.correlation_id, "shell-path-grant-created")
                not in console_markers
                or (grant.correlation_id, "additional-path-grant-created")
                not in journal_markers
            ):
                self._reconcile_path_grant_created(grant)
                console_markers.add(
                    (grant.correlation_id, "shell-path-grant-created")
                )
                journal_markers.add(
                    (grant.correlation_id, "additional-path-grant-created")
                )
        for item in terminal["grant_denials"]:
            denial = AdditionalPathGrantDenial(**item)
            if (
                (denial.correlation_id, "shell-path-grant-denied")
                not in console_markers
                or (denial.correlation_id, "additional-path-grant-denied")
                not in journal_markers
            ):
                self._reconcile_path_grant_denied(denial)
                console_markers.add(
                    (denial.correlation_id, "shell-path-grant-denied")
                )
                journal_markers.add(
                    (denial.correlation_id, "additional-path-grant-denied")
                )
        for record in terminal["commands"]:
            correlation_id = str(record.get("correlation_id", ""))
            required_console, required_journal = self._audit_requirements(record)
            if any(
                (correlation_id, phase) not in console_markers
                for phase in required_console
            ) or any(
                (correlation_id, action_type) not in journal_markers
                for action_type in required_journal
            ):
                self._reconcile_submission_audit(record)
                console_markers.update(
                    (correlation_id, phase) for phase in required_console
                )
                journal_markers.update(
                    (correlation_id, action_type)
                    for action_type in required_journal
                )

    @staticmethod
    def _audit_requirements(
        record: dict[str, Any],
    ) -> tuple[tuple[str, ...], tuple[str, ...]]:
        console: list[str] = []
        journal: list[str] = []
        if record.get("classification") != "auto-allowed":
            console.append("shell-approval-request")
            journal.append("shell-command-approval-requested")
        if record.get("approver"):
            console.append("shell-approved")
            journal.append("shell-command-approved")
        status = record.get("status")
        if status == "denied":
            console.append("shell-denied")
            journal.append("shell-command-denied")
        elif status in {"completed", "failed"}:
            console.append("shell-finished")
            journal.append(
                "shell-command-completed"
                if status == "completed"
                else "shell-command-failed"
            )
        elif status == "outcome-unknown":
            console.append("shell-outcome-unknown")
            journal.append("shell-command-outcome-unknown")
        return tuple(console), tuple(journal)

    def _reconcile_execution_ledger(
        self,
        terminal: dict[str, Any],
        *,
        terminal_lock_held: bool = False,
    ) -> dict[str, Any]:
        """Project durable typed receipts into Shell's canonical command store."""

        if not terminal_lock_held:
            with WorkspaceSnapshotService._json_store_lock(self._terminal_path):
                return self._reconcile_execution_ledger(
                    self._load_terminal(), terminal_lock_held=True
                )
        journal_records_by_mission: dict[
            str, dict[str, tuple[ExecutionRequest, ExecutionReceipt]]
        ] = {}
        commands = list(terminal["commands"])
        changed_records: list[dict[str, Any]] = []
        changed = False
        for index, record in enumerate(commands):
            if record.get("status") not in {"executing", "outcome-unknown"}:
                continue
            request_id = record.get("execution_request_id")
            request_digest = record.get("execution_request_digest")
            if not isinstance(request_id, str) or not isinstance(request_digest, str):
                continue
            mission_id = record.get("mission_id")
            if not isinstance(mission_id, str) or not mission_id.strip():
                raise WorkspacePersistenceError(
                    "Shell Terminal execution record has no Mission boundary."
                )
            if mission_id not in journal_records_by_mission:
                journal_path = self._execution_journal_path_for_mission(mission_id)
                journal = ExecutionJournal(journal_path)
                journal.reconcile()
                records_for_mission = {
                    request.request_id: (request, receipt)
                    for request, receipt in journal.inspect_records()
                }
                legacy_path = (
                    self._snapshots.preferences_path.parent / "execution-receipts.json"
                )
                if journal_path.resolve() != legacy_path.resolve():
                    legacy = ExecutionJournal(legacy_path)
                    legacy.reconcile()
                    for request, receipt in legacy.inspect_records():
                        authority = request.authority
                        if (
                            request.request_id not in records_for_mission
                            and isinstance(authority, ShellExecutionAuthority)
                            and authority.mission_id == mission_id
                        ):
                            records_for_mission[request.request_id] = (
                                request,
                                receipt,
                            )
                journal_records_by_mission[mission_id] = records_for_mission
            request_and_receipt = journal_records_by_mission[mission_id].get(request_id)
            if request_and_receipt is None:
                continue
            request, receipt = request_and_receipt
            if receipt.status == "executing":
                continue
            if receipt.request_digest != request_digest:
                raise WorkspacePersistenceError(
                    "Shell Terminal execution receipt does not match its request boundary."
                )
            authority = request.authority
            if not isinstance(authority, ShellExecutionAuthority) or (
                request.effect != "shell"
                or authority.mission_id != mission_id
                or authority.command_id != record.get("command_id")
                or authority.correlation_id != record.get("correlation_id")
                or authority.command != record.get("command")
                or authority.classification != record.get("classification")
                or authority.requester != record.get("requester")
                or authority.working_directory != record.get("working_directory")
                or authority.requested_paths != tuple(record.get("requested_paths", []))
                or authority.access_level != record.get("access_level", "read")
                or authority.approval_actor != record.get("approver", "")
                or request.working_directory != record.get("working_directory")
            ):
                raise WorkspacePersistenceError(
                    "Shell Terminal execution receipt authority does not match its command."
                )
            if receipt.reconciliation_required:
                reconciled = {
                    **record,
                    "status": "outcome-unknown",
                    "exit_code": None,
                    "executor_pid": None,
                    "executor_identity": "",
                    "execution_receipt_id": receipt.receipt_id,
                    "execution_status": receipt.status,
                }
            else:
                exit_code = receipt.exit_code if receipt.exit_code is not None else 1
                reconciled = {
                    **record,
                    "status": "completed" if exit_code == 0 else "failed",
                    "exit_code": exit_code,
                    "executor_pid": None,
                    "executor_identity": "",
                    "execution_receipt_id": receipt.receipt_id,
                    "execution_status": receipt.status,
                }
            if reconciled != record:
                commands[index] = reconciled
                changed_records.append(reconciled)
                changed = True
        if not changed:
            return terminal
        reconciled_terminal = {
            **terminal,
            "revision": terminal["revision"] + 1,
            "commands": commands,
        }
        chronology_path = (
            self._snapshots.preferences_path.parent / ".chronology-order.lock"
        )
        with _chronology_order_lock(chronology_path):
            self._persist_terminal(
                revision=reconciled_terminal["revision"],
                commands=commands,
                grants=terminal["grants"],
                grant_denials=terminal["grant_denials"],
            )
        for record in changed_records:
            self._reconcile_submission_audit(record)
        return reconciled_terminal

    @staticmethod
    def _result_from_record(record: dict[str, Any]) -> ShellTerminalCommandResult:
        try:
            outcome_unknown = record["status"] == "outcome-unknown"
            return ShellTerminalCommandResult(
                command_id=record["command_id"],
                correlation_id=record["correlation_id"],
                classification=record["classification"],
                status=record["status"],
                exit_code=record.get("exit_code"),
                stdout="",
                stderr=(
                    "Shell Terminal recorded that execution started, but its final "
                    "outcome was not durably stored. The command will not be retried "
                    "automatically; inspect its intended effects before deciding what "
                    "to do next."
                    if outcome_unknown
                    else ""
                ),
            )
        except (KeyError, TypeError) as exc:
            raise WorkspacePersistenceError(
                "Shell Terminal command record cannot be replayed."
            ) from exc

    def _sandbox_mounts(
        self,
        *,
        terminal: dict[str, Any],
        record: dict[str, Any],
    ) -> tuple[dict[Path, Literal["read", "write"]], str]:
        mission_id = str(record.get("mission_id", ""))
        if not mission_id:
            snapshot = self._snapshots.snapshot()
            mission_id = snapshot.active_mission.id if snapshot.active_mission else ""
        mission = self._snapshots._missions.get(mission_id)
        if mission is None:
            return {}, "Shell Terminal sandbox could not resolve the submitting Mission."
        workspace = mission.target_repo.resolve()
        requested_access = record.get("access_level", "read")
        requested_mounts: dict[Path, Literal["read", "write"]] = {
            workspace: requested_access
        }
        governed_paths = {
            Path(record["working_directory"]).resolve(),
            *(Path(path).resolve() for path in record["requested_paths"]),
        }
        for path in governed_paths:
            if self._is_within(path, workspace):
                continue
            mount_access = self._granted_mount_access(
                path,
                access_level=record.get("access_level", "read"),
                grants=terminal["grants"],
            )
            if mount_access is None:
                return (
                    {},
                    "Shell Terminal sandbox refused an external path because its "
                    f"Additional Path Grant is missing or expired: {path}",
                )
            if not path.exists():
                return (
                    {},
                    "Shell Terminal sandbox cannot mount an authorized path that does "
                    f"not exist: {path}",
                )
            requested_mounts[path] = mount_access
        return requested_mounts, ""

    def _execution_sandbox_for_record(
        self,
        *,
        terminal: dict[str, Any],
        record: dict[str, Any],
    ) -> ExecutionSandbox:
        requested_mounts, mount_error = self._sandbox_mounts(
            terminal=terminal,
            record=record,
        )
        if mount_error:
            raise AlbertError(mount_error)
        return ExecutionSandbox(
            mode="bubblewrap",
            readable_roots=tuple(
                str(path.resolve())
                for path, access in requested_mounts.items()
                if access == "read"
            ),
            writable_roots=tuple(
                str(path.resolve())
                for path, access in requested_mounts.items()
                if access == "write"
            ),
        )

    def _sandbox_argv(
        self,
        *,
        bubblewrap: str,
        terminal: dict[str, Any],
        record: dict[str, Any],
    ) -> tuple[list[str], str]:
        requested_mounts, mount_error = self._sandbox_mounts(
            terminal=terminal,
            record=record,
        )
        if mount_error:
            return [], mount_error
        readable_roots = tuple(
            path for path, access in requested_mounts.items() if access == "read"
        )
        writable_roots = tuple(
            path for path, access in requested_mounts.items() if access == "write"
        )
        argv, sandboxed = sandboxed_process_argv(
            shlex.split(record["command"]),
            working_directory=Path(record["working_directory"]),
            readable_roots=readable_roots,
            writable_roots=writable_roots,
            allow_implicit_executable_bindings=False,
        )
        if not sandboxed or not isinstance(argv, list):
            return [], (
                "Shell Terminal sandbox unavailable: bubblewrap (bwrap) is required "
                "for governed command execution; the command was not executed."
            )
        return argv, ""

    @classmethod
    def _granted_mount_access(
        cls,
        path: Path,
        *,
        access_level: Literal["read", "write"],
        grants: list[dict[str, Any]],
    ) -> Literal["read", "write"] | None:
        now = datetime.now(timezone.utc).isoformat(timespec="seconds").replace(
            "+00:00", "Z"
        )
        matching = [
            grant
            for grant in grants
            if now < grant["expires_at"]
            and cls._is_within(path, Path(grant["path"]))
            and (
                grant["access_level"] == "write"
                or grant["access_level"] == access_level
            )
        ]
        if matching:
            return access_level
        return None

    def _persist_terminal(
        self,
        *,
        revision: int,
        commands: list[dict[str, Any]],
        grants: list[dict[str, Any]],
        grant_denials: list[dict[str, Any]],
    ) -> None:
        WorkspaceSnapshotService._write_json_atomically(
            self._terminal_path,
            {
                "schema_version": 1,
                "revision": revision,
                "commands": commands,
                "grants": grants,
                "grant_denials": grant_denials,
            },
        )

    def _persist_path_grant_requests(
        self,
        requests: list[dict[str, Any]],
    ) -> None:
        WorkspaceSnapshotService._write_json_atomically(
            self._path_grant_requests_path,
            {
                "schema_version": 1,
                "requests": requests,
            },
        )

    def _load_path_grant_requests(self) -> dict[str, Any]:
        if not self._path_grant_requests_path.exists():
            return {"schema_version": 1, "requests": []}
        try:
            payload = json.loads(
                self._path_grant_requests_path.read_text(encoding="utf-8")
            )
            if not isinstance(payload, dict):
                raise ValueError(
                    "Additional Path Grant request store must be an object"
                )
            if payload.get("schema_version") != 1:
                raise ValueError("unsupported Additional Path Grant request schema")
            requests = payload.get("requests")
            if not isinstance(requests, list):
                raise ValueError("Additional Path Grant requests must be a list")
            parsed: list[AdditionalPathGrantRequestRecord] = []
            for item in requests:
                if not isinstance(item, dict):
                    raise ValueError(
                        "Additional Path Grant request records must be objects"
                    )
                try:
                    request = AdditionalPathGrantRequestRecord(**item)
                except TypeError as exc:
                    raise ValueError(
                        "Additional Path Grant request has invalid fields"
                    ) from exc
                for field_name in (
                    "request_id",
                    "correlation_id",
                    "mission_id",
                    "requester",
                    "reason",
                    "affected_action",
                ):
                    if not getattr(request, field_name).strip():
                        raise ValueError(
                            f"Additional Path Grant request {field_name} must be named"
                        )
                self._validated_terminal_path(
                    request.path,
                    label="Additional Path Grant request",
                )
                if request.access_level not in {"read", "write"}:
                    raise ValueError(
                        "Additional Path Grant request access_level is invalid"
                    )
                if (
                    not isinstance(request.duration_seconds, int)
                    or isinstance(request.duration_seconds, bool)
                    or request.duration_seconds <= 0
                ):
                    raise ValueError(
                        "Additional Path Grant request duration must be positive"
                    )
                if request.status != "pending":
                    raise ValueError(
                        "Stored Additional Path Grant request status must be pending"
                    )
                self._validated_terminal_timestamp(
                    request.requested_at,
                    label="Additional Path Grant request requested_at",
                )
                parsed.append(request)
            request_ids = [request.request_id for request in parsed]
            if len(request_ids) != len(set(request_ids)):
                raise ValueError("Additional Path Grant request ids must be unique")
            boundaries = [
                (request.correlation_id, request.path) for request in parsed
            ]
            if len(boundaries) != len(set(boundaries)):
                raise ValueError(
                    "Additional Path Grant request boundaries must be unique"
                )
            return payload
        except (json.JSONDecodeError, OSError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(
                f"Additional Path Grant request persistence read failed: {exc}"
            ) from exc

    def _path_grant_request_by_id(
        self,
        request_id: str,
    ) -> AdditionalPathGrantRequestRecord | None:
        matches = [
            item
            for item in self._load_path_grant_requests()["requests"]
            if item.get("request_id") == request_id
        ]
        if len(matches) > 1:
            raise WorkspacePersistenceError(
                f"Additional Path Grant request id is not unique: {request_id}"
            )
        return AdditionalPathGrantRequestRecord(**matches[0]) if matches else None

    @staticmethod
    def _projected_path_grant_request(
        item: dict[str, Any],
        *,
        terminal: dict[str, Any],
    ) -> AdditionalPathGrantRequestRecord:
        request = AdditionalPathGrantRequestRecord(**item)
        if any(
            grant.get("request_id") == request.request_id
            for grant in terminal["grants"]
        ):
            return replace(request, status="granted")
        if any(
            denial.get("request_id") == request.request_id
            for denial in terminal["grant_denials"]
        ):
            return replace(request, status="denied")
        return request

    @staticmethod
    def _validated_terminal_path(value: object, *, label: str) -> str:
        if not isinstance(value, str) or not value.strip():
            raise ValueError(f"{label} path must be a non-empty string")
        path = Path(value)
        if not path.is_absolute() or str(path.resolve(strict=False)) != value:
            raise ValueError(f"{label} path must be canonical and absolute")
        return value

    @staticmethod
    def _validated_terminal_timestamp(value: object, *, label: str) -> datetime_module.datetime:
        if not isinstance(value, str) or not value.endswith("Z"):
            raise ValueError(f"{label} timestamp must be UTC ISO-8601")
        try:
            parsed = datetime_module.datetime.fromisoformat(
                value.removesuffix("Z") + "+00:00"
            )
        except ValueError as exc:
            raise ValueError(f"{label} timestamp must be UTC ISO-8601") from exc
        if parsed.utcoffset() != datetime_module.timedelta(0):
            raise ValueError(f"{label} timestamp must be UTC ISO-8601")
        return parsed

    @classmethod
    def _validate_terminal_command_record(cls, item: object) -> None:
        if not isinstance(item, dict):
            raise ValueError("Shell Terminal command records must be objects")
        for field_name in (
            "command_id",
            "correlation_id",
            "command",
            "working_directory",
            "requester",
        ):
            if not isinstance(item.get(field_name), str) or not item[field_name].strip():
                raise ValueError(
                    f"Shell Terminal command {field_name} must be a non-empty string"
                )
        mission_id = item.get("mission_id", "")
        if not isinstance(mission_id, str):
            raise ValueError("Shell Terminal command mission_id must be a string")
        if item.get("classification") not in {
            "auto-allowed",
            "frontier-approvable",
            "human-required",
        }:
            raise ValueError("Shell Terminal command classification is invalid")
        status = item.get("status")
        if status not in {
            "pending-approval",
            "executing",
            "outcome-unknown",
            "completed",
            "failed",
            "denied",
        }:
            raise ValueError("Shell Terminal command status is invalid")
        exit_code = item.get("exit_code")
        if status in {"completed", "failed"}:
            if not isinstance(exit_code, int) or isinstance(exit_code, bool):
                raise ValueError("Shell Terminal completed command exit_code is invalid")
        elif exit_code is not None:
            raise ValueError("Shell Terminal unfinished command exit_code must be null")
        cls._validated_terminal_path(
            item.get("working_directory"),
            label="Shell Terminal working directory",
        )
        requested_paths = item.get("requested_paths")
        if not isinstance(requested_paths, list):
            raise ValueError("Shell Terminal requested_paths must be a list")
        normalized_paths = [
            cls._validated_terminal_path(path, label="Shell Terminal requested")
            for path in requested_paths
        ]
        if len(normalized_paths) != len(set(normalized_paths)):
            raise ValueError("Shell Terminal requested_paths must be unique")
        if item.get("access_level", "read") not in {"read", "write"}:
            raise ValueError("Shell Terminal access_level is invalid")
        for field_name in (
            "approver",
            "decider",
            "reason",
            "executor_identity",
            "execution_request_id",
            "execution_request_digest",
            "execution_receipt_id",
            "execution_status",
        ):
            if field_name in item and not isinstance(item[field_name], str):
                raise ValueError(f"Shell Terminal command {field_name} must be a string")
        for field_name in ("executor_pid",):
            if field_name in item and item[field_name] is not None and (
                not isinstance(item[field_name], int) or isinstance(item[field_name], bool)
            ):
                raise ValueError(f"Shell Terminal command {field_name} is invalid")
        execution_request_id = item.get("execution_request_id")
        execution_request_digest = item.get("execution_request_digest")
        if (execution_request_id is None) != (execution_request_digest is None):
            raise ValueError("Shell Terminal execution request identity is incomplete")
        if execution_request_id is not None:
            if (
                not isinstance(execution_request_id, str)
                or not execution_request_id.strip()
                or not re.fullmatch(r"[A-Za-z0-9._:/=-]+", execution_request_id)
                or not isinstance(execution_request_digest, str)
                or re.fullmatch(r"[0-9a-f]{64}", execution_request_digest) is None
            ):
                raise ValueError("Shell Terminal execution request identity is invalid")
        receipt_id = item.get("execution_receipt_id")
        execution_status = item.get("execution_status")
        if (receipt_id is None) != (execution_status is None):
            raise ValueError("Shell Terminal execution receipt is incomplete")
        if receipt_id is not None:
            if (
                not isinstance(receipt_id, str)
                or not receipt_id.strip()
                or not re.fullmatch(r"[A-Za-z0-9._:/=-]+", receipt_id)
                or execution_status
                not in {
                    "completed",
                    "failed",
                    "cancelled",
                    "timed-out",
                    "output-limit",
                    "start-failed",
                    "outcome-unknown",
                }
            ):
                raise ValueError("Shell Terminal execution receipt is invalid")
        if receipt_id is not None and execution_request_id is None:
            raise ValueError("Shell Terminal execution request is missing")
        if (
            execution_request_id is not None
            and status in {"completed", "failed"}
            and receipt_id is None
        ):
            raise ValueError("Shell Terminal terminal receipt is missing")

    @classmethod
    def _validate_terminal_grant(cls, item: object) -> None:
        if not isinstance(item, dict):
            raise ValueError("Additional Path Grant records must be objects")
        try:
            grant = AdditionalPathGrant(**item)
        except TypeError as exc:
            raise ValueError("Additional Path Grant record has invalid fields") from exc
        for field_name in ("grant_id", "correlation_id"):
            if not getattr(grant, field_name).strip():
                raise ValueError(f"Additional Path Grant {field_name} must be named")
        if not isinstance(grant.request_id, str):
            raise ValueError("Additional Path Grant request_id must be a string")
        cls._validated_terminal_path(grant.path, label="Additional Path Grant")
        if grant.access_level not in {"read", "write"}:
            raise ValueError("Additional Path Grant access_level is invalid")
        if (
            not isinstance(grant.duration_seconds, int)
            or isinstance(grant.duration_seconds, bool)
            or grant.duration_seconds <= 0
        ):
            raise ValueError("Additional Path Grant duration must be positive")
        if grant.granted_by != "mission-commander":
            raise ValueError("Additional Path Grant actor is invalid")
        granted_at = cls._validated_terminal_timestamp(
            grant.granted_at,
            label="Additional Path Grant granted_at",
        )
        expires_at = cls._validated_terminal_timestamp(
            grant.expires_at,
            label="Additional Path Grant expires_at",
        )
        if expires_at <= granted_at:
            raise ValueError("Additional Path Grant expiry must follow grant time")

    @classmethod
    def _validate_terminal_grant_denial(cls, item: object) -> None:
        if not isinstance(item, dict):
            raise ValueError("Additional Path Grant denial records must be objects")
        try:
            denial = AdditionalPathGrantDenial(**item)
        except TypeError as exc:
            raise ValueError("Additional Path Grant denial has invalid fields") from exc
        for field_name in (
            "denial_id",
            "correlation_id",
            "request_id",
            "reason",
            "affected_action",
        ):
            if not getattr(denial, field_name).strip():
                raise ValueError(f"Additional Path Grant denial {field_name} must be named")
        cls._validated_terminal_path(denial.path, label="Additional Path Grant denial")
        if denial.access_level not in {"read", "write"}:
            raise ValueError("Additional Path Grant denial access_level is invalid")
        if (
            not isinstance(denial.duration_seconds, int)
            or isinstance(denial.duration_seconds, bool)
            or denial.duration_seconds <= 0
        ):
            raise ValueError("Additional Path Grant denial duration must be positive")
        if denial.denied_by != "mission-commander":
            raise ValueError("Additional Path Grant denial actor is invalid")
        cls._validated_terminal_timestamp(
            denial.denied_at,
            label="Additional Path Grant denial denied_at",
        )

    def _load_terminal(self) -> dict[str, Any]:
        if not self._terminal_path.exists():
            return {
                "schema_version": 1,
                "revision": 0,
                "commands": [],
                "grants": [],
                "grant_denials": [],
            }
        try:
            payload = json.loads(self._terminal_path.read_text(encoding="utf-8"))
            if payload["schema_version"] != 1:
                raise ValueError("unsupported Shell Terminal schema")
            if not isinstance(payload["revision"], int) or payload["revision"] < 0:
                raise ValueError("Shell Terminal revision must be non-negative")
            if not isinstance(payload["commands"], list):
                raise ValueError("Shell Terminal commands must be a list")
            for item in payload["commands"]:
                self._validate_terminal_command_record(item)
            correlations = [
                item.get("correlation_id")
                for item in payload["commands"]
                if isinstance(item, dict)
            ]
            if len(correlations) != len(payload["commands"]) or any(
                not isinstance(item, str) or not item.strip()
                for item in correlations
            ):
                raise ValueError("Shell Terminal command correlations must be named")
            if len(correlations) != len(set(correlations)):
                raise ValueError("Shell Terminal command correlations must be unique")
            if not isinstance(payload.get("grants", []), list):
                raise ValueError("Additional Path Grants must be a list")
            if not isinstance(payload.get("grant_denials", []), list):
                raise ValueError("Additional Path Grant denials must be a list")
            payload["grants"] = payload.get("grants", [])
            payload["grant_denials"] = payload.get("grant_denials", [])
            for item in payload["grants"]:
                self._validate_terminal_grant(item)
            for item in payload["grant_denials"]:
                self._validate_terminal_grant_denial(item)
            for label, values in (
                ("Additional Path Grant ids", [item["grant_id"] for item in payload["grants"]]),
                (
                    "Additional Path Grant correlations",
                    [item["correlation_id"] for item in payload["grants"]],
                ),
                (
                    "Additional Path Grant denial ids",
                    [item["denial_id"] for item in payload["grant_denials"]],
                ),
                (
                    "Additional Path Grant denial correlations",
                    [item["correlation_id"] for item in payload["grant_denials"]],
                ),
            ):
                if len(values) != len(set(values)):
                    raise ValueError(f"{label} must be unique")
            return payload
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(
                f"Shell Terminal persistence read failed: {exc}"
            ) from exc

    @staticmethod
    def _is_within(path: Path, parent: Path) -> bool:
        return path == parent or parent in path.parents

    @classmethod
    def _path_authorized(
        cls,
        path: Path,
        *,
        access_level: Literal["read", "write"],
        workspace: Path,
        grants: list[dict[str, Any]],
    ) -> bool:
        if cls._is_within(path, workspace):
            return True
        now = datetime.now(timezone.utc).isoformat(timespec="seconds").replace(
            "+00:00", "Z"
        )
        return any(
            now < grant["expires_at"]
            and cls._is_within(path, Path(grant["path"]))
            and (
                grant["access_level"] == "write"
                or grant["access_level"] == access_level
            )
            for grant in grants
        )

    @classmethod
    def _matching_grant_expired(
        cls,
        path: Path,
        *,
        access_level: Literal["read", "write"],
        grants: list[dict[str, Any]],
    ) -> bool:
        now = datetime.now(timezone.utc).isoformat(timespec="seconds").replace(
            "+00:00", "Z"
        )
        return any(
            now >= grant["expires_at"]
            and cls._is_within(path, Path(grant["path"]))
            and (
                grant["access_level"] == "write"
                or grant["access_level"] == access_level
            )
            for grant in grants
        )


AgentConsoleRole = Literal["user", "assistant", "system"]
AgentConsoleOutcome = Literal[
    "proposed", "pending", "acknowledged", "rejected", "model-commentary"
]
AgentConsoleActionOutcome = Literal["no-action", "awaiting-orchestrator"]
_AGENT_CONSOLE_ACTION_MESSAGES: dict[AgentConsoleActionOutcome, str] = {
    "no-action": (
        "No action taken. Controller prose is commentary and no correlated "
        "Orchestrator receipt exists."
    ),
    "awaiting-orchestrator": (
        "Coding task route selected. No action has occurred until a correlated "
        "Orchestrator receipt is recorded."
    ),
}


@dataclass(frozen=True)
class AgentConsoleMessage:
    message_id: str
    sequence: int
    role: AgentConsoleRole
    content: str
    scope: ConversationScope
    outcome: AgentConsoleOutcome
    source: str
    correlation_id: str = ""
    action_phase: str = ""
    action_outcome: AgentConsoleActionOutcome | Literal[""] = ""
    action_message: str = ""


AgentConsoleResponseIntent = Literal["discussion", "coding-task"]


@dataclass(frozen=True)
class AgentConsoleResponseRoute:
    intent: AgentConsoleResponseIntent
    task_request: str
    acceptance_criteria: tuple[str, ...]


@dataclass(frozen=True)
class AgentConsoleResponseProjection:
    message: AgentConsoleMessage
    route: AgentConsoleResponseRoute
    wayfinder: "WayfinderProjection"


WayfinderMode = Literal["outside", "chart", "work-through"]
WayfinderGateStatus = Literal["not-applicable", "pending", "open"]


@dataclass(frozen=True)
class WayfinderGate:
    status: WayfinderGateStatus
    opened_by: str = ""
    receipt_id: str = ""


@dataclass(frozen=True)
class WayfinderFlow:
    flow_id: str
    mode: Literal["chart", "work-through"]
    originating_message_id: str
    scope: ConversationScope
    reference: str = ""


@dataclass(frozen=True)
class WayfinderProjection:
    mode: WayfinderMode
    gate: WayfinderGate
    flow: WayfinderFlow | None
    continuing: bool
    turn_complete: bool


@dataclass(frozen=True)
class WayfinderDecision:
    projection: WayfinderProjection
    content: str
    correlation_id: str = ""
    action_phase: str = ""
    requires_agent_acknowledgement: bool = False
    allows_controller: bool = False


class WayfinderService:
    """Owns deterministic Wayfinder entry, continuation, and gate eligibility."""

    _SCHEMA_VERSION = 1
    _READ_ONLY = re.compile(
        r"^\s*(?:(?:please|could\s+you|can\s+you|would\s+you|"
        r"i\s+(?:want|need)\s+to)\s+)?"
        r"(?:explain|what(?:'s|\s+is)|why|how|status|review|diagnos(?:e|is)|inspect|show)\b",
        re.IGNORECASE,
    )
    _EXISTING_REFERENCE = re.compile(
        r"\b(?:wayfinder|wayfinding)\s+(?:(?:map|ticket|issue)(?:\s*#\d+)?|#\d+)\b",
        re.IGNORECASE,
    )
    _NEW_PROJECT = re.compile(
        r"\b(?:new\s+(?:project|app|application|service|repository|product)|"
        r"(?:start|create|build|launch|plan|design)\s+(?:a\s+)?new\s+"
        r"(?:project|app|application|service|repository|product))\b",
        re.IGNORECASE,
    )
    _CONSEQUENTIAL_CHANGE = re.compile(
        r"\b(?:consequential\s+change|architecture|architectural|redesign|migrat(?:e|ion)|"
        r"cross[-\s]cutting|platform[-\s]wide|replace\s+the\s+(?:system|architecture))\b",
        re.IGNORECASE,
    )
    _COMMANDER_CONFIRMATION = re.compile(
        r"^\s*(?:/wayfinder\s+confirm|confirm\s+shared\s+understanding)\b",
        re.IGNORECASE,
    )
    _ACKNOWLEDGEMENT_FIELDS = ("destination", "scope", "constraints", "uncertainty")

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots
        self._path = wayfinder_state_path(
            runtime_root=snapshots._primary_mission.runtime_root,
            target_repo=snapshots._primary_mission.target_repo,
        )

    @property
    def state_path(self) -> Path:
        return self._path

    def route(self, message: AgentConsoleMessage) -> WayfinderDecision | None:
        with WorkspaceSnapshotService._json_store_lock(self._path):
            state = self._load()
            active = state["active_flow"]
            if active is not None:
                return self._continue_active(state, active, message)
            return self._enter_if_required(state, message)

    def ensure_gate_open(self) -> None:
        ensure_wayfinder_gate_open(
            runtime_root=self._snapshots._primary_mission.runtime_root,
            target_repo=self._snapshots._primary_mission.target_repo,
        )

    def _enter_if_required(
        self,
        state: dict[str, Any],
        message: AgentConsoleMessage,
    ) -> WayfinderDecision | None:
        content = message.content.strip()
        if self._EXISTING_REFERENCE.search(content):
            return self._start_flow(
                state,
                message,
                mode="work-through",
                reference=content,
                content=(
                    "Wayfinder Work-through mode is active for the referenced planning work. "
                    "The Shared Understanding Gate is pending; no canonical artifact, "
                    "delegation, or production implementation was started."
                ),
            )
        if self._READ_ONLY.search(content):
            return None
        if self._NEW_PROJECT.search(content) or self._CONSEQUENTIAL_CHANGE.search(content):
            return self._start_flow(
                state,
                message,
                mode="chart",
                reference="",
                content=(
                    "Wayfinder Chart mode is active for this new or consequential effort. "
                    "The Shared Understanding Gate is pending; no canonical artifact, "
                    "delegation, or production implementation was started."
                ),
            )
        return None

    def _start_flow(
        self,
        state: dict[str, Any],
        message: AgentConsoleMessage,
        *,
        mode: Literal["chart", "work-through"],
        reference: str,
        content: str,
    ) -> WayfinderDecision:
        flow = {
            "flow_id": f"wayfinder-{message.message_id}",
            "mode": mode,
            "originating_message_id": message.message_id,
            "scope": asdict(message.scope),
            "reference": reference,
            "gate": {"status": "pending", "opened_by": "", "receipt_id": ""},
        }
        state["active_flow"] = flow
        self._write(state)
        return WayfinderDecision(
            projection=self._projection(flow, continuing=False),
            content=content,
            correlation_id=f"wayfinder-entry:{message.message_id}",
            action_phase=f"wayfinder-{mode}-entered",
        )

    def _continue_active(
        self,
        state: dict[str, Any],
        active: dict[str, Any],
        message: AgentConsoleMessage,
    ) -> WayfinderDecision:
        if self._COMMANDER_CONFIRMATION.search(message.content):
            receipt_id = f"wayfinder-gate:{message.message_id}"
            active["gate"] = {
                "status": "open",
                "opened_by": "mission-commander",
                "receipt_id": receipt_id,
            }
            self._write(state)
            return WayfinderDecision(
                projection=self._projection(active, continuing=True),
                content=(
                    "Mission Commander receipt opened the Shared Understanding Gate. "
                    "Wayfinder did not automatically create an artifact, delegate work, "
                    "or invoke another skill."
                ),
                correlation_id=receipt_id,
                action_phase="shared-understanding-gate-opened",
            )
        if active["gate"]["status"] == "pending" and self._has_agent_acknowledgement(
            message.content
        ):
            return WayfinderDecision(
                projection=self._projection(active, continuing=True),
                content="",
                requires_agent_acknowledgement=True,
            )
        return WayfinderDecision(
            projection=self._projection(active, continuing=True, turn_complete=False),
            content="",
            allows_controller=True,
        )

    def acknowledge_agent(self, message: AgentConsoleMessage) -> WayfinderDecision:
        """Persist the visible Wayfinder-agent acknowledgement for one pending flow."""
        with WorkspaceSnapshotService._json_store_lock(self._path):
            state = self._load()
            active = state["active_flow"]
            if (
                active is None
                or active["gate"]["status"] != "pending"
                or not self._has_agent_acknowledgement(message.content)
            ):
                raise AlbertError("Wayfinder agent acknowledgement is no longer eligible.")
            receipt_id = f"wayfinder-acknowledgement:{message.message_id}"
            active["gate"] = {
                "status": "open",
                "opened_by": "wayfinder-agent",
                "receipt_id": receipt_id,
            }
            self._write(state)
            return WayfinderDecision(
                projection=self._projection(active, continuing=True),
                content=(
                    "Wayfinder acknowledges the stated destination, scope, constraints, "
                    "and uncertainty. The Shared Understanding Gate is open. This "
                    "acknowledgement ends the turn; it did not create an artifact, "
                    "delegate work, or invoke another skill."
                ),
                correlation_id=receipt_id,
                action_phase="shared-understanding-agent-acknowledged",
            )

    def _projection(
        self,
        flow: dict[str, Any],
        *,
        continuing: bool,
        turn_complete: bool = True,
    ) -> WayfinderProjection:
        gate = flow["gate"]
        return WayfinderProjection(
            mode=flow["mode"],
            gate=WayfinderGate(**gate),
            flow=WayfinderFlow(
                flow_id=flow["flow_id"],
                mode=flow["mode"],
                originating_message_id=flow["originating_message_id"],
                scope=ConversationScope(**flow["scope"]),
                reference=flow["reference"],
            ),
            continuing=continuing,
            turn_complete=turn_complete,
        )

    def _has_agent_acknowledgement(self, content: str) -> bool:
        return all(
            re.search(rf"\b{field}\s*:\s*[^;\n]+", content, re.IGNORECASE)
            for field in self._ACKNOWLEDGEMENT_FIELDS
        )

    def _load(self) -> dict[str, Any]:
        try:
            state = load_wayfinder_state(self._path)
            active = state.get("active_flow")
            if active is None:
                return state
            self._projection(active, continuing=False)
            return state
        except WayfinderStatePersistenceError as exc:
            raise WorkspacePersistenceError(str(exc)) from exc
        except (json.JSONDecodeError, AttributeError, KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(
                f"Wayfinder state persistence read failed: {exc}"
            ) from exc

    def _write(self, state: dict[str, Any]) -> None:
        WorkspaceSnapshotService._write_json_atomically(self._path, state)


class AgentConsoleHistoryService:
    """Persists the continuous, scoped Agent Console record for a Workspace Session."""

    _roles = {"user", "assistant", "system"}
    _outcomes = {"proposed", "pending", "acknowledged", "rejected", "model-commentary"}
    _transient_sources = {"frontier-model-stream", "shell-terminal-stream", "raw-telemetry"}

    def __init__(self, snapshots: WorkspaceSnapshotService):
        self._snapshots = snapshots
        self._history_path = snapshots.preferences_path.parent / "agent-console-history.json"

    @property
    def history_path(self) -> Path:
        return self._history_path

    @_causal_chronology
    def append(
        self,
        *,
        role: AgentConsoleRole,
        content: str,
        outcome: AgentConsoleOutcome,
        source: str,
        expected_revision: int | None = None,
        expected_scope: ConversationScope | None = None,
        recorded_scope: ConversationScope | None = None,
        correlation_id: str = "",
        action_phase: str = "",
        action_outcome: AgentConsoleActionOutcome | Literal[""] = "",
        action_message: str = "",
    ) -> AgentConsoleMessage:
        if role not in self._roles:
            raise AlbertError(f"Unknown Agent Console role: {role}")
        if outcome not in self._outcomes:
            raise AlbertError(f"Unknown Agent Console outcome: {outcome}")
        if not content.strip():
            raise AlbertError("Agent Console message content must not be empty")
        content_limit = (
            _AGENT_CONSOLE_USER_CONTENT_CHARACTER_LIMIT
            if role == "user"
            else _AGENT_CONSOLE_CONTENT_CHARACTER_LIMIT
        )
        if len(content) > content_limit:
            raise AlbertError(
                f"Agent Console {role} message exceeds the {content_limit}-character limit"
            )
        if not source.strip():
            raise AlbertError("Agent Console message source must not be empty")
        if bool(correlation_id.strip()) != bool(action_phase.strip()):
            raise AlbertError(
                "Agent Console audit correlation id and action phase must be provided together"
            )
        if outcome == "model-commentary" and correlation_id.strip():
            raise AlbertError(
                "Model commentary cannot carry an Orchestrator receipt identity"
            )
        if action_outcome not in {"", "no-action", "awaiting-orchestrator"}:
            raise AlbertError(f"Unknown Agent Console action outcome: {action_outcome}")
        if bool(action_outcome) != bool(action_message.strip()):
            raise AlbertError(
                "Agent Console action outcome and message must be provided together"
            )
        if (
            action_outcome
            and action_message != _AGENT_CONSOLE_ACTION_MESSAGES[action_outcome]
        ):
            raise AlbertError(
                "Agent Console action message does not match its typed outcome"
            )
        if action_outcome and (
            outcome != "model-commentary" or source != "frontier-model"
        ):
            raise AlbertError(
                "Only Frontier Model commentary may carry a controller action outcome"
            )
        self._reject_transient_source(source)
        if not action_phase.startswith("shell-"):
            ShellTerminalService(self._snapshots).reconcile_audit()
        snapshot = (
            self._snapshots.snapshot()
            if recorded_scope is None
            or expected_revision is not None
            or expected_scope is not None
            else None
        )
        if snapshot is None and recorded_scope is None:
            raise AlbertError("Agent Console recorded scope is required.")
        if expected_revision is not None and expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        if expected_scope is not None and expected_scope != snapshot.conversation_scope:
            if snapshot.active_mission is not None:
                expected_scope = self._snapshots._qualify_scope(
                    expected_scope,
                    active_mission_id=snapshot.active_mission.id,
                )
        if expected_scope is not None and expected_scope != snapshot.conversation_scope:
            raise WorkspaceScopeMismatchError(
                expected_scope=expected_scope,
                current_scope=snapshot.conversation_scope,
            )
        with WorkspaceSnapshotService._json_store_lock(self._history_path):
            messages = list(self.history())
            durable_scope = recorded_scope or snapshot.conversation_scope
            if correlation_id:
                existing = next(
                    (
                        item
                        for item in messages
                        if item.correlation_id == correlation_id
                        and item.action_phase == action_phase
                    ),
                    None,
                )
                if existing is not None:
                    if (
                        existing.role != role
                        or existing.content != content
                        or existing.scope != durable_scope
                        or existing.outcome != outcome
                        or existing.source != source
                        or existing.action_outcome != action_outcome
                        or existing.action_message != action_message
                    ):
                        raise WorkspacePersistenceError(
                            "Agent Console audit marker resolves to a different effect: "
                            f"{correlation_id}/{action_phase}"
                        )
                    return existing
            sequence = len(messages) + 1
            message = AgentConsoleMessage(
                message_id=f"console-{sequence:06d}",
                sequence=sequence,
                role=role,
                content=content,
                scope=durable_scope,
                outcome=outcome,
                source=source,
                correlation_id=correlation_id,
                action_phase=action_phase,
                action_outcome=action_outcome,
                action_message=action_message,
            )
            messages.append(message)
            WorkspaceSnapshotService._write_json_atomically(
                self._history_path,
                {"schema_version": 1, "messages": [asdict(item) for item in messages]},
            )
        return message

    def reconcile_supervision_receipts(self) -> None:
        """Project canonical non-healthy supervision receipts exactly once."""

        for mission in self._snapshots._missions.values():
            for raw_receipt in mission.supervision.get("receipts", {}).values():
                if not isinstance(raw_receipt, dict):
                    raise WorkspacePersistenceError(
                        "Mission supervision receipt projection is invalid."
                    )
                outcome = raw_receipt.get("outcome")
                if outcome in {"no-change", "attention-recorded"}:
                    continue
                if outcome not in {
                    "recovered",
                    "result-reconciled",
                    "decision-needed",
                }:
                    raise WorkspacePersistenceError(
                        "Mission supervision receipt outcome is invalid."
                    )
                correlation_id = raw_receipt.get("correlation_id")
                session_id = raw_receipt.get("session_id")
                receipt_id = raw_receipt.get("receipt_id")
                if not all(
                    isinstance(value, str) and value.strip()
                    for value in (correlation_id, session_id, receipt_id)
                ):
                    raise WorkspacePersistenceError(
                        "Mission supervision receipt identity is invalid."
                    )
                if outcome == "recovered":
                    content = (
                        f"Deterministic supervision proved runner and process-group "
                        f"loss for {session_id} and queued one same-session/worktree "
                        f"recovery. Receipt {receipt_id}."
                    )
                    phase = "runner-recovered"
                    console_outcome: AgentConsoleOutcome = "acknowledged"
                elif outcome == "result-reconciled":
                    content = (
                        f"Deterministic supervision reconciled the exact late result "
                        f"for {session_id} instead of rerunning it. Receipt {receipt_id}."
                    )
                    phase = "runner-result-reconciled"
                    console_outcome = "acknowledged"
                else:
                    content = (
                        f"Deterministic supervision stopped automation for {session_id}; "
                        "a Mission Commander decision is required. Choose manual Retry "
                        "with a reason from Mission Work, or leave the session stopped. "
                        f"Receipt {receipt_id}."
                    )
                    phase = "runner-decision-needed"
                    console_outcome = "pending"
                self.append(
                    role="system",
                    content=content,
                    outcome=console_outcome,
                    source="orchestrator",
                    recorded_scope=self._mission_scope(mission.mission_id),
                    correlation_id=correlation_id,
                    action_phase=phase,
                )

    def record_workstation_action(
        self,
        *,
        correlation_id: str,
        action_type: str,
        target_id: str,
        effect_summary: str,
        mission_id: str,
    ) -> None:
        label = action_type.replace("-", " ")
        recorded_scope = self._mission_scope(mission_id)
        self.append(
            role="user",
            content=f"Workstation action: Mission Commander requested {label} for {target_id}.",
            outcome="pending",
            source="mission-commander",
            recorded_scope=recorded_scope,
            correlation_id=correlation_id,
            action_phase="request",
        )
        self.append(
            role="assistant",
            content=f"Orchestrator accepted workstation action: {effect_summary}",
            outcome="acknowledged",
            source="orchestrator",
            recorded_scope=recorded_scope,
            correlation_id=correlation_id,
            action_phase="acknowledgement",
        )

    def record_workstation_action_rejected(
        self,
        *,
        correlation_id: str,
        action_type: str,
        target_id: str,
        reason: str,
        mission_id: str,
    ) -> None:
        label = action_type.replace("-", " ")
        recorded_scope = self._mission_scope(mission_id)
        self.append(
            role="user",
            content=(
                f"Workstation action: Mission Commander requested {label} for "
                f"{target_id}."
            ),
            outcome="pending",
            source="mission-commander",
            recorded_scope=recorded_scope,
            correlation_id=correlation_id,
            action_phase="request",
        )
        self.append(
            role="assistant",
            content=f"Orchestrator rejected workstation action: {reason}",
            outcome="rejected",
            source="orchestrator",
            recorded_scope=recorded_scope,
            correlation_id=correlation_id,
            action_phase="rejection",
        )

    def record_shell_command_approval_requested(
        self,
        *,
        correlation_id: str,
        command_id: str,
        classification: str,
        required_approver: str,
        mission_id: str = "",
    ) -> None:
        self.append(
            role="system",
            content=(
                f"Shell Terminal command requires {required_approver} approval: "
                f"{command_id} ({classification})."
            ),
            outcome="pending",
            source="orchestrator",
            recorded_scope=self._mission_scope(mission_id),
            correlation_id=correlation_id,
            action_phase="shell-approval-request",
        )

    def record_shell_command_denied(
        self,
        *,
        correlation_id: str,
        command_id: str,
        reason: str,
        mission_id: str = "",
    ) -> None:
        self.append(
            role="user",
            content=f"Mission Commander denied Shell Terminal command {command_id}: {reason}",
            outcome="rejected",
            source="mission-commander",
            recorded_scope=self._mission_scope(mission_id),
            correlation_id=correlation_id,
            action_phase="shell-denied",
        )

    def record_shell_command_approved(
        self,
        *,
        correlation_id: str,
        command_id: str,
        approver: str,
        mission_id: str = "",
    ) -> None:
        approver_label = (
            "Mission Commander" if approver == "mission-commander" else "Frontier Model"
        )
        self.append(
            role="user" if approver == "mission-commander" else "assistant",
            content=f"{approver_label} approved Shell Terminal command {command_id}.",
            outcome="acknowledged",
            source=approver,
            recorded_scope=self._mission_scope(mission_id),
            correlation_id=correlation_id,
            action_phase="shell-approved",
        )

    def record_shell_command_finished(
        self,
        *,
        correlation_id: str,
        command_id: str,
        status: str,
        exit_code: int,
        mission_id: str = "",
    ) -> None:
        self.append(
            role="system",
            content=f"Shell Terminal command {status} with exit code {exit_code}: {command_id}.",
            outcome="acknowledged",
            source="orchestrator",
            recorded_scope=self._mission_scope(mission_id),
            correlation_id=correlation_id,
            action_phase="shell-finished",
        )

    def record_shell_command_outcome_unknown(
        self,
        *,
        correlation_id: str,
        command_id: str,
        mission_id: str = "",
    ) -> None:
        self.append(
            role="system",
            content=(
                "Shell Terminal command started, but its final outcome is unknown: "
                f"{command_id}. It will not be retried automatically."
            ),
            outcome="rejected",
            source="orchestrator",
            recorded_scope=self._mission_scope(mission_id),
            correlation_id=correlation_id,
            action_phase="shell-outcome-unknown",
        )

    def _mission_scope(self, mission_id: str) -> ConversationScope | None:
        mission = self._snapshots._missions.get(mission_id)
        if mission is None:
            return None
        return ConversationScope(
            kind="mission",
            target_id=mission.mission_id,
            label=mission.prd_title,
            mission_id=mission.mission_id,
        )

    def record_additional_path_grant_created(
        self,
        *,
        grant: AdditionalPathGrant,
    ) -> None:
        self.append(
            role="user",
            content=(
                f"Mission Commander granted {grant.access_level} Additional Path Grant "
                f"{grant.grant_id} for {grant.path} for {grant.duration_seconds} seconds "
                f"until {grant.expires_at}."
            ),
            outcome="acknowledged",
            source="mission-commander",
            correlation_id=grant.correlation_id,
            action_phase="shell-path-grant-created",
        )

    def record_additional_path_grant_requested(
        self,
        *,
        request: AdditionalPathGrantRequestRecord,
    ) -> None:
        self.append(
            role="system",
            content=(
                f"Shell Terminal requested {request.access_level} Additional Path Grant "
                f"{request.request_id} for {request.path} for "
                f"{request.duration_seconds} seconds: {request.reason}"
            ),
            outcome="pending",
            source="orchestrator",
            recorded_scope=self._mission_scope(request.mission_id),
            correlation_id=f"{request.correlation_id}:{request.request_id}",
            action_phase="shell-path-grant-requested",
        )

    def record_additional_path_grant_denied(
        self,
        *,
        denial: AdditionalPathGrantDenial,
        mission_id: str = "",
    ) -> None:
        self.append(
            role="user",
            content=(
                f"Mission Commander denied Additional Path Grant request "
                f"{denial.request_id} for {denial.access_level} access to {denial.path} "
                f"for {denial.duration_seconds} seconds; affected action: "
                f"{denial.affected_action}. Reason: {denial.reason}"
            ),
            outcome="rejected",
            source="mission-commander",
            recorded_scope=self._mission_scope(mission_id),
            correlation_id=denial.correlation_id,
            action_phase="shell-path-grant-denied",
        )

    def history(self) -> tuple[AgentConsoleMessage, ...]:
        if not self._history_path.exists():
            return ()
        try:
            payload = json.loads(self._history_path.read_text(encoding="utf-8"))
            if payload["schema_version"] != 1 or not isinstance(payload["messages"], list):
                raise ValueError("unsupported Agent Console history schema")
            raw_messages = payload["messages"]
            skipped_transient = any(
                isinstance(item, dict) and self._is_transient_source(item.get("source"))
                for item in raw_messages
            )
            messages = tuple(
                self._parse_message(item)
                for item in raw_messages
                if not (isinstance(item, dict) and self._is_transient_source(item.get("source")))
            )
            if skipped_transient:
                return tuple(
                    replace(
                        message,
                        message_id=f"console-{sequence:06d}",
                        sequence=sequence,
                    )
                    for sequence, message in enumerate(messages, start=1)
                )
            if [item.sequence for item in messages] != list(range(1, len(messages) + 1)):
                raise ValueError("Agent Console message sequence must be contiguous")
            if [item.message_id for item in messages] != [
                f"console-{sequence:06d}" for sequence in range(1, len(messages) + 1)
            ]:
                raise ValueError("Agent Console message ids must match sequence")
            audit_markers = [
                (item.correlation_id, item.action_phase)
                for item in messages
                if item.correlation_id
            ]
            if len(audit_markers) != len(set(audit_markers)):
                raise ValueError("Agent Console audit markers must be unique")
            return messages
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(
                f"Agent Console history persistence read failed: {exc}"
            ) from exc

    def _parse_message(self, item: dict[str, Any]) -> AgentConsoleMessage:
        role = item["role"]
        outcome = item["outcome"]
        if role not in self._roles:
            raise ValueError(f"unknown Agent Console role: {role}")
        if outcome not in self._outcomes:
            raise ValueError(f"unknown Agent Console outcome: {outcome}")
        if not isinstance(item["content"], str) or not item["content"].strip():
            raise ValueError("Agent Console message content must not be empty")
        if len(item["content"]) > _AGENT_CONSOLE_CONTENT_CHARACTER_LIMIT:
            raise ValueError("Agent Console message content exceeds the persistence limit")
        if not isinstance(item["source"], str) or not item["source"].strip():
            raise ValueError("Agent Console message source must not be empty")
        correlation_id = item.get("correlation_id", "")
        action_phase = item.get("action_phase", "")
        action_outcome = item.get("action_outcome", "")
        action_message = item.get("action_message", "")
        if not isinstance(correlation_id, str) or not isinstance(action_phase, str):
            raise ValueError("Agent Console audit markers must be strings")
        if bool(correlation_id.strip()) != bool(action_phase.strip()):
            raise ValueError(
                "Agent Console audit correlation id and action phase must be provided together"
            )
        if outcome == "model-commentary" and correlation_id.strip():
            raise ValueError(
                "model commentary cannot carry an Orchestrator receipt identity"
            )
        if action_outcome not in {"", "no-action", "awaiting-orchestrator"}:
            raise ValueError(f"unknown Agent Console action outcome: {action_outcome}")
        if not isinstance(action_message, str):
            raise ValueError("Agent Console action message must be a string")
        if bool(action_outcome) != bool(action_message.strip()):
            raise ValueError(
                "Agent Console action outcome and message must be provided together"
            )
        if (
            action_outcome
            and action_message != _AGENT_CONSOLE_ACTION_MESSAGES[action_outcome]
        ):
            raise ValueError(
                "Agent Console action message does not match its typed outcome"
            )
        if action_outcome and (
            outcome != "model-commentary" or item["source"] != "frontier-model"
        ):
            raise ValueError(
                "only Frontier Model commentary may carry a controller action outcome"
            )
        scope = ConversationScope(**item["scope"])
        if scope.kind not in {"working-directory", "mission", "issue-slice"}:
            raise ValueError(f"unknown Conversation Scope kind: {scope.kind}")
        if not isinstance(scope.target_id, str) or not scope.target_id.strip():
            raise ValueError("Conversation Scope target must not be empty")
        if not isinstance(scope.label, str) or not scope.label.strip():
            raise ValueError("Conversation Scope label must not be empty")
        return AgentConsoleMessage(
            message_id=item["message_id"],
            sequence=item["sequence"],
            role=role,
            content=item["content"],
            scope=scope,
            outcome=outcome,
            source=item["source"],
            correlation_id=correlation_id,
            action_phase=action_phase,
            action_outcome=action_outcome,
            action_message=action_message,
        )

    @classmethod
    def _reject_transient_source(
        cls,
        source: str,
        *,
        error_type: type[Exception] = AlbertError,
    ) -> None:
        if not cls._is_transient_source(source):
            return
        raise error_type(
            "Transient stream telemetry must not be persisted in Agent Console history"
        )

    @classmethod
    def _is_transient_source(cls, source: object) -> bool:
        return isinstance(source, str) and source in cls._transient_sources


class AgentConsoleResponseService:
    """Generates and records a controller response for one correlated user prompt."""

    _CONTROLLER_OUTPUT_BYTES_LIMIT = 1_000_000
    _REPLY_CHARACTER_LIMIT = 100_000
    _TASK_REQUEST_CHARACTER_LIMIT = 4_000
    _ACCEPTANCE_CRITERION_CHARACTER_LIMIT = 2_000
    _ACCEPTANCE_CRITERIA_COUNT_LIMIT = 12
    _ACCEPTANCE_CRITERIA_CHARACTER_LIMIT = 12_000
    _NO_ACTION_MESSAGE = _AGENT_CONSOLE_ACTION_MESSAGES["no-action"]
    _AWAITING_ORCHESTRATOR_MESSAGE = _AGENT_CONSOLE_ACTION_MESSAGES[
        "awaiting-orchestrator"
    ]
    _MALFORMED_RESPONSE_MESSAGE = (
        "The controller response was malformed and remains discussion. No action taken."
    )
    _NON_AUTHORITATIVE_DISCUSSION_MESSAGE = (
        "Controller classified this prompt as discussion. Untrusted reply prose was "
        "not retained. No action taken."
    )
    _NON_AUTHORITATIVE_CODING_ROUTE_MESSAGE = (
        "Controller classified this prompt as a coding task. Untrusted reply prose "
        "was not retained; no action has occurred."
    )

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots
        self._history = AgentConsoleHistoryService(snapshots)

    def respond(
        self,
        *,
        message_id: str,
        expected_revision: int,
        expected_scope: ConversationScope,
        agent_id: str = "",
    ) -> AgentConsoleResponseProjection:
        if not message_id.strip():
            raise AlbertError("Agent Console response message id must not be empty.")
        snapshot = self._snapshots.snapshot()
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        if expected_scope != snapshot.conversation_scope:
            if snapshot.active_mission is not None:
                expected_scope = self._snapshots._qualify_scope(
                    expected_scope,
                    active_mission_id=snapshot.active_mission.id,
                )
        if expected_scope != snapshot.conversation_scope:
            raise WorkspaceScopeMismatchError(
                expected_scope=expected_scope,
                current_scope=snapshot.conversation_scope,
            )
        latest = self._correlated_user_message(message_id)
        wayfinder_service = WayfinderService(self._snapshots)
        wayfinder = wayfinder_service.route(latest)
        if wayfinder is not None and wayfinder.requires_agent_acknowledgement:
            wayfinder = wayfinder_service.acknowledge_agent(latest)
        if wayfinder is not None and not wayfinder.allows_controller:
            acknowledged = bool(wayfinder.correlation_id)
            message = self._history.append(
                role="assistant",
                content=wayfinder.content,
                outcome="acknowledged" if acknowledged else "model-commentary",
                source=(
                    "wayfinder-agent"
                    if wayfinder.action_phase == "shared-understanding-agent-acknowledged"
                    else "orchestrator"
                    if acknowledged
                    else "frontier-model"
                ),
                recorded_scope=latest.scope,
                correlation_id=wayfinder.correlation_id,
                action_phase=wayfinder.action_phase,
                action_outcome="" if acknowledged else "no-action",
                action_message=(
                    ""
                    if acknowledged
                    else self._NO_ACTION_MESSAGE
                ),
            )
            return AgentConsoleResponseProjection(
                message=message,
                route=self._discussion_route(),
                wayfinder=wayfinder.projection,
            )
        agent: AgentConfig | None = None
        content = self._command_response(latest, agent)
        route = self._discussion_route()
        if content is None:
            agent = self._select_agent(agent_id)
            controller_output = self._controller_response(agent, latest)
            parsed_reply, route = self._parse_controller_output(controller_output)
            if parsed_reply == self._MALFORMED_RESPONSE_MESSAGE:
                content = parsed_reply
            elif route.intent == "coding-task":
                content = self._NON_AUTHORITATIVE_CODING_ROUTE_MESSAGE
            else:
                content = self._NON_AUTHORITATIVE_DISCUSSION_MESSAGE
        if latest.content.lstrip().startswith("/"):
            route = self._discussion_route()
        action_outcome: AgentConsoleActionOutcome = (
            "awaiting-orchestrator"
            if route.intent == "coding-task"
            else "no-action"
        )
        message = self._history.append(
            role="assistant",
            content=content,
            outcome="model-commentary",
            source="frontier-model",
            recorded_scope=latest.scope,
            action_outcome=action_outcome,
            action_message=(
                self._AWAITING_ORCHESTRATOR_MESSAGE
                if action_outcome == "awaiting-orchestrator"
                else self._NO_ACTION_MESSAGE
            ),
        )
        return AgentConsoleResponseProjection(
            message=message,
            route=route,
            wayfinder=(
                wayfinder.projection
                if wayfinder is not None
                else WayfinderProjection(
                    mode="outside",
                    gate=WayfinderGate(status="not-applicable"),
                    flow=None,
                    continuing=False,
                    turn_complete=False,
                )
            ),
        )

    @staticmethod
    def _discussion_route() -> AgentConsoleResponseRoute:
        return AgentConsoleResponseRoute(
            intent="discussion",
            task_request="",
            acceptance_criteria=(),
        )

    def _parse_controller_output(
        self,
        output: str,
    ) -> tuple[str, AgentConsoleResponseRoute]:
        candidate = output.strip()
        lines = candidate.splitlines()
        if (
            len(lines) >= 3
            and lines[0].strip().casefold() in {"```", "```json"}
            and lines[-1].strip() == "```"
        ):
            candidate = "\n".join(lines[1:-1]).strip()
        try:
            payload = json.loads(candidate)
        except (json.JSONDecodeError, TypeError):
            return self._MALFORMED_RESPONSE_MESSAGE, self._discussion_route()
        if not isinstance(payload, dict) or set(payload) != {"reply", "route"}:
            return self._MALFORMED_RESPONSE_MESSAGE, self._discussion_route()
        reply = payload.get("reply")
        route = payload.get("route")
        if (
            not isinstance(reply, str)
            or not reply.strip()
            or len(reply.strip()) > self._REPLY_CHARACTER_LIMIT
            or "\0" in reply
        ):
            return self._MALFORMED_RESPONSE_MESSAGE, self._discussion_route()
        safe_reply = reply.strip()
        if not isinstance(route, dict) or set(route) != {
            "intent",
            "task_request",
            "acceptance_criteria",
        }:
            return self._MALFORMED_RESPONSE_MESSAGE, self._discussion_route()
        intent = route.get("intent")
        task_request = route.get("task_request")
        criteria = route.get("acceptance_criteria")
        if intent == "discussion":
            return safe_reply, self._discussion_route()
        if intent != "coding-task":
            return self._MALFORMED_RESPONSE_MESSAGE, self._discussion_route()
        if (
            not isinstance(task_request, str)
            or not task_request.strip()
            or len(task_request.strip()) > self._TASK_REQUEST_CHARACTER_LIMIT
            or "\0" in task_request
            or not isinstance(criteria, list)
            or not (1 <= len(criteria) <= self._ACCEPTANCE_CRITERIA_COUNT_LIMIT)
            or any(
                not isinstance(item, str)
                or not item.strip()
                or len(item.strip()) > self._ACCEPTANCE_CRITERION_CHARACTER_LIMIT
                or "\0" in item
                for item in criteria
            )
        ):
            return self._MALFORMED_RESPONSE_MESSAGE, self._discussion_route()
        normalized_criteria = tuple(item.strip() for item in criteria)
        if sum(len(item) for item in normalized_criteria) > self._ACCEPTANCE_CRITERIA_CHARACTER_LIMIT:
            return self._MALFORMED_RESPONSE_MESSAGE, self._discussion_route()
        return (
            safe_reply,
            AgentConsoleResponseRoute(
                intent="coding-task",
                task_request=task_request.strip(),
                acceptance_criteria=normalized_criteria,
            ),
        )

    def _correlated_user_message(self, message_id: str) -> AgentConsoleMessage:
        message = next(
            (
                candidate
                for candidate in self._history.history()
                if candidate.message_id == message_id
            ),
            None,
        )
        if message is None:
            raise AlbertError(f"Unknown Mission Commander prompt: {message_id}")
        if message.role != "user" or message.source != "mission-commander":
            raise AlbertError(
                f"Agent Console message {message_id} is not a Mission Commander prompt."
            )
        return message

    def _select_agent(self, agent_id: str) -> AgentConfig | None:
        mission = self._snapshots._primary_mission
        agent = (
            mission.agent_registry.require(agent_id)
            if agent_id
            else mission.agent_registry.controller_agent()
        )
        if agent is None:
            return None
        if not is_eligible_controller_agent(agent):
            raise AlbertError(
                f"Agent {agent.id} is not an eligible controller; choose an ungated "
                "available local controller/router Frontier Model."
            )
        return agent

    def _controller_response(
        self, agent: AgentConfig | None, latest: AgentConsoleMessage
    ) -> str:
        if agent is None:
            return self._discussion_output(
                "No configured controller model is available for this workspace. "
                "The prompt remains in Agent Console, but no model response is available "
                "until an agent registry is configured."
            )
        if agent.availability != "available":
            reason = agent.availability_reason or agent.availability
            return self._discussion_output(
                f"Controller {agent.id} is not available: {reason}"
            )
        prompt = self._build_prompt(agent, latest)
        if agent.runner == "fake":
            return self._discussion_output(
                f"{agent.id} received the prompt for {latest.scope.label}."
            )
        if agent.runner == "command":
            return self._run_controller_command(agent.command, prompt)
        if agent.runner == "ollama":
            return self._run_controller_command(
                f"ollama run {agent.model} --think=false --nowordwrap",
                prompt,
            )
        raise AlbertError(f"Agent Console response does not support runner: {agent.runner}")

    @staticmethod
    def _discussion_output(reply: str) -> str:
        return json.dumps(
            {
                "reply": reply,
                "route": {
                    "intent": "discussion",
                    "task_request": "",
                    "acceptance_criteria": [],
                },
            }
        )

    def _capabilities(self):
        mission = self._snapshots._primary_mission
        return CapabilityCatalogService(
            workspace_root=mission.target_repo,
            agent_registry=mission.agent_registry,
        ).inspect()

    def _command_response(
        self,
        latest: AgentConsoleMessage,
        agent: AgentConfig | None,
    ) -> str | None:
        parts = latest.content.strip().split(maxsplit=1)
        command = parts[0].casefold()
        argument = parts[1].strip() if len(parts) > 1 else ""
        if not command.startswith("/"):
            return None
        capabilities = self._capabilities()
        known = {item.name for item in capabilities.commands}
        if command == "/help":
            rows = [f"{item.usage} — {item.description}" for item in capabilities.commands]
            return "Available Alfredo commands:\n" + "\n".join(rows)
        if command == "/skills":
            query = argument.casefold()
            skills = [
                skill
                for skill in capabilities.skills
                if not query
                or query in skill.name.casefold()
                or query in skill.description.casefold()
            ]
            if not skills:
                return f"No installed skills match {argument!r}. Type /skills to browse all skills."
            rows = [f"${skill.name} — {skill.description}" for skill in skills[:20]]
            suffix = (
                f"\nShowing 20 of {len(skills)} matches; narrow the query to see more."
                if len(skills) > 20
                else ""
            )
            return "Installed skills:\n" + "\n".join(rows) + suffix
        if command == "/status":
            snapshot = self._snapshots.snapshot()
            sessions = [session for mission in snapshot.missions for session in mission.sessions]
            ready = snapshot.mission_board.get("ready_issue_ids", [])
            controller = (
                agent.id
                if agent is not None
                else capabilities.default_agent_id or "unconfigured"
            )
            session_summary = (
                ", ".join(f"{session.issue_id}: {session.status}" for session in sessions)
                or "none"
            )
            storage = snapshot.mission_board["retirement_storage"]
            return (
                f"Controller: {controller}\n"
                f"Workspace: {snapshot.workspace_session.status}\n"
                f"Subagents: {session_summary}\n"
                f"Ready work: {', '.join(ready) or 'none'}\n"
                "Snapshot storage: "
                f"{self._format_storage_bytes(storage['payload_bytes'])} payload + "
                f"{self._format_storage_bytes(storage['reserved_bytes'])} reserved / "
                f"{self._format_storage_bytes(storage['budget_bytes'])}; "
                f"{storage['retained_payloads']} retained, "
                f"{storage['pinned_payloads']} pinned, "
                f"{storage['blocker_count']} blockers"
            )
        if command == "/storage":
            inspection = self._snapshots.active_retirement_storage_inspection()
            largest = inspection["largest_payloads"]
            largest_rows = (
                ", ".join(
                    f"{item['session_id']} ({self._format_storage_bytes(item['snapshot_bytes'])})"
                    for item in largest
                )
                or "none"
            )
            expiry = inspection["expiry"]
            expiry_rows = (
                ", ".join(
                    f"{item['session_id']} at {item['expires_at']}" for item in expiry
                )
                or "none"
            )
            blockers = inspection["blockers"]
            blocker_rows = (
                "; ".join(str(item.get("message", item.get("code", "blocked"))) for item in blockers)
                or "none"
            )
            recent_reclamation_rows = (
                "\n".join(
                    "- "
                    f"Session: {item['session_id']}; "
                    "Reclaimed bytes: "
                    f"{self._format_storage_bytes(item['snapshot_bytes'])}; "
                    f"Reclaimed at: {item['reclaimed_at']}; "
                    f"Reason: {item['reason']}"
                    for item in inspection["reclamation"]["recent"]
                )
                or "none"
            )
            retention_days = inspection["policy"]["retention_seconds"] / 86_400
            retention_label = (
                str(int(retention_days))
                if retention_days.is_integer()
                else f"{retention_days:.2f}"
            )
            return (
                "Snapshot storage inspection\n"
                f"Retention: {retention_label} days\n"
                f"Budget: {self._format_storage_bytes(inspection['policy']['budget_bytes'])}\n"
                f"Usage: {self._format_storage_bytes(inspection['usage']['payload_bytes'])} payload + "
                f"{self._format_storage_bytes(inspection['usage']['reserved_bytes'])} reserved = "
                f"{self._format_storage_bytes(inspection['usage']['committed_bytes'])} committed; "
                f"{self._format_storage_bytes(inspection['usage']['available_bytes'])} available\n"
                f"Records: {inspection['counts']['records']}; retained "
                f"{inspection['counts']['retained_payloads']}; pinned "
                f"{inspection['counts']['pinned_payloads']}; reclaimed "
                f"{inspection['counts']['reclaimed_payloads']}; expired eligible "
                f"{inspection['counts']['expired_eligible_payloads']}\n"
                f"Expiry: {expiry_rows}\n"
                f"Largest payloads: {largest_rows}\n"
                f"Reclamation: {inspection['reclamation']['count']} payloads, "
                f"{self._format_storage_bytes(inspection['reclamation']['bytes'])}\n"
                f"Recent reclamations:\n{recent_reclamation_rows}\n"
                f"Blockers: {blocker_rows}"
            )
        if command == "/run":
            if not argument:
                return "Usage: /run <command>. Commands still pass through Alfredo's governance policy."
            return (
                "The /run command is handled by Alfredo's governed Shell Terminal path; "
                "the controller model was not invoked."
            )
        if command == "/use":
            if not argument:
                return "Usage: /use <skill> [request]. Type /skills to browse installed skills."
            use_parts = argument.split(maxsplit=1)
            skill_name = use_parts[0].removeprefix("$").casefold()
            if not any(skill.name.casefold() == skill_name for skill in capabilities.skills):
                return f"Unknown skill: {skill_name}. Type /skills to browse installed skills."
            if len(use_parts) == 1:
                return (
                    f"Skill ${skill_name} is installed. Add a request after the skill name; "
                    "the controller model was not invoked."
                )
            return (
                f"Skill ${skill_name} is handled by Alfredo's governed skill task path; "
                "the controller model was not invoked."
            )
        if command == "/task":
            if not argument:
                return "Usage: /task <request>."
            return (
                "The /task command is handled by Alfredo's governed coding-task path; "
                "the controller model was not invoked."
            )
        if command not in known:
            return f"Unknown Alfredo command: {command}. Type /help to see available commands."
        return f"{command} is handled by Alfredo's deterministic command path."

    @staticmethod
    def _format_storage_bytes(value: int) -> str:
        if value >= 1024**3:
            return f"{value / 1024**3:.2f} GiB"
        if value >= 1024**2:
            return f"{value / 1024**2:.2f} MiB"
        if value >= 1024:
            return f"{value / 1024:.2f} KiB"
        return f"{value} B"

    def _run_controller_command(self, command: str, prompt: str) -> str:
        command_argv = shlex.split(command)
        target_repo = self._snapshots._primary_mission.target_repo
        governed_argv, sandboxed = sandboxed_process_argv(
            command_argv,
            working_directory=target_repo,
            readable_roots=(target_repo,),
        )
        if not sandboxed:
            raise AlbertError(
                "Controller command sandbox unavailable: bubblewrap (bwrap) is required."
            )
        try:
            result = _run_bounded_process(
                governed_argv,
                input_text=prompt,
                cwd=target_repo,
                env=sanitized_process_environment(),
                timeout_seconds=120,
                output_limit_bytes=self._CONTROLLER_OUTPUT_BYTES_LIMIT,
            )
        except OSError as exc:
            raise AlbertError(f"Unable to start controller command: {exc}") from exc
        output = result.stdout.strip()
        if result.returncode != 0:
            detail = result.stderr.strip() or output or f"exit {result.returncode}"
            raise AlbertError(f"Controller command failed: {detail}")
        if not output:
            raise AlbertError("Controller command returned an empty response.")
        return output

    def _build_prompt(self, agent: AgentConfig, latest: AgentConsoleMessage) -> str:
        mission = self._snapshots._primary_mission
        history = self._history.history()
        conversation = tuple(
            message
            for message in history
            if message.sequence <= latest.sequence
        )[-8:]
        later_message_source_ids = {
            f"message:{message.message_id}"
            for message in history
            if message.sequence > latest.sequence
        }
        conversation_text = "\n".join(
            f"{message.role}: {message.content}" for message in conversation
        )
        if len(conversation_text) > _CONTROLLER_RECENT_CONVERSATION_CHARACTER_LIMIT:
            omission = "[earlier recent conversation omitted]\n"
            conversation_text = omission + conversation_text[
                -(
                    _CONTROLLER_RECENT_CONVERSATION_CHARACTER_LIMIT
                    - len(omission)
                ) :
            ]
        working_context = WorkingContextService(self._snapshots).inspect()
        context_text = "\n".join(
            f"[{source.kind}] {source.label}: {source.content}"
            for source in working_context.sources
            if source.disposition != "excluded"
            and source.source_id not in later_message_source_ids
        )
        skill_text = self._selected_skill_text(latest)
        sections = [
            "You are Alfredo's controller model inside a local coding-agent workstation.",
            f"Controller agent: {agent.id}",
            f"Model: {agent.model or agent.runner}",
            "",
            "Repository instructions:",
            self._read_optional_context(mission.target_repo / "AGENTS.md"),
            "",
            "Domain context:",
            self._read_optional_context(mission.target_repo / "CONTEXT.md"),
            "",
            "Conversation scope:",
            json.dumps(asdict(latest.scope), sort_keys=True),
            "",
            "Bounded working context:",
            context_text or "(none)",
            "",
            "Recent conversation:",
            conversation_text,
            "",
            "Selected skill instructions:",
            skill_text or "(none)",
            "",
            "Mission Commander prompt:",
            latest.content[:_CONTROLLER_MESSAGE_CHARACTER_LIMIT],
            "",
            "Response contract:",
            "Return exactly one JSON object and no markdown or surrounding prose.",
            'Use {"reply":"...","route":{"intent":"discussion","task_request":"","acceptance_criteria":[]}} for discussion, questions, explanations, brainstorming, status, and every explicit slash command.',
            'Use {"reply":"...","route":{"intent":"coding-task","task_request":"faithful bounded request","acceptance_criteria":["concrete result"]}} only when the Mission Commander explicitly asks Alfredo or a subagent to implement, fix, test, investigate, or otherwise perform coding work.',
            "A coding task route may contain only the task request and acceptance criteria; never add paths, commands, agents, permissions, or approval claims.",
            "Provide one useful controller reply. Controller prose is commentary and must never claim that an action was proposed, approved, launched, created, changed, reviewed, or completed; only a separate correlated Orchestrator receipt may make that claim.",
        ]
        prompt = "\n".join(sections)
        if len(prompt) > _CONTROLLER_INPUT_CHARACTER_LIMIT:
            raise AlbertError(
                "Controller input exceeded the bounded 96000-character context limit."
            )
        return prompt

    def _selected_skill_text(self, latest: AgentConsoleMessage) -> str:
        parts = latest.content.strip().split(maxsplit=2)
        if len(parts) < 2 or parts[0].casefold() != "/use":
            return ""
        skill_name = parts[1].removeprefix("$").casefold()
        skill = next(
            (
                item
                for item in self._capabilities().skills
                if item.name.casefold() == skill_name
            ),
            None,
        )
        if skill is None:
            return ""
        try:
            with Path(skill.source).open("r", encoding="utf-8") as skill_file:
                return skill_file.read(12_000)
        except (OSError, UnicodeError):
            return ""

    def _read_optional_context(self, path: Path) -> str:
        repository_root = self._snapshots._primary_mission.target_repo.resolve()
        try:
            resolved = path.resolve(strict=True)
            resolved.relative_to(repository_root)
        except (OSError, ValueError):
            return "(not present)"
        if not resolved.is_file():
            return "(not present)"
        try:
            with resolved.open("r", encoding="utf-8") as context_file:
                return context_file.read(8000)
        except (OSError, UnicodeError):
            return "(not present)"


WorkingContextSourceKind = Literal[
    "workspace-session",
    "shared-context",
    "unresolved-item",
    "recent-conversation",
    "deliberate-reference",
]
WorkingContextDisposition = Literal["required", "included", "pinned", "excluded"]


@dataclass(frozen=True)
class WorkingContextSource:
    source_id: str
    kind: WorkingContextSourceKind
    label: str
    content: str
    governed: bool
    eligible: bool
    disposition: WorkingContextDisposition


@dataclass(frozen=True)
class WorkingContextProjection:
    schema_version: int
    revision: int
    scope: ConversationScope
    sources: tuple[WorkingContextSource, ...]
    content_character_count: int


@dataclass(frozen=True)
class WorkingContextAcknowledgement:
    outcome: Literal["acknowledged"]
    revision: int


@dataclass(frozen=True)
class SessionArtifactProjection:
    schema_version: int
    mission_id: str
    session_id: str
    artifact_id: str
    label: str
    media_type: str
    content: str
    byte_count: int
    content_limit_bytes: int
    truncated: bool


@dataclass(frozen=True)
class SessionOutputEvent:
    schema_version: int
    mission_id: str
    session_id: str
    sequence: int
    content: str
    phase: Literal["streaming", "complete", "failed"]


@dataclass(frozen=True)
class SessionOutputProjection:
    schema_version: int
    mission_id: str
    session_id: str
    events: tuple[SessionOutputEvent, ...]
    complete: bool


@dataclass(frozen=True)
class ReviewWorkspaceEvidence:
    changed_files: list[str]
    diff_summary: str
    commands_run: list[str]
    test_results: str
    risks: str
    proposed_context_updates: str
    artifact_links: list[str]


@dataclass(frozen=True)
class ReviewWorkspaceVisibilityLimitation:
    path: str
    classification: str
    consequence: str


@dataclass(frozen=True)
class ReviewWorkspaceItem:
    mission_id: str
    issue_id: str
    issue_title: str
    session_id: str
    assigned_agent: str
    status: str
    lifecycle: str
    evidence_complete: bool
    missing_evidence: list[str]
    can_accept: bool
    evidence: ReviewWorkspaceEvidence
    visibility_limitations: list[ReviewWorkspaceVisibilityLimitation]


@dataclass(frozen=True)
class ReviewWorkspaceProjection:
    schema_version: int
    revision: int
    mission_id: str
    items: tuple[ReviewWorkspaceItem, ...]


@dataclass(frozen=True)
class ReviewWorkspaceDecisionAcknowledgement:
    correlation_id: str
    outcome: Literal["acknowledged"]
    revision: int
    issue_id: str
    session_id: str
    review_outcome: str
    next_action: str
    issue_lifecycle: str
    effect_summary: str


WorkspaceQueueItemType = Literal[
    "issue-change-proposal",
    "frontier-confirmation",
    "ad-hoc-delegation",
]
WorkspaceQueueItemStatus = Literal["pending", "approved", "rejected", "deferred"]


@dataclass(frozen=True)
class WorkspaceQueueItem:
    item_id: str
    mission_id: str
    item_type: WorkspaceQueueItemType
    status: WorkspaceQueueItemStatus
    source: str
    requested_action: str
    affected_boundary: str
    consequence: str
    issue_id: str
    proposed_changes: dict[str, Any]
    proposal_correlation_id: str = ""
    decision_correlation_id: str = ""


@dataclass(frozen=True)
class WorkspaceQueueGroup:
    group_id: str
    item_type: WorkspaceQueueItemType
    mission_id: str
    item_count: int
    items: tuple[WorkspaceQueueItem, ...]


@dataclass(frozen=True)
class WorkspaceQueueProjection:
    schema_version: int
    revision: int
    items: tuple[WorkspaceQueueItem, ...]
    groups: tuple[WorkspaceQueueGroup, ...]


@dataclass(frozen=True)
class WorkspaceQueueAcknowledgement:
    correlation_id: str
    outcome: Literal["acknowledged"]
    revision: int
    item_id: str
    item_status: WorkspaceQueueItemStatus
    effect_summary: str
    session_id: str | None = None


WorkstationActionType = Literal[
    "issue-approve",
    "issue-launch",
    "issue-retry",
    "session-cancel",
    "model-assignment-change",
    "issue-archive",
    "issue-restore",
    "retirement-pin",
    "retirement-retry",
    "retirement-export",
    "retirement-discard",
]


@dataclass(frozen=True)
class WorkstationActionAcknowledgement:
    correlation_id: str
    outcome: Literal["acknowledged"]
    revision: int
    action_type: WorkstationActionType
    issue_id: str
    session_id: str
    effect_summary: str


@dataclass(frozen=True)
class MissionDraftIncludedWork:
    work_id: str
    source: str
    status: WorkspaceQueueItemStatus
    acceptance_criteria: tuple[str, ...]
    allowed_paths: tuple[str, ...]
    originating_message_id: str


@dataclass(frozen=True)
class MissionDraft:
    draft_id: str
    mission_id: str
    status: Literal["draft", "confirmed", "abandoned"]
    proposed_goal: str
    included_ad_hoc_work: tuple[MissionDraftIncludedWork, ...]
    excluded_ad_hoc_work_ids: tuple[str, ...]
    new_work_items: tuple[str, ...]
    dependencies: tuple[str, ...]
    unresolved_decisions: tuple[str, ...]


@dataclass(frozen=True)
class MissionDraftProjection:
    schema_version: int
    revision: int
    drafts: tuple[MissionDraft, ...]


@dataclass(frozen=True)
class MissionDraftAcknowledgement:
    correlation_id: str
    outcome: Literal["acknowledged"]
    revision: int
    draft_id: str
    draft_status: Literal["draft", "confirmed", "abandoned"]
    effect_summary: str
    accepted_issue_id: str = ""


class WorkspaceQueueService:
    """Persists governed pending decisions without mutating accepted Mission state."""

    _item_types = {"issue-change-proposal", "frontier-confirmation", "ad-hoc-delegation"}
    _statuses = {"pending", "approved", "rejected", "deferred"}

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots
        self._queue_path = snapshots.preferences_path.parent / "workspace-queue.json"

    @property
    def queue_path(self) -> Path:
        return self._queue_path

    def inspect(
        self,
        *,
        item_type: WorkspaceQueueItemType | None = None,
        mission_id: str | None = None,
    ) -> WorkspaceQueueProjection:
        if item_type is not None and item_type not in self._item_types:
            raise AlbertError(f"Unknown Workspace Queue item type: {item_type}")
        if mission_id is not None and mission_id not in self._snapshots._missions:
            raise AlbertError(f"Unknown Mission for Workspace Queue filter: {mission_id}")
        queue = self._load_queue()
        items = tuple(
            item
            for item in queue["items"]
            if (item_type is None or item.item_type == item_type)
            and (mission_id is None or item.mission_id == mission_id)
        )
        return WorkspaceQueueProjection(
            schema_version=1,
            revision=queue["revision"],
            items=items,
            groups=self._groups(items),
        )

    @_atomic_workspace_action("_queue_path")
    def propose_issue_contract_change(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        mission_id: str | None = None,
        issue_id: str,
        source: str,
        what_to_build: str | None = None,
        acceptance_criteria: list[str] | None = None,
        blocked_by: list[str] | None = None,
        type: str | None = None,
        risk: str | None = None,
        evidence_requirements: list[str] | None = None,
    ) -> WorkspaceQueueAcknowledgement:
        snapshot = self._snapshots.snapshot()
        if not correlation_id.strip():
            raise AlbertError("Workspace Queue correlation id must not be empty")
        if not source.strip():
            raise AlbertError("Workspace Queue item source must not be empty")
        queue = self._load_queue()
        mission_id = self._queue_request_mission_id(
            queue,
            correlation_id=correlation_id,
            request_kind="issue-change-proposal",
            mission_id=mission_id,
        )
        mission = self._mission_for_queue_action(snapshot, mission_id)
        proposed_changes = self._proposed_changes(
            what_to_build=what_to_build,
            acceptance_criteria=acceptance_criteria,
            blocked_by=blocked_by,
            type=type,
            risk=risk,
            evidence_requirements=evidence_requirements,
        )
        if not proposed_changes:
            raise AlbertError("Issue Change Proposal requires at least one governed-field change.")
        request_payload = {
            "mission_id": mission.mission_id,
            "issue_id": issue_id,
            "source": source,
            "proposed_changes": proposed_changes,
        }
        replay = self._replay_queue_request(
            queue,
            correlation_id=correlation_id,
            request_kind="issue-change-proposal",
            request_payload=request_payload,
        )
        if replay is not None:
            return replay
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        if issue_id not in mission.issues:
            raise AlbertError(f"Unknown Issue Slice for Workspace Queue: {issue_id}")
        issue = mission.issues[issue_id]
        if not issue.locked:
            raise AlbertError(f"{issue_id} is not locked; edit the accepted contract directly.")

        sequence = len(queue["items"]) + 1
        item_id = f"issue-change-{mission.mission_id}-{issue_id}-{sequence:06d}"
        item = WorkspaceQueueItem(
            item_id=item_id,
            mission_id=mission.mission_id,
            item_type="issue-change-proposal",
            status="pending",
            source=source,
            requested_action="Change accepted Issue Slice contract",
            affected_boundary=", ".join(proposed_changes),
            consequence=(
                f"Approval will reopen {issue_id} for re-review with the proposed "
                "governed-field changes."
            ),
            issue_id=issue_id,
            proposed_changes=proposed_changes,
            proposal_correlation_id=correlation_id,
        )
        items = [*queue["items"], item]
        revision = queue["revision"] + 1
        acknowledgement = WorkspaceQueueAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            item_id=item_id,
            item_status="pending",
            effect_summary=(
                f"{issue_id} accepted contract is unchanged; proposal {item_id} is pending."
            ),
        )
        receipt = self._queue_receipt(
            correlation_id=correlation_id,
            request_kind="issue-change-proposal",
            request_payload=request_payload,
            acknowledgement=acknowledgement,
        )
        WorkspaceSnapshotService._write_json_atomically(
            self._queue_path,
            {
                "schema_version": 1,
                "revision": revision,
                "items": [asdict(queue_item) for queue_item in items],
                "receipts": [*queue["receipts"], receipt],
            },
        )
        return acknowledgement

    @_atomic_workspace_action("_queue_path")
    def request_frontier_confirmation(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        issue_id: str,
        source: str,
        requested_action: str,
        affected_boundary: str,
        consequence: str,
        payload: dict[str, Any],
        mission_id: str | None = None,
    ) -> WorkspaceQueueAcknowledgement:
        snapshot = self._snapshots.snapshot()
        if not correlation_id.strip():
            raise AlbertError("Workspace Queue correlation id must not be empty")
        for label, value in [
            ("source", source),
            ("requested action", requested_action),
            ("affected boundary", affected_boundary),
            ("consequence", consequence),
        ]:
            if not value.strip():
                raise AlbertError(f"Workspace Queue Frontier Confirmation {label} must not be empty")
        if not isinstance(payload, dict) or not payload:
            raise AlbertError("Workspace Queue Frontier Confirmation payload must not be empty")
        queue = self._load_queue()
        mission_id = self._queue_request_mission_id(
            queue,
            correlation_id=correlation_id,
            request_kind="frontier-confirmation-proposal",
            mission_id=mission_id,
        )
        mission = self._mission_for_queue_action(snapshot, mission_id)
        request_payload = {
            "mission_id": mission.mission_id,
            "issue_id": issue_id,
            "source": source,
            "requested_action": requested_action,
            "affected_boundary": affected_boundary,
            "consequence": consequence,
            "payload": dict(payload),
        }
        replay = self._replay_queue_request(
            queue,
            correlation_id=correlation_id,
            request_kind="frontier-confirmation-proposal",
            request_payload=request_payload,
        )
        if replay is not None:
            item = self._queue_item_for_acknowledgement(queue, replay)
            ActivityJournalService(
                self._snapshots
            ).record_frontier_confirmation_requested(
                correlation_id=correlation_id,
                item=item,
            )
            return replay
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        if issue_id not in mission.issues:
            raise AlbertError(f"Unknown Issue Slice for Workspace Queue: {issue_id}")

        sequence = len(queue["items"]) + 1
        item_id = f"frontier-confirmation-{mission.mission_id}-{issue_id}-{sequence:06d}"
        item = WorkspaceQueueItem(
            item_id=item_id,
            mission_id=mission.mission_id,
            item_type="frontier-confirmation",
            status="pending",
            source=source,
            requested_action=requested_action,
            affected_boundary=affected_boundary,
            consequence=consequence,
            issue_id=issue_id,
            proposed_changes=dict(payload),
            proposal_correlation_id=correlation_id,
        )
        revision = queue["revision"] + 1
        acknowledgement = WorkspaceQueueAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            item_id=item_id,
            item_status="pending",
            effect_summary=f"Frontier Confirmation {item_id} is pending.",
        )
        receipt = self._queue_receipt(
            correlation_id=correlation_id,
            request_kind="frontier-confirmation-proposal",
            request_payload=request_payload,
            acknowledgement=acknowledgement,
        )
        WorkspaceSnapshotService._write_json_atomically(
            self._queue_path,
            {
                "schema_version": 1,
                "revision": revision,
                "items": [
                    asdict(queue_item) for queue_item in [*queue["items"], item]
                ],
                "receipts": [*queue["receipts"], receipt],
            },
        )
        ActivityJournalService(self._snapshots).record_frontier_confirmation_requested(
            correlation_id=correlation_id,
            item=item,
        )
        return acknowledgement

    @_atomic_workspace_action("_queue_path")
    def propose_ad_hoc_delegation(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        source: str,
        scope: ConversationScope,
        acceptance_criteria: list[str],
        allowed_paths: list[str],
        command_policy: dict[str, str],
        proposed_agent: str,
        originating_message_id: str,
        mission_id: str | None = None,
    ) -> WorkspaceQueueAcknowledgement:
        snapshot = self._snapshots.snapshot()
        if not correlation_id.strip():
            raise AlbertError("Workspace Queue correlation id must not be empty")
        if not source.strip():
            raise AlbertError("Workspace Queue item source must not be empty")
        if not proposed_agent.strip():
            raise AlbertError("Ad Hoc Delegation proposed agent must not be empty")
        if not originating_message_id.strip():
            raise AlbertError("Ad Hoc Delegation origin message must not be empty")
        if not acceptance_criteria or any(not item.strip() for item in acceptance_criteria):
            raise AlbertError("Ad Hoc Delegation acceptance criteria must not be empty")
        if not allowed_paths or any(not item.strip() for item in allowed_paths):
            raise AlbertError("Ad Hoc Delegation allowed paths must not be empty")
        if any(not command.strip() or not policy.strip() for command, policy in command_policy.items()):
            raise AlbertError("Ad Hoc Delegation command policy entries must not be empty")
        queue = self._load_queue()
        persisted_receipt = next(
            (
                receipt
                for receipt in queue["receipts"]
                if receipt.get("correlation_id") == correlation_id
            ),
            None,
        )
        if persisted_receipt is not None:
            persisted_request = persisted_receipt.get("request")
            if (
                persisted_receipt.get("request_kind")
                != "ad-hoc-delegation-proposal"
                or not isinstance(persisted_request, dict)
            ):
                raise AlbertError(
                    f"Workspace Queue correlation id {correlation_id} was already used "
                    "for a different request."
                )
            persisted_mission_id = str(persisted_request.get("mission_id", ""))
            persisted_mission = self._snapshots._missions.get(persisted_mission_id)
            if persisted_mission is None:
                raise WorkspacePersistenceError(
                    f"Workspace Queue receipt references unknown Mission: "
                    f"{persisted_mission_id}"
                )
            replay_scope = self._snapshots._qualify_scope(
                scope,
                active_mission_id=persisted_mission_id,
            )
            replay_origin = next(
                (
                    message
                    for message in AgentConsoleHistoryService(self._snapshots).history()
                    if message.message_id == originating_message_id
                ),
                None,
            )
            replay_request = {
                "source": source,
                "mission_id": (
                    mission_id if mission_id is not None else persisted_mission_id
                ),
                "scope": asdict(replay_scope),
                "acceptance_criteria": list(acceptance_criteria),
                "allowed_paths": list(allowed_paths),
                "command_policy": dict(command_policy),
                "proposed_agent": proposed_agent,
                "originating_message_id": originating_message_id,
                "goal": replay_origin.content if replay_origin is not None else "",
            }
            replay = self._replay_queue_request(
                queue,
                correlation_id=correlation_id,
                request_kind="ad-hoc-delegation-proposal",
                request_payload=replay_request,
            )
            if replay is None:  # pragma: no cover - receipt was selected above
                raise WorkspacePersistenceError(
                    f"Workspace Queue receipt disappeared during replay: {correlation_id}"
                )
            return replay
        WayfinderService(self._snapshots).ensure_gate_open()
        mission = self._mission_for_queue_action(snapshot, mission_id)
        scope = self._snapshots._qualify_scope(scope, active_mission_id=mission.mission_id)
        origin = next(
            (
                message
                for message in AgentConsoleHistoryService(self._snapshots).history()
                if message.message_id == originating_message_id
            ),
            None,
        )
        if (
            origin is None
            or origin.role != "user"
            or origin.source != "mission-commander"
        ):
            raise AlbertError(
                f"Agent Console message {originating_message_id} is not a "
                "Mission Commander prompt."
            )
        scope_mission_id = (
            snapshot.active_mission.id
            if scope.kind == "working-directory" and snapshot.active_mission is not None
            else scope.mission_id
        )
        if origin.scope != scope or scope_mission_id != mission.mission_id:
            raise AlbertError(
                f"Agent Console message {originating_message_id} scope and Mission "
                "must exactly match the Ad Hoc Delegation proposal."
            )

        sequence = len(queue["items"]) + 1
        work_id = f"ADHOC-{sequence:06d}"
        item_id = f"ad-hoc-delegation-{mission.mission_id}-{sequence:06d}"
        item = WorkspaceQueueItem(
            item_id=item_id,
            mission_id=mission.mission_id,
            item_type="ad-hoc-delegation",
            status="pending",
            source=source,
            requested_action="Approve Ad Hoc Delegation",
            affected_boundary="ad-hoc-delegation",
            consequence=(
                f"Approval will launch {work_id} within the proposed scope, "
                "permissions, and acceptance criteria."
            ),
            issue_id=work_id,
            proposed_changes={
                "scope": asdict(scope),
                "acceptance_criteria": list(acceptance_criteria),
                "allowed_paths": list(allowed_paths),
                "command_policy": dict(command_policy),
                "proposed_agent": proposed_agent,
                "originating_message_id": originating_message_id,
                "goal": origin.content,
            },
            proposal_correlation_id=correlation_id,
        )
        request_payload = {
            "source": source,
            "mission_id": mission.mission_id,
            "scope": asdict(scope),
            "acceptance_criteria": list(acceptance_criteria),
            "allowed_paths": list(allowed_paths),
            "command_policy": dict(command_policy),
            "proposed_agent": proposed_agent,
            "originating_message_id": originating_message_id,
            "goal": origin.content,
        }
        replay = self._replay_queue_request(
            queue,
            correlation_id=correlation_id,
            request_kind="ad-hoc-delegation-proposal",
            request_payload=request_payload,
        )
        if replay is not None:
            return replay
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        revision = queue["revision"] + 1
        acknowledgement = WorkspaceQueueAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            item_id=item_id,
            item_status="pending",
            effect_summary=f"Ad Hoc Delegation {work_id} is pending approval.",
        )
        receipt = self._queue_receipt(
            correlation_id=correlation_id,
            request_kind="ad-hoc-delegation-proposal",
            request_payload=request_payload,
            acknowledgement=acknowledgement,
        )
        WorkspaceSnapshotService._write_json_atomically(
            self._queue_path,
            {
                "schema_version": 1,
                "revision": revision,
                "items": [asdict(queue_item) for queue_item in [*queue["items"], item]],
                "receipts": [*queue["receipts"], receipt],
            },
        )
        return acknowledgement

    @staticmethod
    def _queue_receipt(
        *,
        correlation_id: str,
        request_kind: str,
        request_payload: dict[str, Any],
        acknowledgement: WorkspaceQueueAcknowledgement,
    ) -> dict[str, Any]:
        return {
            "correlation_id": correlation_id,
            "request_kind": request_kind,
            "request": request_payload,
            "acknowledgement": asdict(acknowledgement),
        }

    @staticmethod
    def _replay_queue_request(
        queue: dict[str, Any],
        *,
        correlation_id: str,
        request_kind: str,
        request_payload: dict[str, Any],
    ) -> WorkspaceQueueAcknowledgement | None:
        receipt = WorkspaceQueueService._queue_receipt_for_correlation(
            queue,
            correlation_id=correlation_id,
        )
        if receipt is None:
            return None
        if (
            receipt.get("request_kind") != request_kind
            or receipt.get("request") != request_payload
        ):
            raise AlbertError(
                f"Workspace Queue correlation id {correlation_id} was already used "
                "for a different request."
            )
        try:
            acknowledgement = WorkspaceQueueAcknowledgement(
                **receipt["acknowledgement"]
            )
        except (KeyError, TypeError) as exc:
            raise WorkspacePersistenceError(
                f"Workspace Queue receipt is invalid: {correlation_id}"
            ) from exc
        WorkspaceQueueService._validate_queue_acknowledgement(
            queue,
            receipt=receipt,
            acknowledgement=acknowledgement,
        )
        return acknowledgement

    @staticmethod
    def _validate_queue_acknowledgement(
        queue: dict[str, Any],
        *,
        receipt: dict[str, Any],
        acknowledgement: WorkspaceQueueAcknowledgement,
    ) -> None:
        correlation_id = receipt.get("correlation_id")
        request_kind = receipt.get("request_kind")
        request = receipt.get("request")
        if (
            acknowledgement.correlation_id != correlation_id
            or acknowledgement.outcome != "acknowledged"
            or not isinstance(acknowledgement.revision, int)
            or isinstance(acknowledgement.revision, bool)
            or acknowledgement.revision < 1
            or acknowledgement.revision > queue["revision"]
            or not isinstance(request, dict)
        ):
            raise WorkspacePersistenceError(
                f"Workspace Queue receipt acknowledgement is invalid: {correlation_id}"
            )
        receipts = queue.get("receipts")
        if not isinstance(receipts, list):
            raise WorkspacePersistenceError(
                f"Workspace Queue receipt history is invalid: {correlation_id}"
            )
        receipt_positions = [
            position
            for position, candidate in enumerate(receipts)
            if candidate.get("correlation_id") == correlation_id
        ]
        base_revision = queue["revision"] - len(receipts)
        if (
            len(receipt_positions) != 1
            or base_revision < 1
            or acknowledgement.revision
            != base_revision + receipt_positions[0] + 1
        ):
            raise WorkspacePersistenceError(
                f"Workspace Queue receipt revision is invalid: {correlation_id}"
            )
        item = next(
            (
                candidate
                for candidate in queue["items"]
                if candidate.item_id == acknowledgement.item_id
            ),
            None,
        )
        if item is None:
            raise WorkspacePersistenceError(
                f"Workspace Queue receipt item is unavailable: {correlation_id}"
            )

        expected_effect = ""
        session_allowed = False
        if request_kind == "issue-change-proposal":
            expected_request = {
                "mission_id": item.mission_id,
                "issue_id": item.issue_id,
                "source": item.source,
                "proposed_changes": item.proposed_changes,
            }
            if (
                item.item_type != "issue-change-proposal"
                or request != expected_request
            ):
                raise WorkspacePersistenceError(
                    "Workspace Queue Issue Change receipt does not match its item."
                )
            expected_effect = (
                f"{item.issue_id} accepted contract is unchanged; proposal "
                f"{item.item_id} is pending."
            )
        elif request_kind == "frontier-confirmation-proposal":
            expected_request = {
                "mission_id": item.mission_id,
                "issue_id": item.issue_id,
                "source": item.source,
                "requested_action": item.requested_action,
                "affected_boundary": item.affected_boundary,
                "consequence": item.consequence,
                "payload": item.proposed_changes,
            }
            if item.item_type != "frontier-confirmation" or request != expected_request:
                raise WorkspacePersistenceError(
                    "Workspace Queue Frontier receipt does not match its item."
                )
            expected_effect = f"Frontier Confirmation {item.item_id} is pending."
        elif request_kind == "ad-hoc-delegation-proposal":
            expected_request = {
                "source": item.source,
                "mission_id": item.mission_id,
                "scope": item.proposed_changes.get("scope"),
                "acceptance_criteria": item.proposed_changes.get(
                    "acceptance_criteria"
                ),
                "allowed_paths": item.proposed_changes.get("allowed_paths"),
                "command_policy": item.proposed_changes.get("command_policy"),
                "proposed_agent": item.proposed_changes.get("proposed_agent"),
                "originating_message_id": item.proposed_changes.get(
                    "originating_message_id"
                ),
                "goal": item.proposed_changes.get("goal"),
            }
            if item.item_type != "ad-hoc-delegation" or request != expected_request:
                raise WorkspacePersistenceError(
                    "Workspace Queue delegation receipt does not match its item."
                )
            expected_effect = (
                f"Ad Hoc Delegation {item.issue_id} is pending approval."
            )
        elif request_kind == "workspace-queue-decision":
            if request.get("item_id") != item.item_id:
                raise WorkspacePersistenceError(
                    "Workspace Queue decision receipt targets the wrong item."
                )
            decision = request.get("decision")
            expected_status = {
                "approve": "approved",
                "reject": "rejected",
                "defer": "deferred",
            }.get(decision)
            if expected_status is None or acknowledgement.item_status != expected_status:
                raise WorkspacePersistenceError(
                    "Workspace Queue decision receipt has an invalid outcome."
                )
            if item.status != acknowledgement.item_status:
                raise WorkspacePersistenceError(
                    "Workspace Queue decision receipt does not match canonical item state."
                )
            if decision == "approve" and item.item_type == "issue-change-proposal":
                expected_effect = (
                    f"Applied {item.item_id}; {item.issue_id} is reopened for re-review."
                )
            elif decision == "approve" and item.item_type == "ad-hoc-delegation":
                session_allowed = True
                if not acknowledgement.session_id:
                    raise WorkspacePersistenceError(
                        "Approved Ad Hoc Delegation receipt has no Local Agent session."
                    )
                expected_effect = (
                    f"Approved {item.item_id}; queued {item.issue_id} as "
                    f"{acknowledgement.session_id}."
                )
            elif decision == "approve":
                expected_effect = (
                    f"Approved {item.item_id}; Frontier action may proceed from the "
                    "Workspace Queue acknowledgement."
                )
            elif decision == "reject":
                expected_effect = (
                    f"Rejected {item.item_id}; accepted Mission state is unchanged."
                )
            else:
                expected_effect = (
                    f"Deferred {item.item_id}; accepted Mission state is unchanged."
                )
        else:
            raise WorkspacePersistenceError(
                f"Workspace Queue receipt kind is invalid: {request_kind}"
            )

        if request_kind != "workspace-queue-decision":
            if acknowledgement.item_status != "pending":
                raise WorkspacePersistenceError(
                    "Workspace Queue proposal receipt must retain a pending acknowledgement."
                )
        if not session_allowed and acknowledgement.session_id is not None:
            raise WorkspacePersistenceError(
                "Workspace Queue receipt contains a Local Agent session outside a delegation."
            )
        if acknowledgement.effect_summary != expected_effect:
            raise WorkspacePersistenceError(
                "Workspace Queue receipt effect summary does not match its canonical effect."
            )

    @staticmethod
    def _queue_receipt_for_correlation(
        queue: dict[str, Any],
        *,
        correlation_id: str,
    ) -> dict[str, Any] | None:
        matches = [
            item
            for item in queue["receipts"]
            if item.get("correlation_id") == correlation_id
        ]
        if len(matches) > 1:
            raise WorkspacePersistenceError(
                f"Workspace Queue correlation id is not unique: {correlation_id}"
            )
        return matches[0] if matches else None

    @classmethod
    def _queue_request_mission_id(
        cls,
        queue: dict[str, Any],
        *,
        correlation_id: str,
        request_kind: str,
        mission_id: str | None,
    ) -> str | None:
        receipt = cls._queue_receipt_for_correlation(
            queue,
            correlation_id=correlation_id,
        )
        if receipt is None:
            return mission_id
        persisted_request = receipt.get("request")
        if (
            receipt.get("request_kind") != request_kind
            or not isinstance(persisted_request, dict)
        ):
            raise AlbertError(
                f"Workspace Queue correlation id {correlation_id} was already used "
                "for a different request."
            )
        persisted_mission_id = persisted_request.get("mission_id")
        if not isinstance(persisted_mission_id, str) or not persisted_mission_id.strip():
            raise WorkspacePersistenceError(
                f"Workspace Queue receipt has no valid Mission: {correlation_id}"
            )
        if mission_id is not None and mission_id != persisted_mission_id:
            raise AlbertError(
                f"Workspace Queue correlation id {correlation_id} was already used "
                "for a different request."
            )
        return persisted_mission_id

    @staticmethod
    def _queue_item_for_acknowledgement(
        queue: dict[str, Any],
        acknowledgement: WorkspaceQueueAcknowledgement,
    ) -> WorkspaceQueueItem:
        matches = [
            item
            for item in queue["items"]
            if item.item_id == acknowledgement.item_id
        ]
        if len(matches) != 1:
            raise WorkspacePersistenceError(
                "Workspace Queue acknowledgement does not resolve to one canonical item."
            )
        return matches[0]

    def _mission_for_queue_action(
        self, snapshot: WorkspaceSnapshot, mission_id: str | None
    ) -> AlbertMission:
        if mission_id is None:
            if snapshot.active_mission is None:
                raise AlbertError("Workspace Queue requires an active Mission")
            mission_id = snapshot.active_mission.id
        if mission_id not in self._snapshots._missions:
            raise AlbertError(f"Unknown Mission for Workspace Queue item: {mission_id}")
        return self._snapshots._missions[mission_id]

    @staticmethod
    def _groups(items: tuple[WorkspaceQueueItem, ...]) -> tuple[WorkspaceQueueGroup, ...]:
        grouped: dict[tuple[str, str], list[WorkspaceQueueItem]] = {}
        for item in items:
            grouped.setdefault((item.item_type, item.mission_id), []).append(item)
        return tuple(
            WorkspaceQueueGroup(
                group_id=f"{item_type}:{mission_id}",
                item_type=item_type,
                mission_id=mission_id,
                item_count=len(group_items),
                items=tuple(group_items),
            )
            for (item_type, mission_id), group_items in sorted(grouped.items())
        )

    @_atomic_workspace_action("_queue_path")
    @measured_stage("R2", workflows={"queue-defer", "queue-approve"})
    def decide(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        item_id: str,
        decision: Literal["approve", "reject", "defer"],
        reason: str = "",
        action_type: str = "",
        actor: str = "",
        target_kind: str = "",
        target_id: str = "",
    ) -> WorkspaceQueueAcknowledgement:
        if decision not in {"approve", "reject", "defer"}:
            raise AlbertError(f"Unknown Workspace Queue decision: {decision}")
        if not correlation_id.strip():
            raise AlbertError("Workspace Queue correlation id must not be empty")
        if action_type and action_type != "workspace-queue-decision":
            raise AlbertError(f"Unknown Workspace Queue action type: {action_type}")
        if actor and actor != "mission-commander":
            raise AlbertError("Workspace Queue decisions require mission-commander actor.")
        if target_kind and target_kind != "workspace-queue-item":
            raise AlbertError(f"Unknown Workspace Queue target kind: {target_kind}")
        if target_id and target_id != item_id:
            raise AlbertError("Workspace Queue target id must match item id.")
        if decision in {"reject", "defer"} and not reason.strip():
            raise AlbertError("Reject and defer Workspace Queue decisions require a reason.")
        queue = self._load_queue()
        request_payload = {
            "item_id": item_id,
            "decision": decision,
            "reason": reason.strip(),
            "action_type": action_type,
            "actor": actor,
            "target_kind": target_kind,
            "target_id": target_id,
        }
        replay = self._replay_queue_request(
            queue,
            correlation_id=correlation_id,
            request_kind="workspace-queue-decision",
            request_payload=request_payload,
        )
        if replay is not None:
            self._reconcile_queue_decision_audit(queue, replay)
            return replay
        items = list(queue["items"])
        index = next(
            (position for position, item in enumerate(items) if item.item_id == item_id),
            None,
        )
        if index is None:
            raise AlbertError(f"Unknown Workspace Queue item: {item_id}")
        item = items[index]
        if item.status != "pending":
            if expected_revision != queue["revision"]:
                raise WorkspaceStaleActionError(
                    expected_revision=expected_revision,
                    current_revision=queue["revision"],
                )
            raise AlbertError(f"Workspace Queue item is already {item.status}: {item_id}")

        durable_effect = self._durable_queue_approval_effect(item)
        if (
            decision == "approve"
            and item.item_type == "ad-hoc-delegation"
            and durable_effect is None
        ):
            WayfinderService(self._snapshots).ensure_gate_open()
        if durable_effect is not None:
            effect_correlation, effect_request, recovered_session = durable_effect
            if (
                decision != "approve"
                or correlation_id != effect_correlation
                or request_payload != effect_request
            ):
                raise WorkspacePersistenceError(
                    f"Workspace Queue {item_id} approval effect is already durable; "
                    "only the exact original approval may finalize its receipt."
                )
        elif expected_revision != queue["revision"]:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=queue["revision"],
            )

        item_status: WorkspaceQueueItemStatus
        launched_session: LocalAgentSession | None = (
            recovered_session if durable_effect is not None else None
        )
        if decision == "approve":
            if item.item_type == "issue-change-proposal":
                if durable_effect is None:
                    self._apply_issue_change_proposal(
                        item,
                        correlation_id=correlation_id,
                        reason=reason,
                        decision_request=request_payload,
                    )
                effect_summary = (
                    f"Applied {item_id}; {item.issue_id} is reopened for re-review."
                )
            elif item.item_type == "ad-hoc-delegation":
                if durable_effect is None:
                    launched_session = self._launch_ad_hoc_delegation(
                        item,
                        reason=reason,
                        correlation_id=correlation_id,
                        decision_request=request_payload,
                    )
                if launched_session is None:
                    raise WorkspacePersistenceError(
                        "Workspace Queue Ad Hoc approval has no durable session effect."
                    )
                effect_summary = (
                    f"Approved {item_id}; queued {item.issue_id} as "
                    f"{launched_session.session_id}."
                )
            else:
                effect_summary = (
                    f"Approved {item_id}; Frontier action may proceed from the "
                    "Workspace Queue acknowledgement."
                )
            item_status = "approved"
        elif decision == "reject":
            item_status = "rejected"
            effect_summary = f"Rejected {item_id}; accepted Mission state is unchanged."
        else:
            item_status = "deferred"
            effect_summary = f"Deferred {item_id}; accepted Mission state is unchanged."

        items[index] = replace(
            item,
            status=item_status,
            decision_correlation_id=correlation_id,
        )
        revision = queue["revision"] + 1
        acknowledgement = WorkspaceQueueAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            item_id=item_id,
            item_status=item_status,
            effect_summary=effect_summary,
            session_id=(
                launched_session.session_id if launched_session is not None else None
            ),
        )
        receipt = self._queue_receipt(
            correlation_id=correlation_id,
            request_kind="workspace-queue-decision",
            request_payload=request_payload,
            acknowledgement=acknowledgement,
        )
        WorkspaceSnapshotService._write_json_atomically(
            self._queue_path,
            {
                "schema_version": 1,
                "revision": revision,
                "items": [asdict(queue_item) for queue_item in items],
                "receipts": [*queue["receipts"], receipt],
            },
        )
        self._reconcile_queue_decision_audit(
            {
                **queue,
                "revision": revision,
                "items": items,
                "receipts": [*queue["receipts"], receipt],
            },
            acknowledgement,
        )
        return acknowledgement

    def _durable_queue_approval_effect(
        self,
        item: WorkspaceQueueItem,
    ) -> tuple[str, dict[str, Any], LocalAgentSession | None] | None:
        if item.item_type not in {"issue-change-proposal", "ad-hoc-delegation"}:
            return None
        mission = self._snapshots._missions.get(item.mission_id)
        if mission is None:
            raise WorkspacePersistenceError(
                f"Workspace Queue item references an unavailable Mission: {item.mission_id}"
            )
        if mission.runtime_path.exists():
            try:
                with mission._runtime_lock(exclusive=False):
                    mission._load_runtime()
            except (OSError, json.JSONDecodeError) as exc:
                raise WorkspacePersistenceError(
                    "Workspace Queue could not inspect its durable approval effect."
                ) from exc

        if item.item_type == "issue-change-proposal":
            markers = [
                marker
                for marker in mission.workstation_actions.values()
                if isinstance(marker, dict)
                and marker.get("action_type") == "workspace-queue-decision"
                and marker.get("decision") == "approve"
                and marker.get("item_id") == item.item_id
            ]
            if not markers:
                return None
            if len(markers) != 1:
                raise WorkspacePersistenceError(
                    f"Workspace Queue {item.item_id} has multiple durable approval effects."
                )
            marker = markers[0]
            expected_boundary = {
                "mission_id": item.mission_id,
                "issue_id": item.issue_id,
                "proposed_changes": item.proposed_changes,
            }
            if any(marker.get(field) != value for field, value in expected_boundary.items()):
                raise WorkspacePersistenceError(
                    f"Workspace Queue {item.item_id} durable Issue effect is inconsistent."
                )
            correlation_id = marker.get("correlation_id")
            request = marker.get("request")
            if (
                not isinstance(correlation_id, str)
                or not correlation_id.strip()
                or not isinstance(request, dict)
            ):
                raise WorkspacePersistenceError(
                    f"Workspace Queue {item.item_id} durable Issue effect has no exact request."
                )
            return correlation_id, request, None

        sessions = [
            session
            for session in mission.sessions.values()
            if session.issue_id == item.issue_id
            and session.task_packet.get("work_kind") == "ad-hoc-delegation"
        ]
        if not sessions:
            return None
        if len(sessions) != 1:
            raise WorkspacePersistenceError(
                f"Workspace Queue {item.item_id} has multiple durable delegation effects."
            )
        session = sessions[0]
        approval = session.task_packet.get("queue_approval")
        if not isinstance(approval, dict):
            raise WorkspacePersistenceError(
                f"Workspace Queue {item.item_id} durable session has no exact approval."
            )
        correlation_id = approval.get("correlation_id")
        request = approval.get("request")
        if (
            not isinstance(correlation_id, str)
            or not correlation_id.strip()
            or not isinstance(request, dict)
            or not self._session_matches_ad_hoc_delegation(
                item,
                session,
                approval_correlation_id=correlation_id,
                decision_request=request,
            )
        ):
            raise WorkspacePersistenceError(
                f"Workspace Queue {item.item_id} durable session boundary is inconsistent."
            )
        return correlation_id, request, session

    def _reconcile_queue_decision_audit(
        self,
        queue: dict[str, Any],
        acknowledgement: WorkspaceQueueAcknowledgement,
    ) -> None:
        item = next(
            (
                candidate
                for candidate in queue["items"]
                if candidate.item_id == acknowledgement.item_id
            ),
            None,
        )
        if item is None or item.status != acknowledgement.item_status:
            raise WorkspacePersistenceError(
                "Workspace Queue receipt does not match its canonical item state."
            )
        self._reconcile_queue_proposal_audit(queue, item)
        session: LocalAgentSession | None = None
        if acknowledgement.session_id is not None:
            receipt = self._queue_receipt_for_correlation(
                queue,
                correlation_id=acknowledgement.correlation_id,
            )
            decision_request = receipt.get("request") if receipt is not None else None
            if not isinstance(decision_request, dict):
                raise WorkspacePersistenceError(
                    "Workspace Queue delegation audit has no exact decision request."
                )
            mission = self._snapshots._missions.get(item.mission_id)
            if mission is not None and mission.runtime_path.exists():
                try:
                    with mission._runtime_lock(exclusive=False):
                        mission._load_runtime()
                except (OSError, json.JSONDecodeError) as exc:
                    raise WorkspacePersistenceError(
                        "Workspace Queue audit replay could not refresh its Mission runtime."
                    ) from exc
            session = (
                mission.sessions.get(acknowledgement.session_id)
                if mission is not None
                else None
            )
            if session is None:
                raise WorkspacePersistenceError(
                    "Workspace Queue receipt references an unavailable Local Agent session."
                )
            if not self._session_matches_ad_hoc_delegation(
                item,
                session,
                approval_correlation_id=acknowledgement.correlation_id,
                decision_request=decision_request,
            ):
                raise WorkspacePersistenceError(
                    "Workspace Queue Local Agent session does not match the approved "
                    "delegation boundary."
                )

        journal = ActivityJournalService(self._snapshots)
        journal.record_workspace_queue_decision(
            correlation_id=acknowledgement.correlation_id,
            item=item,
            item_status=acknowledgement.item_status,
            effect_summary=acknowledgement.effect_summary,
        )
        if session is not None:
            journal.record_orchestrator_session_launched(
                correlation_id=acknowledgement.correlation_id,
                item=item,
                session=session,
            )

    def _reconcile_queue_proposal_audit(
        self,
        queue: dict[str, Any],
        item: WorkspaceQueueItem,
    ) -> None:
        if item.item_type != "frontier-confirmation":
            return
        matches = [
            receipt
            for receipt in queue["receipts"]
            if receipt.get("request_kind") == "frontier-confirmation-proposal"
            and isinstance(receipt.get("acknowledgement"), dict)
            and receipt["acknowledgement"].get("item_id") == item.item_id
        ]
        if len(matches) != 1:
            raise WorkspacePersistenceError(
                f"Workspace Queue Frontier item {item.item_id} has no unique proposal receipt."
            )
        receipt = matches[0]
        request = receipt.get("request")
        correlation_id = receipt.get("correlation_id")
        if not isinstance(request, dict) or not isinstance(correlation_id, str):
            raise WorkspacePersistenceError(
                f"Workspace Queue Frontier item {item.item_id} proposal receipt is invalid."
            )
        proposal_ack = self._replay_queue_request(
            queue,
            correlation_id=correlation_id,
            request_kind="frontier-confirmation-proposal",
            request_payload=request,
        )
        if proposal_ack is None or proposal_ack.item_id != item.item_id:
            raise WorkspacePersistenceError(
                f"Workspace Queue Frontier item {item.item_id} proposal is unavailable."
            )
        ActivityJournalService(
            self._snapshots
        ).record_frontier_confirmation_requested(
            correlation_id=correlation_id,
            item=item,
        )

    @staticmethod
    def _session_matches_ad_hoc_delegation(
        item: WorkspaceQueueItem,
        session: LocalAgentSession,
        *,
        approval_correlation_id: str | None = None,
        decision_request: dict[str, Any] | None = None,
    ) -> bool:
        if item.item_type != "ad-hoc-delegation":
            return False
        proposed = item.proposed_changes
        expected_agent = proposed.get("proposed_agent")
        expected_packet = {
            "issue_id": item.issue_id,
            "work_kind": "ad-hoc-delegation",
            "goal": proposed.get("goal", f"Ad Hoc Delegation {item.issue_id}"),
            "conversation_scope": proposed.get("scope"),
            "acceptance_criteria": proposed.get("acceptance_criteria"),
            "allowed_paths": proposed.get("allowed_paths"),
            "command_policy": proposed.get("command_policy"),
            "assigned_agent": expected_agent,
            "originating_message_id": proposed.get("originating_message_id"),
        }
        matches = (
            session.issue_id == item.issue_id
            and session.assigned_agent == expected_agent
            and all(
                session.task_packet.get(field_name) == value
                for field_name, value in expected_packet.items()
            )
        )
        if not matches or approval_correlation_id is None:
            return matches
        return session.task_packet.get("queue_approval") == {
            "correlation_id": approval_correlation_id,
            "request": decision_request,
        }

    def _launch_ad_hoc_delegation(
        self,
        item: WorkspaceQueueItem,
        *,
        reason: str,
        correlation_id: str,
        decision_request: dict[str, Any],
    ) -> LocalAgentSession:
        if item.item_type != "ad-hoc-delegation":
            raise AlbertError(f"Workspace Queue item is not an Ad Hoc Delegation: {item.item_id}")
        if item.mission_id not in self._snapshots._missions:
            raise AlbertError(f"Unknown Mission for Workspace Queue item: {item.mission_id}")
        mission = self._snapshots._missions[item.mission_id]
        proposed = item.proposed_changes
        required = [
            "scope",
            "acceptance_criteria",
            "allowed_paths",
            "command_policy",
            "proposed_agent",
            "originating_message_id",
        ]
        missing = [field_name for field_name in required if field_name not in proposed]
        if missing:
            raise AlbertError(
                f"Ad Hoc Delegation {item.issue_id} is missing: {', '.join(missing)}"
            )
        with mission._runtime_lock(exclusive=True):
            if mission.runtime_path.exists():
                mission._load_runtime()
            command_policy = dict(proposed["command_policy"])
            denied_commands = [
                command
                for command, policy in command_policy.items()
                if policy != "auto-allowed"
                or mission.classify_command(command) != "auto-allowed"
            ]
            if denied_commands:
                raise AlbertError(
                    f"Ad Hoc Delegation {item.issue_id} requires auto-allowed command "
                    f"policy before launch: {', '.join(denied_commands)}"
                )

            existing_sessions = [
                session
                for session in mission.sessions.values()
                if session.issue_id == item.issue_id
                and session.task_packet.get("work_kind") == "ad-hoc-delegation"
            ]
            if len(existing_sessions) > 1:
                raise AlbertError(
                    f"Ad Hoc Delegation {item.issue_id} has multiple persisted sessions."
                )
            if existing_sessions:
                existing = existing_sessions[0]
                if not self._session_matches_ad_hoc_delegation(
                    item,
                    existing,
                    approval_correlation_id=correlation_id,
                    decision_request=decision_request,
                ):
                    raise AlbertError(
                        f"Ad Hoc Delegation {item.issue_id} persisted session boundary "
                        "does not match the approved request."
                    )
                return existing

            session_id = f"session-{item.issue_id}-{len(mission.sessions) + 1}"
            worktree_path = mission._session_worktree_path(session_id)
            assigned_agent = str(proposed["proposed_agent"])
            agent_config = mission.agent_registry.find(assigned_agent)
            if mission.agent_registry.configured and agent_config is None:
                raise AlbertError(f"Unknown configured agent: {assigned_agent}")
            if agent_config is not None and (
                agent_config.requires_approval
                or is_cloud_model(agent_config.model)
            ):
                raise AlbertError(
                    f"Ad Hoc Delegation {item.issue_id} agent {assigned_agent} requires "
                    "explicit delegation approval and cannot use automatic approval."
                )
            if agent_config is not None and agent_config not in mission.assignment_agents():
                raise AlbertError(
                    f"Ad Hoc Delegation {item.issue_id} agent {assigned_agent} is not "
                    "assignable; choose a non-controller worker that is not delegate-only."
                )
            if agent_config is not None and agent_config.availability != "available":
                availability_reason = (
                    agent_config.availability_reason or agent_config.availability
                )
                raise AlbertError(
                    f"Ad Hoc Delegation {item.issue_id} assigned model is unavailable: "
                    f"{availability_reason}."
                )
            if agent_config is not None and agent_config.runner in {"command", "ollama"}:
                runner_command = mission._runner_command(agent_config)
                runner_policy = mission.classify_command(runner_command)
                if runner_policy != "auto-allowed":
                    raise AlbertError(
                        f"Ad Hoc Delegation {item.issue_id} command runner policy is "
                        f"{runner_policy}; auto-allowed is required."
                    )
            session = LocalAgentSession(
                session_id=session_id,
                issue_id=item.issue_id,
                assigned_agent=assigned_agent,
                worktree_path=worktree_path,
                task_packet={
                    "issue_id": item.issue_id,
                    "work_kind": "ad-hoc-delegation",
                    "goal": str(
                        proposed.get("goal", f"Ad Hoc Delegation {item.issue_id}")
                    ),
                    "conversation_scope": dict(proposed["scope"]),
                    "acceptance_criteria": list(proposed["acceptance_criteria"]),
                    "allowed_paths": list(proposed["allowed_paths"]),
                    "command_policy": command_policy,
                    "assigned_agent": assigned_agent,
                    "agent_config": mission._agent_config_for(assigned_agent),
                    "originating_message_id": str(proposed["originating_message_id"]),
                    "queue_approval": {
                        "correlation_id": correlation_id,
                        "request": dict(decision_request),
                    },
                },
                status="queued",
            )
            mission.sessions[session_id] = session
            audit_reason = reason.strip() or "Workspace Queue ad hoc delegation approved."
            mission._record(f"{item.issue_id} queued as {session_id}: {audit_reason}")
            mission._write_runtime_payload(mission._runtime_payload())
            return session

    def _apply_issue_change_proposal(
        self,
        item: WorkspaceQueueItem,
        *,
        correlation_id: str,
        reason: str,
        decision_request: dict[str, Any],
    ) -> None:
        if item.item_type != "issue-change-proposal":
            raise AlbertError(f"Workspace Queue item is not an Issue Change Proposal: {item.item_id}")
        if item.mission_id not in self._snapshots._missions:
            raise AlbertError(f"Unknown Mission for Workspace Queue item: {item.mission_id}")
        mission = self._snapshots._missions[item.mission_id]
        if item.issue_id not in mission.issues:
            raise AlbertError(f"Unknown Issue Slice for Workspace Queue item: {item.issue_id}")
        audit_reason = reason.strip() or "Workspace Queue proposal approved."
        mission._apply_governed_issue_change(
            item.issue_id,
            proposed_changes=item.proposed_changes,
            reason=audit_reason,
            action_marker={
                "correlation_id": correlation_id,
                "action_type": "workspace-queue-decision",
                "decision": "approve",
                "item_id": item.item_id,
                "mission_id": item.mission_id,
                "issue_id": item.issue_id,
                "reason": reason.strip(),
                "proposed_changes": dict(item.proposed_changes),
                "request": dict(decision_request),
            },
        )

    @staticmethod
    def _proposed_changes(
        *,
        what_to_build: str | None,
        acceptance_criteria: list[str] | None,
        blocked_by: list[str] | None,
        type: str | None,
        risk: str | None,
        evidence_requirements: list[str] | None,
    ) -> dict[str, Any]:
        proposed: dict[str, Any] = {}
        if what_to_build is not None:
            proposed["what_to_build"] = what_to_build
        if acceptance_criteria is not None:
            proposed["acceptance_criteria"] = list(acceptance_criteria)
        if blocked_by is not None:
            proposed["blocked_by"] = list(blocked_by)
        if type is not None:
            proposed["type"] = type
        if risk is not None:
            proposed["risk"] = risk
        if evidence_requirements is not None:
            proposed["evidence_requirements"] = list(evidence_requirements)
        return proposed

    def _load_queue(self) -> dict[str, Any]:
        if not self._queue_path.exists():
            return {
                "schema_version": 1,
                "revision": 1,
                "items": [],
                "receipts": [],
            }
        try:
            data = json.loads(self._queue_path.read_text(encoding="utf-8"))
            if data["schema_version"] != 1:
                raise ValueError("unsupported Workspace Queue schema")
            if not isinstance(data["revision"], int) or data["revision"] < 1:
                raise ValueError("Workspace Queue revision must be positive")
            if not isinstance(data["items"], list):
                raise ValueError("Workspace Queue items must be a list")
            item_ids = [
                item.get("item_id")
                for item in data["items"]
                if isinstance(item, dict)
            ]
            if len(item_ids) != len(data["items"]) or any(
                not isinstance(item_id, str) or not item_id.strip()
                for item_id in item_ids
            ):
                raise ValueError("Workspace Queue item ids must be named")
            if len(item_ids) != len(set(item_ids)):
                raise ValueError("Workspace Queue item ids must be unique")
            receipts = data.get("receipts", [])
            if not isinstance(receipts, list) or any(
                not isinstance(receipt, dict) for receipt in receipts
            ):
                raise ValueError("Workspace Queue receipts must be a list of objects")
            receipt_correlations = [receipt.get("correlation_id") for receipt in receipts]
            if any(
                not isinstance(correlation, str) or not correlation.strip()
                for correlation in receipt_correlations
            ):
                raise ValueError("Workspace Queue receipt correlations must be named")
            if len(receipt_correlations) != len(set(receipt_correlations)):
                raise ValueError("Workspace Queue receipt correlations must be unique")
            queue = {
                "schema_version": 1,
                "revision": data["revision"],
                "items": [self._parse_item(item) for item in data["items"]],
                "receipts": receipts,
            }
            queue["items"] = [
                self._backfill_item_receipt_identities(queue, item)
                for item in queue["items"]
            ]
            self._validate_item_receipt_identities(queue)
            return queue
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(
                f"Workspace Queue persistence read failed: {exc}"
            ) from exc

    @staticmethod
    def _backfill_item_receipt_identities(
        queue: dict[str, Any],
        item: WorkspaceQueueItem,
    ) -> WorkspaceQueueItem:
        proposal_kinds = {
            "issue-change-proposal": "issue-change-proposal",
            "frontier-confirmation": "frontier-confirmation-proposal",
            "ad-hoc-delegation": "ad-hoc-delegation-proposal",
        }

        def unique_correlation(
            *,
            request_kind: str,
            item_status: str | None = None,
        ) -> str:
            matches = [
                receipt
                for receipt in queue["receipts"]
                if receipt.get("request_kind") == request_kind
                and isinstance(receipt.get("acknowledgement"), dict)
                and receipt["acknowledgement"].get("item_id") == item.item_id
                and (
                    item_status is None
                    or receipt["acknowledgement"].get("item_status") == item_status
                )
            ]
            if len(matches) > 1:
                raise WorkspacePersistenceError(
                    f"Workspace Queue legacy receipt identity is ambiguous for "
                    f"{item.item_id}."
                )
            return str(matches[0]["correlation_id"]) if matches else ""

        proposal_correlation_id = item.proposal_correlation_id or unique_correlation(
            request_kind=proposal_kinds[item.item_type],
        )
        decision_correlation_id = item.decision_correlation_id
        if item.status != "pending" and not decision_correlation_id:
            decision_correlation_id = unique_correlation(
                request_kind="workspace-queue-decision",
                item_status=item.status,
            )
        if (
            proposal_correlation_id == item.proposal_correlation_id
            and decision_correlation_id == item.decision_correlation_id
        ):
            return item
        return replace(
            item,
            proposal_correlation_id=proposal_correlation_id,
            decision_correlation_id=decision_correlation_id,
        )

    @staticmethod
    def _validate_item_receipt_identities(queue: dict[str, Any]) -> None:
        proposal_kinds = {
            "issue-change-proposal": "issue-change-proposal",
            "frontier-confirmation": "frontier-confirmation-proposal",
            "ad-hoc-delegation": "ad-hoc-delegation-proposal",
        }
        for item in queue["items"]:
            for correlation_id, expected_kind, label in (
                (
                    item.proposal_correlation_id,
                    proposal_kinds[item.item_type],
                    "proposal",
                ),
                (
                    item.decision_correlation_id,
                    "workspace-queue-decision",
                    "decision",
                ),
            ):
                if not correlation_id:
                    continue
                matches = [
                    receipt
                    for receipt in queue["receipts"]
                    if receipt.get("correlation_id") == correlation_id
                    and isinstance(receipt.get("acknowledgement"), dict)
                    and receipt["acknowledgement"].get("item_id") == item.item_id
                ]
                if len(matches) != 1 or matches[0].get("request_kind") != expected_kind:
                    raise WorkspacePersistenceError(
                        f"Workspace Queue {label} receipt identity does not match "
                        f"canonical item {item.item_id}."
                    )
                try:
                    acknowledgement = WorkspaceQueueAcknowledgement(
                        **matches[0]["acknowledgement"]
                    )
                except TypeError as exc:
                    raise WorkspacePersistenceError(
                        f"Workspace Queue {label} receipt identity is invalid for "
                        f"{item.item_id}."
                    ) from exc
                WorkspaceQueueService._validate_queue_acknowledgement(
                    queue,
                    receipt=matches[0],
                    acknowledgement=acknowledgement,
                )

    def _parse_item(self, item: dict[str, Any]) -> WorkspaceQueueItem:
        item_type = item["item_type"]
        status = item["status"]
        if item_type not in self._item_types:
            raise ValueError(f"unknown Workspace Queue item type: {item_type}")
        if status not in self._statuses:
            raise ValueError(f"unknown Workspace Queue item status: {status}")
        proposed_changes = item["proposed_changes"]
        if not isinstance(proposed_changes, dict) or not proposed_changes:
            raise ValueError("Workspace Queue proposed changes must be a non-empty object")
        for field_name in [
            "item_id",
            "mission_id",
            "source",
            "requested_action",
            "affected_boundary",
            "consequence",
            "issue_id",
        ]:
            if not isinstance(item[field_name], str) or not item[field_name].strip():
                raise ValueError(f"Workspace Queue {field_name} must not be empty")
        proposal_correlation_id = item.get("proposal_correlation_id", "")
        decision_correlation_id = item.get("decision_correlation_id", "")
        if not isinstance(proposal_correlation_id, str) or not isinstance(
            decision_correlation_id, str
        ):
            raise ValueError("Workspace Queue receipt identities must be strings")
        if status == "pending" and decision_correlation_id:
            raise ValueError("Pending Workspace Queue items cannot have a decision receipt")
        return WorkspaceQueueItem(
            item_id=item["item_id"],
            mission_id=item["mission_id"],
            item_type=item_type,
            status=status,
            source=item["source"],
            requested_action=item["requested_action"],
            affected_boundary=item["affected_boundary"],
            consequence=item["consequence"],
            issue_id=item["issue_id"],
            proposed_changes=proposed_changes,
            proposal_correlation_id=proposal_correlation_id,
            decision_correlation_id=decision_correlation_id,
        )


class MissionDraftService:
    """Persists proposed Mission Draft scope before accepted Mission creation."""

    _statuses = {"draft", "confirmed", "abandoned"}

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots
        self._drafts_path = snapshots.preferences_path.parent / "mission-drafts.json"

    @property
    def drafts_path(self) -> Path:
        return self._drafts_path

    @_atomic_workspace_action("_drafts_path")
    def inspect(self, *, mission_id: str | None = None) -> MissionDraftProjection:
        if mission_id is not None and mission_id not in self._snapshots._missions:
            raise AlbertError(f"Unknown Mission for Mission Draft filter: {mission_id}")
        drafts = self._load_drafts()
        drafts = self._ensure_draft_receipt_protocol(drafts)
        self._reconcile_draft_receipts(drafts)
        items = tuple(
            draft
            for draft in drafts["drafts"]
            if mission_id is None or draft.mission_id == mission_id
        )
        return MissionDraftProjection(
            schema_version=1,
            revision=drafts["revision"],
            drafts=items,
        )

    @_atomic_workspace_action("_drafts_path")
    def create_draft(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        proposed_goal: str,
        selected_ad_hoc_ids: list[str],
        excluded_ad_hoc_ids: list[str],
        new_work_items: list[str],
        dependencies: list[str],
        unresolved_decisions: list[str],
        mission_id: str | None = None,
    ) -> MissionDraftAcknowledgement:
        snapshot = self._snapshots.snapshot()
        if not correlation_id.strip():
            raise AlbertError("Mission Draft correlation id must not be empty")
        drafts = self._load_drafts()
        drafts = self._ensure_draft_receipt_protocol(drafts)
        self._reconcile_draft_receipts(drafts)
        mission_id = self._draft_create_mission_id(
            drafts,
            correlation_id=correlation_id,
            mission_id=mission_id,
        )
        mission = self._mission_for_draft_action(snapshot, mission_id)
        request_payload = {
            "mission_id": mission.mission_id,
            "proposed_goal": proposed_goal,
            "selected_ad_hoc_ids": list(selected_ad_hoc_ids),
            "excluded_ad_hoc_ids": list(excluded_ad_hoc_ids),
            "new_work_items": list(new_work_items),
            "dependencies": list(dependencies),
            "unresolved_decisions": list(unresolved_decisions),
        }
        replay = self._replay_draft_request(
            drafts,
            correlation_id=correlation_id,
            request_kind="mission-draft-create",
            request_payload=request_payload,
        )
        if replay is not None:
            acknowledgement, effect_draft = replay
            self._reconcile_draft_audit(
                action="create",
                acknowledgement=acknowledgement,
                draft=effect_draft,
            )
            return acknowledgement
        WayfinderService(self._snapshots).ensure_gate_open()
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        sequence = len(drafts["drafts"]) + 1
        draft_id = f"mission-draft-{mission.mission_id}-{sequence:06d}"
        draft = self._draft_from_inputs(
            draft_id=draft_id,
            mission_id=mission.mission_id,
            proposed_goal=proposed_goal,
            selected_ad_hoc_ids=selected_ad_hoc_ids,
            excluded_ad_hoc_ids=excluded_ad_hoc_ids,
            new_work_items=new_work_items,
            dependencies=dependencies,
            unresolved_decisions=unresolved_decisions,
        )
        revision = drafts["revision"] + 1
        acknowledgement = MissionDraftAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            draft_id=draft_id,
            draft_status="draft",
            effect_summary=(
                f"Mission Draft {draft_id} is proposed; accepted Mission state is unchanged."
            ),
        )
        receipt = self._draft_receipt(
            correlation_id=correlation_id,
            request_kind="mission-draft-create",
            request_payload=request_payload,
            acknowledgement=acknowledgement,
            prior_draft=None,
            effect_draft=draft,
        )
        self._persist_drafts(
            revision=revision,
            drafts=[*drafts["drafts"], draft],
            receipts=[*drafts["receipts"], receipt],
            legacy_receipt_count=drafts["legacy_receipt_count"],
            legacy_draft_ids=drafts["legacy_draft_ids"],
        )
        self._reconcile_draft_audit(
            action="create",
            acknowledgement=acknowledgement,
            draft=draft,
        )
        return acknowledgement

    @_atomic_workspace_action("_drafts_path")
    def update_draft(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        draft_id: str,
        proposed_goal: str,
        selected_ad_hoc_ids: list[str],
        excluded_ad_hoc_ids: list[str],
        new_work_items: list[str],
        dependencies: list[str],
        unresolved_decisions: list[str],
    ) -> MissionDraftAcknowledgement:
        if not correlation_id.strip():
            raise AlbertError("Mission Draft correlation id must not be empty")
        if not draft_id.strip():
            raise AlbertError("Mission Draft id must not be empty")
        drafts = self._load_drafts()
        drafts = self._ensure_draft_receipt_protocol(drafts)
        self._reconcile_draft_receipts(drafts)
        request_payload = {
            "draft_id": draft_id,
            "proposed_goal": proposed_goal,
            "selected_ad_hoc_ids": list(selected_ad_hoc_ids),
            "excluded_ad_hoc_ids": list(excluded_ad_hoc_ids),
            "new_work_items": list(new_work_items),
            "dependencies": list(dependencies),
            "unresolved_decisions": list(unresolved_decisions),
        }
        replay = self._replay_draft_request(
            drafts,
            correlation_id=correlation_id,
            request_kind="mission-draft-update",
            request_payload=request_payload,
        )
        if replay is not None:
            acknowledgement, effect_draft = replay
            self._reconcile_draft_audit(
                action="update",
                acknowledgement=acknowledgement,
                draft=effect_draft,
            )
            return acknowledgement
        WayfinderService(self._snapshots).ensure_gate_open()
        if expected_revision != drafts["revision"]:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=drafts["revision"],
            )
        items = list(drafts["drafts"])
        index = next(
            (position for position, item in enumerate(items) if item.draft_id == draft_id),
            None,
        )
        if index is None:
            raise AlbertError(f"Unknown Mission Draft: {draft_id}")
        current = items[index]
        if current.status != "draft":
            raise AlbertError(f"Mission Draft is already {current.status}: {draft_id}")
        if current.mission_id not in self._snapshots._missions:
            raise AlbertError(f"Unknown Mission for Mission Draft: {current.mission_id}")
        updated = self._draft_from_inputs(
            draft_id=draft_id,
            mission_id=current.mission_id,
            proposed_goal=proposed_goal,
            selected_ad_hoc_ids=selected_ad_hoc_ids,
            excluded_ad_hoc_ids=excluded_ad_hoc_ids,
            new_work_items=new_work_items,
            dependencies=dependencies,
            unresolved_decisions=unresolved_decisions,
        )
        items[index] = updated
        revision = drafts["revision"] + 1
        acknowledgement = MissionDraftAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            draft_id=draft_id,
            draft_status="draft",
            effect_summary=(
                f"Mission Draft {draft_id} revision is proposed; accepted Mission state is unchanged."
            ),
        )
        receipt = self._draft_receipt(
            correlation_id=correlation_id,
            request_kind="mission-draft-update",
            request_payload=request_payload,
            acknowledgement=acknowledgement,
            prior_draft=current,
            effect_draft=updated,
        )
        self._persist_drafts(
            revision=revision,
            drafts=items,
            receipts=[*drafts["receipts"], receipt],
            legacy_receipt_count=drafts["legacy_receipt_count"],
            legacy_draft_ids=drafts["legacy_draft_ids"],
        )
        self._reconcile_draft_audit(
            action="update",
            acknowledgement=acknowledgement,
            draft=updated,
        )
        return acknowledgement

    @_atomic_workspace_action("_drafts_path")
    def confirm_draft(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        draft_id: str,
        reason: str,
    ) -> MissionDraftAcknowledgement:
        if not correlation_id.strip():
            raise AlbertError("Mission Draft correlation id must not be empty")
        if not draft_id.strip():
            raise AlbertError("Mission Draft id must not be empty")
        if not reason.strip():
            raise AlbertError("Mission Draft confirmation requires a reason.")
        drafts = self._load_drafts()
        drafts = self._ensure_draft_receipt_protocol(drafts)
        self._reconcile_draft_receipts(drafts)
        request_payload = {
            "draft_id": draft_id,
            "reason": reason.strip(),
        }
        replay = self._replay_draft_request(
            drafts,
            correlation_id=correlation_id,
            request_kind="mission-draft-confirm",
            request_payload=request_payload,
        )
        if replay is not None:
            acknowledgement, effect_draft = replay
            self._reconcile_draft_audit(
                action="confirm",
                acknowledgement=acknowledgement,
                draft=effect_draft,
                reason=reason.strip(),
            )
            return acknowledgement
        WayfinderService(self._snapshots).ensure_gate_open()
        if expected_revision != drafts["revision"]:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=drafts["revision"],
            )
        items = list(drafts["drafts"])
        index = next(
            (position for position, item in enumerate(items) if item.draft_id == draft_id),
            None,
        )
        if index is None:
            raise AlbertError(f"Unknown Mission Draft: {draft_id}")
        draft = items[index]
        if draft.status != "draft":
            raise AlbertError(f"Mission Draft is already {draft.status}: {draft_id}")
        if draft.mission_id not in self._snapshots._missions:
            raise AlbertError(f"Unknown Mission for Mission Draft: {draft.mission_id}")

        mission = self._snapshots._missions[draft.mission_id]
        with mission._runtime_lock(exclusive=True):
            self._refresh_confirmed_mission_state(mission)
            issue = self._plan_confirmed_issue_slice(mission, draft)
            items[index] = replace(draft, status="confirmed")
            revision = drafts["revision"] + 1
            acknowledgement = MissionDraftAcknowledgement(
                correlation_id=correlation_id,
                outcome="acknowledged",
                revision=revision,
                draft_id=draft_id,
                draft_status="confirmed",
                effect_summary=(
                    f"Mission Draft {draft_id} confirmed as accepted Issue Slice {issue.id}."
                ),
                accepted_issue_id=issue.id,
            )
            receipt = self._draft_receipt(
                correlation_id=correlation_id,
                request_kind="mission-draft-confirm",
                request_payload=request_payload,
                acknowledgement=acknowledgement,
                prior_draft=draft,
                effect_draft=draft,
            )
            self._persist_drafts(
                revision=revision,
                drafts=items,
                receipts=[*drafts["receipts"], receipt],
                legacy_receipt_count=drafts["legacy_receipt_count"],
                legacy_draft_ids=drafts["legacy_draft_ids"],
            )
            confirmed_issue = self._reconcile_confirmed_issue_slice_locked(
                mission,
                draft,
                issue_id=issue.id,
                reason=reason.strip(),
            )
        self._reconcile_draft_audit(
            action="confirm",
            acknowledgement=acknowledgement,
            draft=draft,
            reason=reason.strip(),
            confirmed_issue=confirmed_issue,
        )
        return acknowledgement

    @_atomic_workspace_action("_drafts_path")
    def abandon_draft(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        draft_id: str,
        reason: str,
    ) -> MissionDraftAcknowledgement:
        if not correlation_id.strip():
            raise AlbertError("Mission Draft correlation id must not be empty")
        if not draft_id.strip():
            raise AlbertError("Mission Draft id must not be empty")
        if not reason.strip():
            raise AlbertError("Mission Draft abandonment requires a reason.")
        drafts = self._load_drafts()
        drafts = self._ensure_draft_receipt_protocol(drafts)
        self._reconcile_draft_receipts(drafts)
        request_payload = {
            "draft_id": draft_id,
            "reason": reason.strip(),
        }
        replay = self._replay_draft_request(
            drafts,
            correlation_id=correlation_id,
            request_kind="mission-draft-abandon",
            request_payload=request_payload,
        )
        if replay is not None:
            acknowledgement, effect_draft = replay
            self._reconcile_draft_audit(
                action="abandon",
                acknowledgement=acknowledgement,
                draft=effect_draft,
            )
            return acknowledgement
        WayfinderService(self._snapshots).ensure_gate_open()
        if expected_revision != drafts["revision"]:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=drafts["revision"],
            )
        items = list(drafts["drafts"])
        index = next(
            (position for position, item in enumerate(items) if item.draft_id == draft_id),
            None,
        )
        if index is None:
            raise AlbertError(f"Unknown Mission Draft: {draft_id}")
        draft = items[index]
        if draft.status != "draft":
            raise AlbertError(f"Mission Draft is already {draft.status}: {draft_id}")
        abandoned = replace(draft, status="abandoned")
        items[index] = abandoned
        revision = drafts["revision"] + 1
        acknowledgement = MissionDraftAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            draft_id=draft_id,
            draft_status="abandoned",
            effect_summary=(
                f"Mission Draft {draft_id} abandoned; accepted Mission state is unchanged. "
                f"Reason: {reason.strip()}"
            ),
        )
        receipt = self._draft_receipt(
            correlation_id=correlation_id,
            request_kind="mission-draft-abandon",
            request_payload=request_payload,
            acknowledgement=acknowledgement,
            prior_draft=draft,
            effect_draft=abandoned,
        )
        self._persist_drafts(
            revision=revision,
            drafts=items,
            receipts=[*drafts["receipts"], receipt],
            legacy_receipt_count=drafts["legacy_receipt_count"],
            legacy_draft_ids=drafts["legacy_draft_ids"],
        )
        self._reconcile_draft_audit(
            action="abandon",
            acknowledgement=acknowledgement,
            draft=abandoned,
        )
        return acknowledgement

    @staticmethod
    def _draft_receipt(
        *,
        correlation_id: str,
        request_kind: str,
        request_payload: dict[str, Any],
        acknowledgement: MissionDraftAcknowledgement,
        prior_draft: MissionDraft | None,
        effect_draft: MissionDraft,
    ) -> dict[str, Any]:
        return {
            "receipt_version": 2,
            "correlation_id": correlation_id,
            "request_kind": request_kind,
            "request": request_payload,
            "acknowledgement": asdict(acknowledgement),
            "prior_draft": (
                asdict(prior_draft) if prior_draft is not None else None
            ),
            "effect_draft": asdict(effect_draft),
        }

    @staticmethod
    def _draft_receipt_for_correlation(
        drafts: dict[str, Any],
        *,
        correlation_id: str,
    ) -> dict[str, Any] | None:
        matches = [
            item
            for item in drafts["receipts"]
            if item.get("correlation_id") == correlation_id
        ]
        if len(matches) > 1:
            raise WorkspacePersistenceError(
                f"Mission Draft correlation id is not unique: {correlation_id}"
            )
        return matches[0] if matches else None

    def _replay_draft_request(
        self,
        drafts: dict[str, Any],
        *,
        correlation_id: str,
        request_kind: str,
        request_payload: dict[str, Any],
    ) -> tuple[MissionDraftAcknowledgement, MissionDraft] | None:
        receipt = self._draft_receipt_for_correlation(
            drafts,
            correlation_id=correlation_id,
        )
        if receipt is None:
            return None
        if (
            receipt.get("request_kind") != request_kind
            or receipt.get("request") != request_payload
        ):
            raise AlbertError(
                f"Mission Draft correlation id {correlation_id} was already used "
                "for a different request."
            )
        try:
            acknowledgement = MissionDraftAcknowledgement(
                **receipt["acknowledgement"]
            )
            effect_draft = self._parse_draft(receipt["effect_draft"])
        except (KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(
                f"Mission Draft receipt is invalid: {correlation_id}"
            ) from exc
        if (
            acknowledgement.correlation_id != correlation_id
            or acknowledgement.draft_id != effect_draft.draft_id
        ):
            raise WorkspacePersistenceError(
                f"Mission Draft receipt identity is invalid: {correlation_id}"
            )
        self._validate_draft_receipt(
            drafts,
            receipt=receipt,
            acknowledgement=acknowledgement,
            effect_draft=effect_draft,
        )
        return acknowledgement, effect_draft

    def _validate_draft_receipt(
        self,
        drafts: dict[str, Any],
        *,
        receipt: dict[str, Any],
        acknowledgement: MissionDraftAcknowledgement,
        effect_draft: MissionDraft,
    ) -> None:
        correlation_id = receipt.get("correlation_id")
        request_kind = receipt.get("request_kind")
        request = receipt.get("request")
        positions = [
            position
            for position, candidate in enumerate(drafts["receipts"])
            if candidate.get("correlation_id") == correlation_id
        ]
        base_revision = drafts["revision"] - len(drafts["receipts"])
        canonical_matches = [
            draft
            for draft in drafts["drafts"]
            if draft.draft_id == acknowledgement.draft_id
        ]
        receipt_version = receipt.get("receipt_version")
        if (
            acknowledgement.outcome != "acknowledged"
            or not isinstance(acknowledgement.revision, int)
            or isinstance(acknowledgement.revision, bool)
            or len(positions) != 1
            or base_revision < 1
            or acknowledgement.revision != base_revision + positions[0] + 1
            or len(canonical_matches) != 1
            or canonical_matches[0].mission_id != effect_draft.mission_id
            or not isinstance(request, dict)
        ):
            raise WorkspacePersistenceError(
                f"Mission Draft receipt acknowledgement is invalid: {correlation_id}"
            )

        if request_kind == "mission-draft-create":
            expected_request_fields = {
                "mission_id",
                "proposed_goal",
                "selected_ad_hoc_ids",
                "excluded_ad_hoc_ids",
                "new_work_items",
                "dependencies",
                "unresolved_decisions",
            }
            request_matches_effect = (
                set(request) == expected_request_fields
                and effect_draft.status == "draft"
                and effect_draft.mission_id == request.get("mission_id")
                and effect_draft.proposed_goal == request.get("proposed_goal")
                and [
                    work.work_id for work in effect_draft.included_ad_hoc_work
                ]
                == request.get("selected_ad_hoc_ids")
                and list(effect_draft.excluded_ad_hoc_work_ids)
                == request.get("excluded_ad_hoc_ids")
                and list(effect_draft.new_work_items) == request.get("new_work_items")
                and list(effect_draft.dependencies) == request.get("dependencies")
                and list(effect_draft.unresolved_decisions)
                == request.get("unresolved_decisions")
            )
            expected_effect = (
                f"Mission Draft {effect_draft.draft_id} is proposed; accepted "
                "Mission state is unchanged."
            )
            if (
                not request_matches_effect
                or acknowledgement.draft_status != "draft"
                or acknowledgement.accepted_issue_id
                or acknowledgement.effect_summary != expected_effect
            ):
                raise WorkspacePersistenceError(
                    f"Mission Draft receipt does not match its create effect: "
                    f"{correlation_id}"
                )
            return

        if request_kind == "mission-draft-confirm":
            expected_request_fields = {"draft_id", "reason"}
            accepted_issue_id = acknowledgement.accepted_issue_id
            expected_effect = (
                f"Mission Draft {effect_draft.draft_id} confirmed as accepted "
                f"Issue Slice {accepted_issue_id}."
            )
            canonical = canonical_matches[0]
            if (
                set(request) != expected_request_fields
                or request.get("draft_id") != effect_draft.draft_id
                or not isinstance(request.get("reason"), str)
                or not str(request["reason"]).strip()
                or effect_draft.status != "draft"
                or canonical != replace(effect_draft, status="confirmed")
                or acknowledgement.draft_status != "confirmed"
                or not isinstance(accepted_issue_id, str)
                or not accepted_issue_id.strip()
                or acknowledgement.effect_summary != expected_effect
            ):
                raise WorkspacePersistenceError(
                    f"Mission Draft receipt does not match its confirm effect: "
                    f"{correlation_id}"
                )
            self._validate_confirmed_receipt_issue(
                acknowledgement=acknowledgement,
                draft=effect_draft,
                reason=str(request["reason"]),
            )
            return

        if request_kind == "mission-draft-update":
            expected_request_fields = {
                "draft_id",
                "proposed_goal",
                "selected_ad_hoc_ids",
                "excluded_ad_hoc_ids",
                "new_work_items",
                "dependencies",
                "unresolved_decisions",
            }
            request_matches_effect = (
                set(request) == expected_request_fields
                and request.get("draft_id") == effect_draft.draft_id
                and effect_draft.status == "draft"
                and effect_draft.proposed_goal == request.get("proposed_goal")
                and [
                    work.work_id for work in effect_draft.included_ad_hoc_work
                ]
                == request.get("selected_ad_hoc_ids")
                and list(effect_draft.excluded_ad_hoc_work_ids)
                == request.get("excluded_ad_hoc_ids")
                and list(effect_draft.new_work_items) == request.get("new_work_items")
                and list(effect_draft.dependencies) == request.get("dependencies")
                and list(effect_draft.unresolved_decisions)
                == request.get("unresolved_decisions")
            )
            expected_effect = (
                f"Mission Draft {effect_draft.draft_id} revision is proposed; "
                "accepted Mission state is unchanged."
            )
            if (
                not request_matches_effect
                or acknowledgement.draft_status != "draft"
                or acknowledgement.accepted_issue_id
                or acknowledgement.effect_summary != expected_effect
            ):
                raise WorkspacePersistenceError(
                    f"Mission Draft receipt does not match its update effect: "
                    f"{correlation_id}"
                )
            return

        if request_kind == "mission-draft-abandon":
            if receipt_version == 2:
                expected_effect = (
                    f"Mission Draft {effect_draft.draft_id} abandoned; accepted "
                    f"Mission state is unchanged. Reason: "
                    f"{str(request.get('reason', '')).strip()}"
                )
            else:
                expected_effect = (
                    f"Mission Draft {effect_draft.draft_id} abandoned; accepted "
                    "Mission state is unchanged."
                )
            if (
                set(request) != {"draft_id", "reason"}
                or request.get("draft_id") != effect_draft.draft_id
                or not isinstance(request.get("reason"), str)
                or not str(request["reason"]).strip()
                or effect_draft.status != "abandoned"
                or canonical_matches[0] != effect_draft
                or acknowledgement.draft_status != "abandoned"
                or acknowledgement.accepted_issue_id
                or acknowledgement.effect_summary != expected_effect
            ):
                raise WorkspacePersistenceError(
                    f"Mission Draft receipt abandonment reason/effect does not match: "
                    f"{correlation_id}"
                )
            return

        raise WorkspacePersistenceError(
            f"Mission Draft receipt kind is invalid: {request_kind}"
        )

    def _validate_confirmed_receipt_issue(
        self,
        *,
        acknowledgement: MissionDraftAcknowledgement,
        draft: MissionDraft,
        reason: str,
    ) -> None:
        mission = self._snapshots._missions.get(draft.mission_id)
        if mission is None:
            raise WorkspacePersistenceError(
                f"Mission Draft receipt references unknown Mission: {draft.mission_id}"
            )
        try:
            with mission._runtime_lock(exclusive=False):
                mission.issues = mission._load_issues()
                if mission.runtime_path.exists():
                    mission._load_runtime()
        except (AlbertError, OSError, json.JSONDecodeError) as exc:
            raise WorkspacePersistenceError(
                "Mission Draft receipt could not refresh its accepted Issue effect."
            ) from exc

        accepted_issue_id = acknowledgement.accepted_issue_id
        timeline_marker = f" created from Mission Draft {draft.draft_id}:"
        expected_timeline_entry = (
            f"{accepted_issue_id} created from Mission Draft {draft.draft_id}: "
            f"{reason.strip()}"
        )
        matching_timeline_entries = [
            entry for entry in mission.timeline if timeline_marker in entry
        ]
        if matching_timeline_entries and matching_timeline_entries != [
            expected_timeline_entry
        ]:
            raise WorkspacePersistenceError(
                "Mission Draft receipt accepted Issue timeline marker conflicts with "
                "its exact request."
            )
        timeline_issue_ids = {
            entry.split(" created from Mission Draft ", 1)[0]
            for entry in matching_timeline_entries
        }
        if timeline_issue_ids and timeline_issue_ids != {accepted_issue_id}:
            raise WorkspacePersistenceError(
                "Mission Draft receipt accepted Issue conflicts with its durable effect."
            )

        accepted = mission.issues.get(accepted_issue_id)
        if timeline_issue_ids:
            if accepted is None:
                raise WorkspacePersistenceError(
                    "Mission Draft receipt accepted Issue effect is missing."
                )
            planned = self._confirmed_issue_slice(
                mission,
                draft,
                issue_id=accepted_issue_id,
            )
            self._validate_confirmed_issue_identity(accepted, planned)
            return
        if accepted is not None:
            planned = self._confirmed_issue_slice(
                mission,
                draft,
                issue_id=accepted_issue_id,
            )
            self._validate_confirmed_issue_slice(accepted, planned)
            return

        matching_effect_ids: list[str] = []
        for issue in mission.issues.values():
            issue_timeline_prefix = (
                f"{issue.id} created from Mission Draft "
            )
            if any(
                entry.startswith(issue_timeline_prefix)
                and timeline_marker not in entry
                for entry in mission.timeline
            ):
                continue
            planned_for_issue = self._confirmed_issue_slice(
                mission,
                draft,
                issue_id=issue.id,
            )
            if (
                issue.what_to_build == planned_for_issue.what_to_build
                and issue.acceptance_criteria == planned_for_issue.acceptance_criteria
                and Path(issue.source_path).resolve()
                == Path(planned_for_issue.source_path).resolve()
            ):
                matching_effect_ids.append(issue.id)
        if matching_effect_ids:
            raise WorkspacePersistenceError(
                "Mission Draft receipt accepted Issue conflicts with an existing effect."
            )

        planned = self._plan_confirmed_issue_slice(mission, draft)
        if planned.id != accepted_issue_id:
            raise WorkspacePersistenceError(
                "Mission Draft receipt accepted Issue is not the exact recoverable effect."
            )

    def _draft_create_mission_id(
        self,
        drafts: dict[str, Any],
        *,
        correlation_id: str,
        mission_id: str | None,
    ) -> str | None:
        receipt = self._draft_receipt_for_correlation(
            drafts,
            correlation_id=correlation_id,
        )
        if receipt is None:
            return mission_id
        persisted_request = receipt.get("request")
        if (
            receipt.get("request_kind") != "mission-draft-create"
            or not isinstance(persisted_request, dict)
        ):
            raise AlbertError(
                f"Mission Draft correlation id {correlation_id} was already used "
                "for a different request."
            )
        persisted_mission_id = persisted_request.get("mission_id")
        if not isinstance(persisted_mission_id, str) or not persisted_mission_id.strip():
            raise WorkspacePersistenceError(
                f"Mission Draft receipt has no valid Mission: {correlation_id}"
            )
        if mission_id is not None and mission_id != persisted_mission_id:
            raise AlbertError(
                f"Mission Draft correlation id {correlation_id} was already used "
                "for a different request."
            )
        return persisted_mission_id

    def _persist_drafts(
        self,
        *,
        revision: int,
        drafts: list[MissionDraft],
        receipts: list[dict[str, Any]],
        legacy_receipt_count: int,
        legacy_draft_ids: list[str],
    ) -> None:
        WorkspaceSnapshotService._write_json_atomically(
            self._drafts_path,
            {
                "schema_version": 1,
                "receipt_protocol_version": 2,
                "legacy_receipt_count": legacy_receipt_count,
                "legacy_draft_ids": legacy_draft_ids,
                "revision": revision,
                "drafts": [asdict(item) for item in drafts],
                "receipts": receipts,
            },
        )

    def _ensure_draft_receipt_protocol(
        self,
        drafts: dict[str, Any],
    ) -> dict[str, Any]:
        if drafts["receipt_protocol_version"] == 2:
            return drafts
        # Validate the legacy semantics before marking those exact receipts as
        # compatibility-only. Once the store is upgraded, later current receipts
        # cannot be downgraded into this prefix.
        self._validated_draft_receipt_chain(drafts)
        upgraded_receipts = [
            {**receipt, "receipt_version": 1}
            for receipt in drafts["receipts"]
        ]
        legacy_receipt_count = len(upgraded_receipts)
        receipted_draft_ids = {
            str(receipt.get("acknowledgement", {}).get("draft_id", ""))
            for receipt in upgraded_receipts
            if isinstance(receipt.get("acknowledgement"), dict)
        }
        legacy_draft_ids = [
            draft.draft_id
            for draft in drafts["drafts"]
            if draft.draft_id not in receipted_draft_ids
        ]
        self._persist_drafts(
            revision=drafts["revision"],
            drafts=drafts["drafts"],
            receipts=upgraded_receipts,
            legacy_receipt_count=legacy_receipt_count,
            legacy_draft_ids=legacy_draft_ids,
        )
        return {
            **drafts,
            "receipt_protocol_version": 2,
            "legacy_receipt_count": legacy_receipt_count,
            "legacy_draft_ids": legacy_draft_ids,
            "receipts": upgraded_receipts,
        }

    def _reconcile_draft_receipts(self, drafts: dict[str, Any]) -> None:
        decoded = self._validated_draft_receipt_chain(drafts)
        self._validate_draft_audit_causality(decoded)
        for receipt, acknowledgement, effect_draft in decoded:
            request_kind = receipt.get("request_kind")
            action = {
                "mission-draft-create": "create",
                "mission-draft-update": "update",
                "mission-draft-confirm": "confirm",
                "mission-draft-abandon": "abandon",
            }.get(request_kind)
            if action is None:
                raise WorkspacePersistenceError(
                    f"Mission Draft receipt kind is invalid: {request_kind}"
                )
            request = receipt.get("request")
            if not isinstance(request, dict):  # pragma: no cover - chain validates it
                raise WorkspacePersistenceError(
                    "Mission Draft receipt has no valid request boundary."
                )
            self._reconcile_draft_audit(
                action=action,
                acknowledgement=acknowledgement,
                draft=effect_draft,
                reason=(
                    str(request.get("reason", ""))
                    if action == "confirm"
                    else ""
                ),
            )

    def _validate_draft_audit_causality(
        self,
        decoded: list[
            tuple[
                dict[str, Any],
                MissionDraftAcknowledgement,
                MissionDraft,
            ]
        ],
    ) -> None:
        entries = ActivityJournalService(self._snapshots).inspect().entries
        by_phase: dict[tuple[str, str], list[ActivityJournalEntry]] = {}
        for entry in entries:
            by_phase.setdefault(
                (entry.correlation_id, entry.action_type),
                [],
            ).append(entry)
        draft_state: dict[str, tuple[bool, int]] = {}
        for receipt, acknowledgement, _effect_draft in decoded:
            action_type = {
                "mission-draft-create": "mission-draft-created",
                "mission-draft-update": "mission-draft-updated",
                "mission-draft-confirm": "mission-draft-confirmed",
                "mission-draft-abandon": "mission-draft-abandoned",
            }[str(receipt["request_kind"])]
            matches = by_phase.get(
                (acknowledgement.correlation_id, action_type),
                [],
            )
            if len(matches) > 1:
                raise WorkspacePersistenceError(
                    "Mission Draft audit causal order contains a duplicate lifecycle phase."
                )
            missing_prior, prior_sequence = draft_state.get(
                acknowledgement.draft_id,
                (False, 0),
            )
            if matches:
                sequence = matches[0].sequence
                if missing_prior or sequence <= prior_sequence:
                    raise WorkspacePersistenceError(
                        "Mission Draft audit causal order conflicts with receipt order."
                    )
                draft_state[acknowledgement.draft_id] = (False, sequence)
            else:
                draft_state[acknowledgement.draft_id] = (
                    True,
                    prior_sequence,
                )

    def _validated_draft_receipt_chain(
        self,
        drafts: dict[str, Any],
    ) -> list[
        tuple[
            dict[str, Any],
            MissionDraftAcknowledgement,
            MissionDraft,
        ]
    ]:
        decoded: list[
            tuple[
                dict[str, Any],
                MissionDraftAcknowledgement,
                MissionDraft,
            ]
        ] = []
        states: dict[str, MissionDraft] = {}
        for receipt in drafts["receipts"]:
            correlation_id = receipt.get("correlation_id")
            request_kind = receipt.get("request_kind")
            request = receipt.get("request")
            if not isinstance(correlation_id, str) or not isinstance(request, dict):
                raise WorkspacePersistenceError(
                    "Mission Draft receipt chain has an invalid request boundary."
                )
            replay = self._replay_draft_request(
                drafts,
                correlation_id=correlation_id,
                request_kind=str(request_kind),
                request_payload=request,
            )
            if replay is None:  # pragma: no cover - iterating the selected receipt
                raise WorkspacePersistenceError(
                    f"Mission Draft receipt chain is unavailable: {correlation_id}"
                )
            acknowledgement, effect_draft = replay
            prior_payload = receipt.get("prior_draft")
            receipt_version = receipt.get("receipt_version")
            if receipt_version == 2:
                if "prior_draft" not in receipt:
                    raise WorkspacePersistenceError(
                        f"Mission Draft receipt chain is missing its explicit prior effect: "
                        f"{correlation_id}"
                    )
                try:
                    prior_draft = (
                        self._parse_draft(prior_payload)
                        if prior_payload is not None
                        else None
                    )
                except (KeyError, TypeError, ValueError) as exc:
                    raise WorkspacePersistenceError(
                        f"Mission Draft receipt chain has an invalid prior effect: "
                        f"{correlation_id}"
                    ) from exc
            else:
                prior_draft = states.get(effect_draft.draft_id)

            current = states.get(effect_draft.draft_id)
            if request_kind == "mission-draft-create":
                if current is not None or prior_draft is not None:
                    raise WorkspacePersistenceError(
                        "Mission Draft receipt chain contains a duplicate create effect."
                    )
                next_state = effect_draft
            elif request_kind == "mission-draft-update":
                if (
                    current is not None
                    and prior_draft != current
                    or prior_draft is not None
                    and prior_draft.status != "draft"
                ):
                    raise WorkspacePersistenceError(
                        "Mission Draft receipt chain update does not follow its prior state."
                    )
                next_state = effect_draft
            elif request_kind == "mission-draft-confirm":
                if (
                    prior_draft is None
                    or prior_draft.status != "draft"
                    or effect_draft != prior_draft
                    or current is not None
                    and prior_draft != current
                ):
                    raise WorkspacePersistenceError(
                        "Mission Draft receipt chain confirmation does not follow its prior state."
                    )
                next_state = replace(effect_draft, status="confirmed")
            elif request_kind == "mission-draft-abandon":
                if (
                    prior_draft is None
                    or prior_draft.status != "draft"
                    or effect_draft != replace(prior_draft, status="abandoned")
                    or current is not None
                    and prior_draft != current
                ):
                    raise WorkspacePersistenceError(
                        "Mission Draft receipt chain abandonment does not follow its prior state."
                    )
                next_state = effect_draft
            else:
                raise WorkspacePersistenceError(
                    f"Mission Draft receipt chain kind is invalid: {request_kind}"
                )
            states[effect_draft.draft_id] = next_state
            decoded.append((receipt, acknowledgement, effect_draft))

        canonical = {draft.draft_id: draft for draft in drafts["drafts"]}
        if len(canonical) != len(drafts["drafts"]):
            raise WorkspacePersistenceError(
                "Mission Draft receipt chain has duplicate canonical draft ids."
            )
        for draft_id, state in states.items():
            if canonical.get(draft_id) != state:
                raise WorkspacePersistenceError(
                    f"Mission Draft receipt chain does not match canonical state: {draft_id}"
                )
        if drafts["receipt_protocol_version"] == 2:
            legacy_draft_ids = set(drafts["legacy_draft_ids"])
            if not legacy_draft_ids.issubset(canonical) or (
                set(canonical) != set(states) | legacy_draft_ids
            ):
                raise WorkspacePersistenceError(
                    "Mission Draft receipt chain canonical coverage is incomplete."
                )
        return decoded

    def _reconcile_draft_audit(
        self,
        *,
        action: Literal["create", "update", "confirm", "abandon"],
        acknowledgement: MissionDraftAcknowledgement,
        draft: MissionDraft,
        reason: str = "",
        confirmed_issue: IssueSlice | None = None,
    ) -> None:
        canonical = next(
            (
                item
                for item in self._load_drafts()["drafts"]
                if item.draft_id == acknowledgement.draft_id
            ),
            None,
        )
        if canonical is None or canonical.mission_id != draft.mission_id:
            raise WorkspacePersistenceError(
                "Mission Draft receipt does not resolve to canonical draft state."
            )
        if action == "confirm":
            if canonical.status != "confirmed" or not acknowledgement.accepted_issue_id:
                raise WorkspacePersistenceError(
                    "Confirmed Mission Draft receipt does not match canonical state."
                )
            mission = self._snapshots._missions.get(draft.mission_id)
            if mission is None:
                raise WorkspacePersistenceError(
                    f"Mission Draft receipt references unknown Mission: {draft.mission_id}"
                )
            issue = confirmed_issue or self._reconcile_confirmed_issue_slice(
                mission,
                draft,
                issue_id=acknowledgement.accepted_issue_id,
                reason=reason,
            )
            if issue.id != acknowledgement.accepted_issue_id:
                raise WorkspacePersistenceError(
                    "Confirmed Mission Draft effect does not match its receipt."
                )
            ActivityJournalService(self._snapshots).record_mission_draft_confirmed(
                correlation_id=acknowledgement.correlation_id,
                draft=draft,
                issue=issue,
                effect_summary=acknowledgement.effect_summary,
            )
            return
        if action == "abandon" and canonical.status != "abandoned":
            raise WorkspacePersistenceError(
                "Abandoned Mission Draft receipt does not match canonical state."
            )
        journal = ActivityJournalService(self._snapshots)
        if action == "create":
            journal.record_mission_draft_created(
                correlation_id=acknowledgement.correlation_id,
                draft=draft,
                effect_summary=acknowledgement.effect_summary,
            )
        elif action == "update":
            journal.record_mission_draft_updated(
                correlation_id=acknowledgement.correlation_id,
                draft=draft,
                effect_summary=acknowledgement.effect_summary,
            )
        else:
            journal.record_mission_draft_abandoned(
                correlation_id=acknowledgement.correlation_id,
                draft=draft,
                effect_summary=acknowledgement.effect_summary,
            )

    def _mission_for_draft_action(
        self, snapshot: WorkspaceSnapshot, mission_id: str | None
    ) -> AlbertMission:
        if mission_id is None:
            if snapshot.active_mission is None:
                raise AlbertError("Mission Draft requires an active Mission")
            mission_id = snapshot.active_mission.id
        if mission_id not in self._snapshots._missions:
            raise AlbertError(f"Unknown Mission for Mission Draft: {mission_id}")
        return self._snapshots._missions[mission_id]

    def _ad_hoc_items_by_id(self, mission_id: str) -> dict[str, WorkspaceQueueItem]:
        return {
            item.issue_id: item
            for item in WorkspaceQueueService(self._snapshots).inspect(
                item_type="ad-hoc-delegation",
                mission_id=mission_id,
            ).items
        }

    def _draft_from_inputs(
        self,
        *,
        draft_id: str,
        mission_id: str,
        proposed_goal: str,
        selected_ad_hoc_ids: list[str],
        excluded_ad_hoc_ids: list[str],
        new_work_items: list[str],
        dependencies: list[str],
        unresolved_decisions: list[str],
    ) -> MissionDraft:
        if not proposed_goal.strip():
            raise AlbertError("Mission Draft proposed goal must not be empty")
        if not selected_ad_hoc_ids and not new_work_items:
            raise AlbertError("Mission Draft requires selected Ad Hoc work or new work.")
        self._require_non_empty_items("selected Ad Hoc ids", selected_ad_hoc_ids)
        self._require_non_empty_items("excluded Ad Hoc ids", excluded_ad_hoc_ids)
        self._require_non_empty_items("new work items", new_work_items)
        self._require_non_empty_items("dependencies", dependencies)
        self._require_non_empty_items("unresolved decisions", unresolved_decisions)
        overlap = sorted(set(selected_ad_hoc_ids) & set(excluded_ad_hoc_ids))
        if overlap:
            raise AlbertError(
                f"Mission Draft cannot both include and exclude Ad Hoc work: {', '.join(overlap)}"
            )
        ad_hoc_items = self._ad_hoc_items_by_id(mission_id)
        missing = [
            work_id
            for work_id in [*selected_ad_hoc_ids, *excluded_ad_hoc_ids]
            if work_id not in ad_hoc_items
        ]
        if missing:
            raise AlbertError(f"Unknown Ad Hoc Delegation for Mission Draft: {', '.join(missing)}")
        return MissionDraft(
            draft_id=draft_id,
            mission_id=mission_id,
            status="draft",
            proposed_goal=proposed_goal,
            included_ad_hoc_work=tuple(
                self._included_work(ad_hoc_items[work_id])
                for work_id in selected_ad_hoc_ids
            ),
            excluded_ad_hoc_work_ids=tuple(excluded_ad_hoc_ids),
            new_work_items=tuple(new_work_items),
            dependencies=tuple(dependencies),
            unresolved_decisions=tuple(unresolved_decisions),
        )

    @staticmethod
    def _included_work(item: WorkspaceQueueItem) -> MissionDraftIncludedWork:
        proposed = item.proposed_changes
        return MissionDraftIncludedWork(
            work_id=item.issue_id,
            source=item.source,
            status=item.status,
            acceptance_criteria=tuple(proposed.get("acceptance_criteria", ())),
            allowed_paths=tuple(proposed.get("allowed_paths", ())),
            originating_message_id=str(proposed.get("originating_message_id", "")),
        )

    @staticmethod
    def _require_non_empty_items(label: str, items: list[str]) -> None:
        if any(not item.strip() for item in items):
            raise AlbertError(f"Mission Draft {label} must not contain empty values")

    def _plan_confirmed_issue_slice(
        self, mission: AlbertMission, draft: MissionDraft
    ) -> IssueSlice:
        sequence = self._next_issue_sequence(mission)
        issue_id = f"ISS-{sequence:02d}"
        slug = self._slug(draft.proposed_goal)
        source_path = mission.issues_dir / f"{sequence:02d}-{slug}.md"
        while source_path.exists() or issue_id in mission.issues:
            sequence += 1
            issue_id = f"ISS-{sequence:02d}"
            source_path = mission.issues_dir / f"{sequence:02d}-{slug}.md"
        return self._confirmed_issue_slice(mission, draft, issue_id=issue_id)

    def _confirmed_issue_slice(
        self,
        mission: AlbertMission,
        draft: MissionDraft,
        *,
        issue_id: str,
    ) -> IssueSlice:
        sequence_text = issue_id.removeprefix("ISS-")
        if (
            not issue_id.startswith("ISS-")
            or not sequence_text.isdigit()
            or f"ISS-{int(sequence_text):02d}" != issue_id
        ):
            raise WorkspacePersistenceError(
                f"Mission Draft receipt has an invalid accepted Issue Slice id: {issue_id}"
            )
        sequence = int(sequence_text)
        slug = self._slug(draft.proposed_goal)
        source_path = mission.issues_dir / f"{sequence:02d}-{slug}.md"
        acceptance = self._confirmed_acceptance_criteria(draft)
        return IssueSlice(
            id=issue_id,
            slug=slug,
            title=slug.replace("-", " ").title(),
            status="needs-review",
            tracker_status="ready-for-agent",
            type="AFK",
            risk="Medium",
            suggested_agent="qwen-coder-local-1",
            assigned_agent="qwen-coder-local-1",
            what_to_build=draft.proposed_goal,
            acceptance_criteria=acceptance,
            blocked_by=[],
            source_path=str(source_path),
        )

    def _reconcile_confirmed_issue_slice(
        self,
        mission: AlbertMission,
        draft: MissionDraft,
        *,
        issue_id: str,
        reason: str,
    ) -> IssueSlice:
        with mission._runtime_lock(exclusive=True):
            self._refresh_confirmed_mission_state(mission)
            return self._reconcile_confirmed_issue_slice_locked(
                mission,
                draft,
                issue_id=issue_id,
                reason=reason,
            )

    def _reconcile_confirmed_issue_slice_locked(
        self,
        mission: AlbertMission,
        draft: MissionDraft,
        *,
        issue_id: str,
        reason: str,
    ) -> IssueSlice:
        planned = self._confirmed_issue_slice(
            mission,
            draft,
            issue_id=issue_id,
        )
        timeline_entry = (
            f"{issue_id} created from Mission Draft {draft.draft_id}: {reason.strip()}"
        )
        source_path = Path(planned.source_path)
        if source_path.exists():
            try:
                persisted = mission._parse_issue(source_path)
            except (AlbertError, OSError, UnicodeError) as exc:
                raise WorkspacePersistenceError(
                    f"Confirmed Mission Draft Issue Slice cannot be reconstructed: {exc}"
                ) from exc
            self._validate_confirmed_issue_slice(persisted, planned)
        else:
            if issue_id in mission.issues:
                raise WorkspacePersistenceError(
                    f"Confirmed Mission Draft Issue Slice source is missing: {issue_id}"
                )
            self._write_text_atomically(
                source_path,
                self._confirmed_issue_source(draft),
            )
            persisted = planned

        existing = mission.issues.get(issue_id)
        if existing is not None:
            if timeline_entry in mission.timeline:
                self._validate_confirmed_issue_identity(existing, planned)
            else:
                self._validate_confirmed_issue_slice(existing, planned)
            issue = existing
        else:
            mission.issues[issue_id] = persisted
            issue = persisted
        if timeline_entry not in mission.timeline:
            mission._record(timeline_entry)
        # Persist on every reconciliation. A prior attempt may have updated this
        # in-memory Mission before its runtime write failed, so object equality
        # alone cannot prove that the canonical runtime contains the effect.
        mission._persist(_runtime_lock_held=True)
        return issue

    @staticmethod
    def _refresh_confirmed_mission_state(mission: AlbertMission) -> None:
        mission.issues = mission._load_issues()
        if mission.runtime_path.exists():
            mission._load_runtime()

    @staticmethod
    def _validate_confirmed_issue_identity(
        persisted: IssueSlice,
        planned: IssueSlice,
    ) -> None:
        if (
            persisted.id != planned.id
            or Path(persisted.source_path).resolve()
            != Path(planned.source_path).resolve()
        ):
            raise WorkspacePersistenceError(
                f"Confirmed Mission Draft Issue Slice identity conflicts with {planned.id}."
            )

    @staticmethod
    def _validate_confirmed_issue_slice(
        persisted: IssueSlice,
        planned: IssueSlice,
    ) -> None:
        if (
            persisted.id != planned.id
            or Path(persisted.source_path).resolve() != Path(planned.source_path).resolve()
            or persisted.what_to_build != planned.what_to_build
            or persisted.acceptance_criteria != planned.acceptance_criteria
        ):
            raise WorkspacePersistenceError(
                f"Confirmed Mission Draft Issue Slice boundary conflicts with {planned.id}."
            )

    def _confirmed_issue_source(self, draft: MissionDraft) -> str:
        acceptance = self._confirmed_acceptance_criteria(draft)
        return "\n".join(
            [
                "Status: ready-for-agent",
                "Type: AFK",
                "Risk: Medium",
                "",
                "## Parent",
                "",
                "Mission Draft",
                "",
                "## What to build",
                "",
                draft.proposed_goal,
                "",
                "## Acceptance criteria",
                "",
                *[f"- [ ] {item}" for item in acceptance],
                "",
                "## Blocked by",
                "",
                "None - can start immediately",
                "",
            ]
        )

    @staticmethod
    def _write_text_atomically(path: Path, content: str) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary_path: Path | None = None
        try:
            with tempfile.NamedTemporaryFile(
                mode="w",
                encoding="utf-8",
                dir=path.parent,
                prefix=f".{path.name}.",
                suffix=".tmp",
                delete=False,
            ) as temporary:
                temporary.write(content)
                temporary_path = Path(temporary.name)
            temporary_path.replace(path)
        finally:
            if temporary_path is not None and temporary_path.exists():
                temporary_path.unlink()

    @staticmethod
    def _next_issue_sequence(mission: AlbertMission) -> int:
        numbers: list[int] = []
        for issue_id in mission.issues:
            if issue_id.startswith("ISS-") and issue_id.removeprefix("ISS-").isdigit():
                numbers.append(int(issue_id.removeprefix("ISS-")))
        return max(numbers, default=0) + 1

    @staticmethod
    def _confirmed_acceptance_criteria(draft: MissionDraft) -> list[str]:
        criteria: list[str] = []
        criteria.extend(
            f"Include Ad Hoc Delegation {item.work_id}."
            for item in draft.included_ad_hoc_work
        )
        criteria.extend(
            f"Exclude Ad Hoc Delegation {work_id}."
            for work_id in draft.excluded_ad_hoc_work_ids
        )
        criteria.extend(f"New work: {item}" for item in draft.new_work_items)
        criteria.extend(f"Dependency: {item}" for item in draft.dependencies)
        criteria.extend(f"Resolve decision: {item}" for item in draft.unresolved_decisions)
        return criteria

    @staticmethod
    def _slug(value: str) -> str:
        slug = "".join(character.lower() if character.isalnum() else "-" for character in value)
        slug = "-".join(part for part in slug.split("-") if part)
        return slug or "mission-draft"

    def _load_drafts(self) -> dict[str, Any]:
        if not self._drafts_path.exists():
            return {
                "schema_version": 1,
                "receipt_protocol_version": 2,
                "legacy_receipt_count": 0,
                "legacy_draft_ids": [],
                "revision": 1,
                "drafts": [],
                "receipts": [],
            }
        try:
            data = json.loads(self._drafts_path.read_text(encoding="utf-8"))
            if data["schema_version"] != 1:
                raise ValueError("unsupported Mission Draft schema")
            if not isinstance(data["revision"], int) or data["revision"] < 1:
                raise ValueError("Mission Draft revision must be positive")
            if not isinstance(data["drafts"], list):
                raise ValueError("Mission Draft drafts must be a list")
            receipt_protocol_version = data.get("receipt_protocol_version", 1)
            if receipt_protocol_version not in {1, 2}:
                raise ValueError("unsupported Mission Draft receipt protocol")
            if receipt_protocol_version == 1 and (
                "legacy_receipt_count" in data or "legacy_draft_ids" in data
            ):
                raise ValueError("Mission Draft receipt protocol downgrade is invalid")
            receipts = data.get("receipts", [])
            if not isinstance(receipts, list) or any(
                not isinstance(receipt, dict) for receipt in receipts
            ):
                raise ValueError("Mission Draft receipts must be a list of objects")
            receipt_correlations = [receipt.get("correlation_id") for receipt in receipts]
            if any(
                not isinstance(correlation, str) or not correlation.strip()
                for correlation in receipt_correlations
            ):
                raise ValueError("Mission Draft receipt correlations must be named")
            if len(receipt_correlations) != len(set(receipt_correlations)):
                raise ValueError("Mission Draft receipt correlations must be unique")
            legacy_receipt_count = (
                data.get("legacy_receipt_count", 0)
                if receipt_protocol_version == 2
                else len(receipts)
            )
            if (
                not isinstance(legacy_receipt_count, int)
                or isinstance(legacy_receipt_count, bool)
                or legacy_receipt_count < 0
                or legacy_receipt_count > len(receipts)
            ):
                raise ValueError("Mission Draft legacy receipt count is invalid")
            legacy_draft_ids = (
                data.get("legacy_draft_ids", [])
                if receipt_protocol_version == 2
                else []
            )
            if (
                not isinstance(legacy_draft_ids, list)
                or any(
                    not isinstance(draft_id, str) or not draft_id.strip()
                    for draft_id in legacy_draft_ids
                )
                or len(legacy_draft_ids) != len(set(legacy_draft_ids))
            ):
                raise ValueError("Mission Draft legacy draft ids are invalid")
            if receipt_protocol_version == 1 and any(
                receipt.get("receipt_version") is not None
                or receipt.get("request_fingerprint") is not None
                or "prior_draft" in receipt
                for receipt in receipts
            ):
                raise ValueError("Mission Draft receipt protocol downgrade is invalid")
            if receipt_protocol_version == 2 and any(
                receipt.get("receipt_version") != (1 if index < legacy_receipt_count else 2)
                or "request_fingerprint" in receipt
                for index, receipt in enumerate(receipts)
            ):
                raise ValueError("Mission Draft receipt protocol downgrade is invalid")
            return {
                "schema_version": 1,
                "receipt_protocol_version": receipt_protocol_version,
                "legacy_receipt_count": legacy_receipt_count,
                "legacy_draft_ids": legacy_draft_ids,
                "revision": data["revision"],
                "drafts": [self._parse_draft(item) for item in data["drafts"]],
                "receipts": receipts,
            }
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(
                f"Mission Draft persistence read failed: {exc}"
            ) from exc

    def _parse_draft(self, item: dict[str, Any]) -> MissionDraft:
        status = item["status"]
        if status not in self._statuses:
            raise ValueError(f"unknown Mission Draft status: {status}")
        for field_name in ["draft_id", "mission_id", "proposed_goal"]:
            if not isinstance(item[field_name], str) or not item[field_name].strip():
                raise ValueError(f"Mission Draft {field_name} must not be empty")
        return MissionDraft(
            draft_id=item["draft_id"],
            mission_id=item["mission_id"],
            status=status,
            proposed_goal=item["proposed_goal"],
            included_ad_hoc_work=tuple(
                self._parse_included_work(work)
                for work in item["included_ad_hoc_work"]
            ),
            excluded_ad_hoc_work_ids=tuple(item["excluded_ad_hoc_work_ids"]),
            new_work_items=tuple(item["new_work_items"]),
            dependencies=tuple(item["dependencies"]),
            unresolved_decisions=tuple(item["unresolved_decisions"]),
        )

    def _parse_included_work(self, item: dict[str, Any]) -> MissionDraftIncludedWork:
        status = item["status"]
        if status not in WorkspaceQueueService._statuses:
            raise ValueError(f"unknown Mission Draft included work status: {status}")
        for field_name in ["work_id", "source", "originating_message_id"]:
            if not isinstance(item[field_name], str) or not item[field_name].strip():
                raise ValueError(f"Mission Draft included work {field_name} must not be empty")
        return MissionDraftIncludedWork(
            work_id=item["work_id"],
            source=item["source"],
            status=status,
            acceptance_criteria=tuple(item["acceptance_criteria"]),
            allowed_paths=tuple(item["allowed_paths"]),
            originating_message_id=item["originating_message_id"],
        )


class SessionArtifactService:
    """Reads one explicitly registered, review-safe session artifact as bounded text."""

    _labels = {
        "review_diff": ("Review diff", "text/x-diff"),
        "result": ("Runner result", "application/json"),
        "completion": ("Completion record", "application/json"),
        "fake_log": ("Fake Agent log", "text/plain"),
    }

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots

    def read(
        self,
        *,
        mission_id: str,
        session_id: str,
        artifact_ref: str,
    ) -> SessionArtifactProjection:
        mission = self._snapshots._missions.get(mission_id)
        if mission is None:
            raise SessionArtifactReadError(
                "The requested Mission evidence boundary is unavailable.",
                code="session-artifact-not-found",
            )
        session = mission.sessions.get(session_id)
        if session is None:
            raise SessionArtifactReadError(
                "The requested Local Agent session is unavailable.",
                code="session-artifact-not-found",
            )
        reference = artifact_ref.strip()
        if not reference:
            raise SessionArtifactReadError(
                "The evidence reference must not be empty.",
                code="session-artifact-not-found",
            )

        for artifact_key, registered_path in sorted(session.artifacts.items()):
            if not mission._artifact_is_safe_for_review(artifact_key):
                continue
            opaque_ref = mission.review_artifact_reference(session, artifact_key)
            if reference not in {opaque_ref, registered_path}:
                continue
            return self._read_registered_artifact(
                mission=mission,
                session=session,
                artifact_key=artifact_key,
                registered_path=registered_path,
            )

        if reference in mission.review_artifact_links(session) and (
            reference.startswith("app-local://")
            or reference.startswith("artifact://evidence/")
        ):
            return self._runtime_evidence_projection(mission=mission, session=session)

        raise SessionArtifactReadError(
            "The evidence reference is not registered for this Local Agent session.",
            code="session-artifact-not-found",
        )

    def _read_registered_artifact(
        self,
        *,
        mission: AlbertMission,
        session: LocalAgentSession,
        artifact_key: str,
        registered_path: str,
    ) -> SessionArtifactProjection:
        artifact_root = (mission.runtime_dir / "sessions" / session.session_id).resolve()
        candidate = Path(registered_path)
        try:
            resolved = candidate.resolve(strict=True)
            resolved.relative_to(artifact_root)
        except (OSError, ValueError):
            raise SessionArtifactReadError(
                "The registered evidence artifact is outside its session runtime boundary.",
                code="session-artifact-forbidden",
                recoverable=False,
            ) from None
        if candidate.is_symlink() or not resolved.is_file():
            raise SessionArtifactReadError(
                "The registered evidence artifact is not a regular session file.",
                code="session-artifact-forbidden",
                recoverable=False,
            )
        decoder = codecs.getincrementaldecoder("utf-8")()
        captured = bytearray()
        capture_limit = _SESSION_ARTIFACT_CONTENT_BYTES_LIMIT + 4
        try:
            with resolved.open("rb") as artifact_stream:
                while chunk := artifact_stream.read(64 * 1024):
                    if b"\0" in chunk:
                        raise SessionArtifactReadError(
                            "The registered evidence artifact is not safe text.",
                            code="session-artifact-unsupported",
                            recoverable=False,
                        )
                    decoder.decode(chunk, final=False)
                    remaining = capture_limit - len(captured)
                    if remaining > 0:
                        captured.extend(chunk[:remaining])
                decoder.decode(b"", final=True)
        except OSError:
            raise SessionArtifactReadError(
                "The registered evidence artifact could not be read.",
            ) from None
        except UnicodeDecodeError:
            raise SessionArtifactReadError(
                "The registered evidence artifact is not valid UTF-8 text.",
                code="session-artifact-unsupported",
                recoverable=False,
            ) from None
        payload = bytes(captured)
        content, byte_count, truncated = self._bounded_content(
            mission,
            session,
            payload,
        )
        default_label = artifact_key.replace("_", " ").capitalize()
        label, media_type = self._labels.get(
            artifact_key,
            (
                default_label,
                "application/json" if artifact_key.endswith("_result") else "text/plain",
            ),
        )
        return SessionArtifactProjection(
            schema_version=1,
            mission_id=mission.mission_id,
            session_id=session.session_id,
            artifact_id=artifact_key,
            label=label,
            media_type=media_type,
            content=content,
            byte_count=byte_count,
            content_limit_bytes=_SESSION_ARTIFACT_CONTENT_BYTES_LIMIT,
            truncated=truncated,
        )

    def _runtime_evidence_projection(
        self,
        *,
        mission: AlbertMission,
        session: LocalAgentSession,
    ) -> SessionArtifactProjection:
        if session.evidence is None:
            raise SessionArtifactReadError(
                "The Local Agent session has no runtime Evidence Package.",
                code="session-artifact-not-found",
            )
        evidence = session.evidence
        payload = json.dumps(
            {
                "evidence_valid": session.evidence_valid,
                "changed_files": evidence.changed_files,
                "diff_summary": evidence.diff_summary,
                "commands_run": evidence.commands_run,
                "test_results": evidence.test_results,
                "known_risks": evidence.known_risks,
                "proposed_context_updates": evidence.proposed_context_updates,
            },
            indent=2,
            sort_keys=True,
        ).encode("utf-8")
        content, byte_count, truncated = self._bounded_content(
            mission,
            session,
            payload,
        )
        return SessionArtifactProjection(
            schema_version=1,
            mission_id=mission.mission_id,
            session_id=session.session_id,
            artifact_id="evidence-package",
            label="Evidence Package",
            media_type="application/json",
            content=content,
            byte_count=byte_count,
            content_limit_bytes=_SESSION_ARTIFACT_CONTENT_BYTES_LIMIT,
            truncated=truncated,
        )

    @staticmethod
    def _bounded_content(
        mission: AlbertMission,
        session: LocalAgentSession,
        payload: bytes,
    ) -> tuple[str, int, bool]:
        truncated = len(payload) > _SESSION_ARTIFACT_CONTENT_BYTES_LIMIT
        content = payload[:_SESSION_ARTIFACT_CONTENT_BYTES_LIMIT].decode(
            "utf-8",
            errors="ignore",
        )
        known_paths = {
            str(mission.target_repo): "[workspace]",
            str(mission.runtime_root): "[runtime]",
            str(mission.runtime_dir): "[mission-runtime]",
            str(session.worktree_path): "[worktree]",
            **{
                artifact_path: "[session-artifact]"
                for artifact_path in session.artifacts.values()
            },
        }
        for host_path, replacement in sorted(
            known_paths.items(),
            key=lambda item: len(item[0]),
            reverse=True,
        ):
            if not host_path:
                continue
            content = content.replace(host_path, replacement)
            content = content.replace(host_path.replace("\\", "/"), replacement)
        encoded = content.encode("utf-8")
        if len(encoded) > _SESSION_ARTIFACT_CONTENT_BYTES_LIMIT:
            content = encoded[:_SESSION_ARTIFACT_CONTENT_BYTES_LIMIT].decode(
                "utf-8",
                errors="ignore",
            )
            encoded = content.encode("utf-8")
            truncated = True
        return content, len(encoded), truncated


class SessionOutputService:
    """Reads the bounded, exact-session output journal for inspector polling."""

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots

    def read(
        self,
        *,
        mission_id: str,
        session_id: str,
        after_sequence: int = 0,
    ) -> SessionOutputProjection:
        if after_sequence < 0:
            raise SessionOutputReadError(
                "The output cursor must not be negative.",
                code="session-output-invalid-cursor",
                recoverable=False,
            )
        mission = self._snapshots._missions.get(mission_id)
        if mission is None:
            raise SessionOutputReadError(
                "The requested Mission output boundary is unavailable.",
                code="session-output-not-found",
            )
        session = mission.sessions.get(session_id)
        if session is None:
            raise SessionOutputReadError(
                "The requested Local Agent session is unavailable.",
                code="session-output-not-found",
            )

        journal_root = (mission.runtime_dir / "sessions").resolve()
        journal_path = mission.runtime_dir / "sessions" / session.session_id / "output-events.jsonl"
        try:
            resolved = journal_path.resolve(strict=False)
            resolved.relative_to(journal_root)
        except (OSError, ValueError):
            raise SessionOutputReadError(
                "The session output journal is outside its runtime boundary.",
                code="session-output-forbidden",
                recoverable=False,
            ) from None
        if journal_path.is_symlink():
            raise SessionOutputReadError(
                "The session output journal is not a regular session file.",
                code="session-output-forbidden",
                recoverable=False,
            )

        complete = self._is_complete(session)
        if not journal_path.exists():
            if after_sequence:
                raise SessionOutputReadError(
                    "The output cursor is newer than the retained exact-session journal.",
                    code="session-output-stale-cursor",
                    recoverable=False,
                )
            return SessionOutputProjection(
                schema_version=1,
                mission_id=mission.mission_id,
                session_id=session.session_id,
                events=(),
                complete=complete,
            )
        if not journal_path.is_file():
            raise SessionOutputReadError(
                "The session output journal is not a regular file.",
                code="session-output-forbidden",
                recoverable=False,
            )

        try:
            with journal_path.open("rb") as source:
                payload = source.read(_SESSION_OUTPUT_JOURNAL_BYTES_LIMIT + 1)
        except OSError:
            raise SessionOutputReadError("The session output journal could not be read.") from None
        if len(payload) > _SESSION_OUTPUT_JOURNAL_BYTES_LIMIT:
            raise SessionOutputReadError(
                "The session output journal exceeded its bounded read limit.",
                code="session-output-too-large",
                recoverable=False,
            )

        events: list[SessionOutputEvent] = []
        has_unread_events = False
        expected_sequence = 1
        lines = payload.splitlines(keepends=True)
        for index, line in enumerate(lines):
            if not line.endswith((b"\n", b"\r")) and index == len(lines) - 1:
                # The runner may be appending this final JSONL record now.
                break
            try:
                raw = json.loads(line.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError, TypeError):
                raise SessionOutputReadError(
                    "The session output journal contains malformed JSON.",
                    code="session-output-contract-failure",
                    recoverable=False,
                ) from None
            if not isinstance(raw, dict):
                raise SessionOutputReadError(
                    "The session output journal contains a non-object event.",
                    code="session-output-contract-failure",
                    recoverable=False,
                )
            if (
                raw.get("schema_version") != 1
                or raw.get("mission_id") != mission.mission_id
                or raw.get("session_id") != session.session_id
                or isinstance(raw.get("sequence"), bool)
                or not isinstance(raw.get("sequence"), int)
                or raw["sequence"] != expected_sequence
                or not isinstance(raw.get("content"), str)
                or len(raw["content"].encode("utf-8")) > _SESSION_OUTPUT_EVENT_CONTENT_BYTES_LIMIT
            ):
                raise SessionOutputReadError(
                    "The session output journal crossed its exact-session contract.",
                    code="session-output-contract-failure",
                    recoverable=False,
                )
            expected_sequence += 1
            if raw["sequence"] > after_sequence and len(events) < _SESSION_OUTPUT_EVENT_COUNT_LIMIT:
                phase = self._phase(session, complete)
                events.append(
                    SessionOutputEvent(
                        schema_version=1,
                        mission_id=mission.mission_id,
                        session_id=session.session_id,
                        sequence=raw["sequence"],
                        content=raw["content"],
                        phase=phase,
                    )
                )
            elif raw["sequence"] > after_sequence:
                has_unread_events = True
        if after_sequence > expected_sequence - 1:
            raise SessionOutputReadError(
                "The output cursor is newer than the retained exact-session journal.",
                code="session-output-stale-cursor",
                recoverable=False,
            )
        return SessionOutputProjection(
            schema_version=1,
            mission_id=mission.mission_id,
            session_id=session.session_id,
            events=tuple(events),
            # A terminal runner can still have a next bounded page. Completion
            # means the exact cursor has drained the retained journal, never
            # merely that the runner has ended.
            complete=complete and not has_unread_events,
        )

    @staticmethod
    def _is_complete(session: LocalAgentSession) -> bool:
        status = session.status.casefold()
        return bool(session.runner_ended_at) or status in {
            "completed",
            "complete",
            "done",
            "evidence-ready",
            "failed",
            "cancelled",
            "review-ready",
            "reviewed",
        }

    @classmethod
    def _phase(
        cls,
        session: LocalAgentSession,
        complete: bool,
    ) -> Literal["streaming", "complete", "failed"]:
        if "fail" in session.status.casefold() or session.status.casefold() == "cancelled":
            return "failed"
        return "complete" if complete else "streaming"


class ReviewWorkspaceService:
    """Builds the exclusive evidence-decision projection for the active Mission."""

    _required_evidence_fields = [
        "changed_files",
        "diff_summary",
        "commands_run",
        "test_results",
        "known_risks",
        "proposed_context_updates",
    ]
    _reviewable_statuses = {"evidence-ready", "failed", "cancelled", "completed"}

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots

    def inspect(self) -> ReviewWorkspaceProjection:
        snapshot = self._snapshots.snapshot()
        if snapshot.active_mission is None:
            return ReviewWorkspaceProjection(
                schema_version=1,
                revision=snapshot.revision,
                mission_id="",
                items=(),
            )
        mission = self._snapshots._missions[snapshot.active_mission.id]
        items = tuple(
            self._item(mission, session)
            for session in sorted(mission.sessions.values(), key=lambda item: item.session_id)
            if session.status in self._reviewable_statuses
        )
        return ReviewWorkspaceProjection(
            schema_version=1,
            revision=snapshot.revision,
            mission_id=mission.mission_id,
            items=items,
        )

    @_atomic_workspace_action()
    def decide(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        session_id: str,
        mission_id: str | None = None,
        decision: Literal["accept", "repair", "escalate-human"],
        reason: str = "",
        failure_type: str = "",
    ) -> ReviewWorkspaceDecisionAcknowledgement:
        snapshot = self._snapshots.snapshot()
        if not correlation_id.strip():
            raise AlbertError("Review decision correlation id must not be empty")
        persisted = self._review_for_correlation(correlation_id)
        if persisted is not None:
            persisted_mission, persisted_review = persisted
            request_payload = self._request_payload(
                mission_id=(
                    mission_id
                    or str(
                        persisted_review.workspace_action.get("request", {}).get(
                            "mission_id",
                            persisted_mission.mission_id,
                        )
                    )
                ),
                session_id=session_id,
                decision=decision,
                reason=reason,
                failure_type=failure_type,
            )
            metadata = persisted_review.workspace_action
            if metadata.get("request") != request_payload:
                raise AlbertError(
                    "Review decision correlation id was already used for a different request."
                )
            return self._acknowledge_review(
                correlation_id=correlation_id,
                mission=persisted_mission,
                review=persisted_review,
            )
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        target_mission_id = mission_id or (
            snapshot.active_mission.id if snapshot.active_mission is not None else ""
        )
        if not target_mission_id:
            raise AlbertError("Review Workspace requires an active or explicit Mission")
        mission = self._snapshots._missions.get(target_mission_id)
        if mission is None:
            raise AlbertError(f"Unknown Review Workspace Mission: {target_mission_id}")
        request_payload = self._request_payload(
            mission_id=target_mission_id,
            session_id=session_id,
            decision=decision,
            reason=reason,
            failure_type=failure_type,
        )
        if session_id not in mission.sessions:
            raise AlbertError(f"Unknown Review Workspace session: {session_id}")
        session = mission.sessions[session_id]
        if session.status not in self._reviewable_statuses:
            raise AlbertError(
                f"{session_id} cannot be reviewed from {session.status}; "
                "evidence-ready or terminal state is required."
            )
        if decision == "accept":
            missing = (
                session.evidence.missing_fields()
                if session.evidence
                else list(self._required_evidence_fields)
            )
            if not session.evidence_valid or missing:
                raise EvidenceValidationError(
                    f"Evidence Package is missing: {', '.join(missing)}"
                )
            review_outcome = "Approved"
            review_reason = reason.strip() or "Accepted through Review Workspace."
        elif decision == "repair":
            if not reason.strip():
                raise AlbertError("Repair review decisions require a reason.")
            review_outcome = "Needs repair"
            review_reason = reason.strip()
        elif decision == "escalate-human":
            review_outcome = "Needs human review"
            review_reason = reason.strip() or "Escalated for human review."
        else:
            raise AlbertError(f"Unknown Review Workspace decision: {decision}")

        review = mission.record_frontier_review(
            session_id,
            review_outcome,
            reason=review_reason,
            failure_type=failure_type.strip(),
            allowed_session_statuses=self._reviewable_statuses,
            workspace_action={
                "correlation_id": correlation_id,
                "request": request_payload,
            },
            expected_revision=session.revision,
        )
        return self._acknowledge_review(
            correlation_id=correlation_id,
            mission=mission,
            review=review,
        )

    @staticmethod
    def _request_payload(
        *,
        mission_id: str,
        session_id: str,
        decision: str,
        reason: str,
        failure_type: str,
    ) -> dict[str, str]:
        return {
            "mission_id": mission_id,
            "session_id": session_id,
            "decision": decision,
            "reason": reason.strip(),
            "failure_type": failure_type.strip(),
        }

    def _review_for_correlation(
        self,
        correlation_id: str,
    ) -> tuple[AlbertMission, ReviewDecision] | None:
        matches: list[tuple[AlbertMission, ReviewDecision]] = []
        for mission in self._snapshots._missions.values():
            if mission.runtime_path.exists():
                with mission._runtime_lock(exclusive=False):
                    mission._load_runtime()
            for review in mission.reviews:
                metadata = review.workspace_action
                if metadata.get("correlation_id") == correlation_id:
                    matches.append((mission, review))
        if len(matches) > 1:
            raise WorkspacePersistenceError(
                f"Review decision correlation id is not unique: {correlation_id}"
            )
        return matches[0] if matches else None

    def _acknowledge_review(
        self,
        *,
        correlation_id: str,
        mission: AlbertMission,
        review: ReviewDecision,
    ) -> ReviewWorkspaceDecisionAcknowledgement:
        snapshot = self._snapshots.snapshot()
        existing_event = next(
            (
                event
                for event in self._snapshots.events()
                if event.correlation_id == correlation_id
            ),
            None,
        )
        if existing_event is None:
            updated = self._snapshots._update_preferences_locked(
                active_mission_id=(
                    snapshot.active_mission.id
                    if snapshot.active_mission is not None
                    else mission.mission_id
                ),
                conversation_scope=snapshot.conversation_scope,
                operations_view=snapshot.operations_view,
                event_metadata={"correlation_id": correlation_id},
            )
            revision = updated.revision
        else:
            revision = existing_event.revision
        session = mission.sessions[review.session_id]
        issue = mission.issues.get(review.issue_id)
        lifecycle = (
            mission._issue_lifecycle(issue)
            if issue
            else self._ad_hoc_lifecycle(review.outcome, session.status)
        )
        ActivityJournalService(self._snapshots).record_review_decision(
            correlation_id=correlation_id,
            mission=mission,
            issue_id=review.issue_id,
            issue_title=issue.title if issue else str(session.task_packet.get("goal", review.issue_id)),
            session_id=review.session_id,
            review_outcome=review.outcome,
            next_action=review.next_action,
            evidence_links=tuple(mission.review_artifact_links(session)),
        )
        return ReviewWorkspaceDecisionAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            issue_id=review.issue_id,
            session_id=review.session_id,
            review_outcome=review.outcome,
            next_action=review.next_action,
            issue_lifecycle=lifecycle,
            effect_summary=self._effect_summary(
                review.outcome,
                review.next_action,
                lifecycle,
                issue_backed=issue is not None,
            ),
        )

    def _item(self, mission: AlbertMission, session: Any) -> ReviewWorkspaceItem:
        issue = mission.issues.get(session.issue_id)
        evidence = session.evidence
        missing = evidence.missing_fields() if evidence else list(self._required_evidence_fields)
        evidence_complete = bool(session.evidence_valid and evidence and not missing)
        changed_files = list(evidence.changed_files) if evidence else []
        issue_title = (
            issue.title
            if issue
            else str(session.task_packet.get("goal", f"Ad Hoc Delegation {session.issue_id}"))
        )
        lifecycle = (
            mission._issue_lifecycle(issue)
            if issue
            else self._ad_hoc_lifecycle(
                mission._latest_review_for_session(session.session_id).outcome
                if mission._latest_review_for_session(session.session_id)
                else "",
                session.status,
            )
        )
        return ReviewWorkspaceItem(
            mission_id=mission.mission_id,
            issue_id=session.issue_id,
            issue_title=issue_title,
            session_id=session.session_id,
            assigned_agent=session.assigned_agent,
            status=session.status,
            lifecycle=lifecycle,
            evidence_complete=evidence_complete,
            missing_evidence=missing,
            can_accept=evidence_complete,
            evidence=ReviewWorkspaceEvidence(
                changed_files=changed_files,
                diff_summary=evidence.diff_summary if evidence else "",
                commands_run=list(evidence.commands_run) if evidence else [],
                test_results=evidence.test_results if evidence else "",
                risks=evidence.known_risks if evidence else "",
                proposed_context_updates=evidence.proposed_context_updates if evidence else "",
                artifact_links=mission.review_artifact_links(session),
            ),
            visibility_limitations=[
                ReviewWorkspaceVisibilityLimitation(
                    path=path,
                    classification=classification,
                    consequence=self._visibility_consequence(classification),
                )
                for path in changed_files
                for classification in [mission.classify_file_for_frontier(path)]
                if classification != "Normal"
            ],
        )

    @staticmethod
    def _visibility_consequence(classification: str) -> str:
        if classification == "Blocked":
            return "Frontier Reviewer cannot inspect this path; human review may be required."
        if classification == "Local-only":
            return "Visible only on this workstation; include summary evidence for Frontier review."
        return "No limitation."

    @staticmethod
    def _ad_hoc_lifecycle(review_outcome: str, status: str) -> str:
        if review_outcome in {"Approved", "Approved with limitations"}:
            return "Complete"
        if review_outcome == "Needs repair":
            return "Needs repair"
        if review_outcome == "Needs human review":
            return "Needs human review"
        if review_outcome == "Rejected":
            return "Rejected"
        if status == "evidence-ready":
            return "Evidence ready"
        return status.replace("-", " ").title()

    @staticmethod
    def _effect_summary(
        review_outcome: str,
        next_action: str,
        lifecycle: str,
        *,
        issue_backed: bool = True,
    ) -> str:
        if not issue_backed:
            if review_outcome in {"Approved", "Approved with limitations"}:
                return f"Ad Hoc Delegation becomes {lifecycle}; next action is {next_action}."
            if review_outcome == "Needs repair":
                return f"Ad Hoc Delegation needs repair; next action is {next_action}."
            if review_outcome == "Needs human review":
                return "Ad Hoc Delegation records needs-human-review and waits for human review."
            return f"Ad Hoc Delegation review outcome is {review_outcome}; next action is {next_action}."
        if review_outcome in {"Approved", "Approved with limitations"}:
            return f"Issue Slice becomes {lifecycle} and PR-ready; it is not marked merged."
        if review_outcome == "Needs repair":
            return f"Issue Slice needs repair; next action is {next_action}."
        if review_outcome == "Needs human review":
            return "Issue Slice records needs-human-review and waits for human review."
        return f"Review recorded; next action is {next_action}."


class WorkstationActionService:
    """Applies typed Agent Workstation actions against acknowledged Orchestrator state."""

    _actions = {
        "issue-approve",
        "issue-launch",
        "issue-retry",
        "session-cancel",
        "model-assignment-change",
        "issue-archive",
        "issue-restore",
        "retirement-pin",
        "retirement-retry",
        "retirement-export",
        "retirement-discard",
    }
    _actors = {"mission-commander"}

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots

    @_audit_rejected_workstation_action
    @_atomic_workspace_action()
    def submit(
        self,
        *,
        correlation_id: str,
        action_type: WorkstationActionType | str,
        actor: str,
        expected_revision: int,
        target_kind: str,
        target_id: str,
        mission_id: str | None = None,
        issue_id: str = "",
        session_id: str = "",
        agent_id: str = "",
        reason: str = "",
        allowed_paths: list[str] | None = None,
        command_policy: dict[str, str] | None = None,
        pin_state: bool | None = None,
        destination: str = "",
        confirmation: str = "",
    ) -> WorkstationActionAcknowledgement:
        snapshot = self._snapshots.snapshot()
        if not correlation_id.strip():
            raise AlbertError("Workstation action correlation id must not be empty")
        if actor not in self._actors:
            raise AlbertError(f"Unknown Workstation action actor: {actor}")
        if action_type not in self._actions:
            raise AlbertError(f"Unknown Workstation action type: {action_type}")
        target_mission_id = mission_id or (
            snapshot.active_mission.id if snapshot.active_mission is not None else ""
        )
        if not target_mission_id:
            raise AlbertError("Workstation actions require an active or explicit Mission")
        mission = self._snapshots._missions.get(target_mission_id)
        if mission is None:
            raise AlbertError(f"Unknown Workstation Mission: {target_mission_id}")
        if mission.runtime_path.exists():
            with mission._runtime_lock(exclusive=False):
                mission._load_runtime()
        request_payload = {
            "issue_id": issue_id,
            "session_id": session_id,
            "agent_id": agent_id,
            "reason": reason.strip(),
            "allowed_paths": list(allowed_paths or []),
            "command_policy": dict(command_policy or {}),
        }
        retirement_action = action_type in {
            "retirement-pin",
            "retirement-retry",
            "retirement-export",
            "retirement-discard",
        }
        if retirement_action:
            request_payload.update(
                {
                    "expected_revision": expected_revision,
                    "pin_state": pin_state,
                    "destination": (
                        str(Path(destination).resolve(strict=False))
                        if destination
                        else ""
                    ),
                    "confirmation": confirmation,
                }
            )
        request_boundary = {
            "action_type": action_type,
            "actor": actor,
            "mission_id": target_mission_id,
            "target_kind": target_kind,
            "target_id": target_id,
            **request_payload,
        }
        replay = self._replay_receipt(
            correlation_id=correlation_id,
            request_boundary=request_boundary,
        )
        if replay is not None:
            return replay
        canonical_action = (
            self._canonical_retirement_action_for_correlation(
                correlation_id=correlation_id,
                request_boundary=request_boundary,
            )
            if retirement_action
            else self._canonical_action_for_correlation(
                correlation_id=correlation_id,
                request_boundary=request_boundary,
            )
        )
        recovering_action = canonical_action is not None
        if not recovering_action and action_type in {"issue-launch", "issue-retry"}:
            WayfinderService(self._snapshots).ensure_gate_open()
        recovered_session = canonical_action[1] if canonical_action is not None else None
        if (
            not recovering_action
            and not retirement_action
            and expected_revision != snapshot.revision
        ):
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )

        workstation_action = {
            "correlation_id": correlation_id,
            "action_type": action_type,
            "target_kind": target_kind,
            "target_id": target_id,
            "request": request_payload,
        }
        if action_type in {"issue-archive", "issue-restore"}:
            workstation_action.update(
                {
                    "actor": actor,
                    "mission_id": target_mission_id,
                    "expected_revision": expected_revision,
                }
            )

        acknowledged_issue_id = issue_id
        acknowledged_session_id = session_id
        journal_actor: ActivityActor = "mission-commander"
        if retirement_action:
            self._validate_session_target(
                target_kind=target_kind,
                target_id=target_id,
                session_id=session_id,
            )
            prior_session = mission.sessions.get(session_id)
            if prior_session is None:
                raise AlbertError(f"Unknown Workstation session: {session_id}")
            acknowledged_issue_id = issue_id or prior_session.issue_id
            if acknowledged_issue_id != prior_session.issue_id:
                raise AlbertError("issue id must match session issue id")
            if recovering_action:
                result_session = recovered_session or prior_session
            elif action_type == "retirement-pin":
                if not isinstance(pin_state, bool):
                    raise AlbertError("Retirement pin requires a boolean pin state.")
                result_session = mission.set_retirement_snapshot_pin(
                    session_id,
                    pinned=pin_state,
                    expected_revision=expected_revision,
                    correlation_id=correlation_id,
                )
            elif action_type == "retirement-retry":
                result_session = mission.retry_retirement_unit(
                    session_id,
                    expected_revision=expected_revision,
                    correlation_id=correlation_id,
                )
            elif action_type == "retirement-export":
                if not destination:
                    raise AlbertError("Retirement export requires a destination.")
                mission.export_retirement_unit(
                    session_id,
                    destination=Path(destination),
                    expected_revision=expected_revision,
                    correlation_id=correlation_id,
                )
                result_session = mission._refresh_persisted_session(session_id)
            else:
                result_session = mission.discard_retained_worktree(
                    session_id,
                    expected_revision=expected_revision,
                    correlation_id=correlation_id,
                    confirmation=confirmation,
                    reason=reason,
                )
            acknowledged_session_id = result_session.session_id
            effect_summary = {
                "retirement-pin": (
                    f"Mission Commander {'pinned' if pin_state else 'unpinned'} "
                    f"the retained Snapshot Payload for {session_id}."
                ),
                "retirement-retry": (
                    f"Mission Commander retried Retirement Unit {session_id}; "
                    f"its canonical phase is {result_session.retirement.get('phase')}."
                ),
                "retirement-export": (
                    f"Mission Commander exported verified Retirement Unit {session_id} "
                    f"to {Path(destination).resolve(strict=False) / 'repository'}."
                ),
                "retirement-discard": (
                    f"Mission Commander irreversibly discarded Retained Worktree "
                    f"{session_id} after exact safety proof."
                ),
            }[action_type]
        elif action_type == "issue-approve":
            self._validate_issue_target(
                target_kind=target_kind,
                target_id=target_id,
                issue_id=issue_id,
            )
            issue = mission.issues.get(issue_id)
            if issue is None:
                raise AlbertError(f"Unknown Issue Slice: {issue_id}")
            if not recovering_action:
                if issue.tracker_status.casefold() not in {
                    "approved",
                    "ready",
                    "ready-for-agent",
                }:
                    raise AlbertError(
                        f"{issue_id} tracker status {issue.tracker_status!r} is not ready for agent approval"
                    )
                mission.approve_issue(
                    issue_id,
                    workstation_action=workstation_action,
                )
            acknowledged_session_id = ""
            effect_summary = f"Mission Commander approved {issue_id} for governed Local Agent launch."
        elif action_type == "issue-launch":
            self._validate_issue_target(
                target_kind=target_kind,
                target_id=target_id,
                issue_id=issue_id,
            )
            session = recovered_session
            if recovering_action and session is None:
                raise WorkspacePersistenceError(
                    f"Recovered Workstation launch has no session: {correlation_id}"
                )
            if session is None:
                session = mission.launch_issue(
                    issue_id,
                    allowed_paths=allowed_paths or [],
                    command_policy=command_policy or {},
                    workstation_action=workstation_action,
                )
            acknowledged_session_id = session.session_id
            effect_summary = (
                f"Orchestrator queued {issue_id} as {session.session_id}."
            )
            journal_actor = "orchestrator"
        elif action_type == "issue-retry":
            self._validate_session_target(
                target_kind=target_kind,
                target_id=target_id,
                session_id=session_id,
            )
            review_workspace_repair = self._review_workspace_repair(
                mission,
                session_id,
            )
            if (
                not recovering_action
                and not reason.strip()
                and review_workspace_repair is None
            ):
                raise AlbertError("Retry requires a reason.")
            prior_session = mission.sessions.get(session_id)
            if prior_session is None:
                raise AlbertError(f"Unknown Workstation session: {session_id}")
            acknowledged_issue_id = issue_id or prior_session.issue_id
            if acknowledged_issue_id != prior_session.issue_id:
                raise AlbertError("issue id must match session issue id")
            session = recovered_session
            if recovering_action and session is None:
                raise WorkspacePersistenceError(
                    f"Recovered Workstation retry has no session: {correlation_id}"
                )
            if session is None:
                if (
                    review_workspace_repair is not None
                    and self._repair_child_for_session(mission, session_id) is not None
                ):
                    raise AlbertError(
                        f"Review Workspace repair was already launched for {session_id}."
                    )
                session = mission.launch_repair(
                    session_id,
                    agent_id=agent_id,
                    allowed_paths=allowed_paths if allowed_paths else None,
                    command_policy=command_policy if command_policy else None,
                    workstation_action=workstation_action,
                    manual_retry_reason=(
                        reason if review_workspace_repair is None else ""
                    ),
                    expected_revision=prior_session.revision,
                )
            acknowledged_session_id = session.session_id
            effect_summary = (
                f"Orchestrator queued repair for {acknowledged_issue_id} as "
                f"{session.session_id} from {prior_session.session_id}."
            )
            journal_actor = "orchestrator"
        elif action_type == "session-cancel":
            self._validate_session_target(
                target_kind=target_kind,
                target_id=target_id,
                session_id=session_id,
            )
            if not reason.strip():
                raise AlbertError("Session cancellation requires a reason.")
            session = mission.sessions.get(session_id)
            if session is None:
                raise AlbertError(f"Unknown Workstation session: {session_id}")
            acknowledged_issue_id = issue_id or session.issue_id
            if acknowledged_issue_id != session.issue_id:
                raise AlbertError("issue id must match session issue id")
            cancelled = (
                session
                if recovering_action
                else mission.cancel_session(
                    session_id,
                    reason=reason,
                    workstation_action=workstation_action,
                    expected_revision=session.revision,
                )
            )
            acknowledged_session_id = cancelled.session_id
            effect_summary = (
                f"Orchestrator cancelled {cancelled.session_id} for "
                f"{acknowledged_issue_id}. Runner termination was requested and "
                "the terminal cancelled state will be preserved."
            )
            journal_actor = "orchestrator"
        elif action_type in {"issue-archive", "issue-restore"}:
            self._validate_issue_target(
                target_kind=target_kind,
                target_id=target_id,
                issue_id=issue_id,
            )
            if issue_id not in mission.issues:
                raise AlbertError(f"Unknown Issue Slice: {issue_id}")
            if not recovering_action:
                if action_type == "issue-archive":
                    mission._archive_issue_from_workstation(
                        issue_id,
                        workstation_action=workstation_action,
                        expected_revision=expected_revision,
                    )
                else:
                    mission._restore_archived_issue_from_workstation(
                        issue_id,
                        workstation_action=workstation_action,
                        expected_revision=expected_revision,
                    )
            acknowledged_session_id = ""
            effect_summary = (
                f"Mission Commander archived {issue_id}; its sessions, evidence, and Activity Journal history remain inspectable."
                if action_type == "issue-archive"
                else f"Mission Commander restored {issue_id} to active Mission Work with its sessions, evidence, and Activity Journal history intact."
            )
        else:
            self._validate_issue_target(
                target_kind=target_kind,
                target_id=target_id,
                issue_id=issue_id,
            )
            if not agent_id.strip():
                raise AlbertError("Model assignment changes require an agent id.")
            if not reason.strip():
                raise AlbertError("Model assignment changes require a reason.")
            if not recovering_action:
                mission.assign_issue(
                    issue_id,
                    agent_id,
                    notes=reason,
                    workstation_action=workstation_action,
                )
            effect_summary = (
                f"Mission Commander assigned {issue_id} to {agent_id}: {reason}"
            )

        acknowledgement = WorkstationActionAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=snapshot.revision + 1,
            action_type=action_type,  # type: ignore[arg-type]
            issue_id=acknowledged_issue_id,
            session_id=acknowledged_session_id,
            effect_summary=effect_summary,
        )
        updated = self._snapshots._update_preferences_locked(
            active_mission_id=(
                snapshot.active_mission.id
                if snapshot.active_mission is not None
                else mission.mission_id
            ),
            conversation_scope=snapshot.conversation_scope,
            operations_view=snapshot.operations_view,
            event_metadata={"correlation_id": correlation_id},
            workstation_receipt={
                "correlation_id": correlation_id,
                "request": request_boundary,
                "acknowledgement": asdict(acknowledgement),
            },
        )
        if updated.revision != acknowledgement.revision:
            raise WorkspacePersistenceError(
                "Workstation action acknowledgement revision did not match the "
                "serialized action transaction."
            )
        self._reconcile_audit_side_effects(
            request_boundary=request_boundary,
            acknowledgement=acknowledgement,
            journal_actor=journal_actor,
        )
        return acknowledgement

    def _record_rejected_attempt(
        self,
        *,
        error: AlbertError,
        correlation_id: str,
        action_type: WorkstationActionType | str,
        actor: str,
        target_kind: str,
        target_id: str,
        mission_id: str | None = None,
        **_request: Any,
    ) -> None:
        if (
            not correlation_id.strip()
            or actor != "mission-commander"
            or action_type not in self._actions
            or not target_kind.strip()
            or not target_id.strip()
        ):
            return
        snapshot = self._snapshots.snapshot()
        target_mission_id = mission_id or (
            snapshot.active_mission.id if snapshot.active_mission is not None else ""
        )
        if target_mission_id not in self._snapshots._missions:
            return
        AgentConsoleHistoryService(
            self._snapshots
        ).record_workstation_action_rejected(
            correlation_id=correlation_id,
            action_type=str(action_type),
            target_id=target_id,
            reason=str(error),
            mission_id=target_mission_id,
        )

    def _reconcile_audit_side_effects(
        self,
        *,
        request_boundary: dict[str, Any],
        acknowledgement: WorkstationActionAcknowledgement,
        journal_actor: ActivityActor | None = None,
    ) -> None:
        mission_id = request_boundary.get("mission_id")
        target_id = request_boundary.get("target_id")
        action_type = request_boundary.get("action_type")
        if (
            not isinstance(mission_id, str)
            or mission_id not in self._snapshots._missions
            or not isinstance(target_id, str)
            or not isinstance(action_type, str)
        ):
            raise WorkspacePersistenceError(
                "Workstation action receipt has an invalid audit boundary."
            )
        mission = self._snapshots._missions[mission_id]
        actor = journal_actor or (
            "orchestrator"
            if action_type in {"issue-launch", "issue-retry", "session-cancel"}
            else "mission-commander"
        )
        ActivityJournalService(self._snapshots).record_workstation_action(
            correlation_id=acknowledgement.correlation_id,
            actor=actor,
            action_type=action_type,
            mission=mission,
            issue_id=acknowledgement.issue_id,
            session_id=acknowledgement.session_id,
            effect_summary=acknowledgement.effect_summary,
        )
        AgentConsoleHistoryService(self._snapshots).record_workstation_action(
            correlation_id=acknowledgement.correlation_id,
            action_type=action_type,
            target_id=target_id,
            effect_summary=acknowledgement.effect_summary,
            mission_id=mission_id,
        )

    @staticmethod
    def _review_workspace_repair(
        mission: AlbertMission,
        session_id: str,
    ) -> ReviewDecision | None:
        review = mission._latest_review_for_session(session_id)
        if (
            review is None
            or review.outcome != "Needs repair"
            or review.next_action not in {
                "same-local-agent-repair",
                "fresh-local-agent-repair",
            }
        ):
            return None
        return review

    @staticmethod
    def _repair_child_for_session(
        mission: AlbertMission,
        session_id: str,
    ) -> LocalAgentSession | None:
        return next(
            (
                candidate
                for candidate in mission.sessions.values()
                if isinstance(candidate.task_packet.get("repair_context"), dict)
                and candidate.task_packet["repair_context"].get("prior_session_id")
                == session_id
            ),
            None,
        )

    def _replay_receipt(
        self,
        *,
        correlation_id: str,
        request_boundary: dict[str, Any],
    ) -> WorkstationActionAcknowledgement | None:
        preferences = self._snapshots._load_preferences()
        receipt = next(
            (
                item
                for item in preferences["workstation_receipts"]
                if item.get("correlation_id") == correlation_id
            ),
            None,
        )
        if receipt is None:
            return None
        if receipt.get("request") != request_boundary:
            raise AlbertError(
                "Workstation action correlation id was already used for a different "
                "request boundary."
            )
        try:
            acknowledgement = WorkstationActionAcknowledgement(
                **receipt["acknowledgement"]
            )
        except (KeyError, TypeError) as exc:
            raise WorkspacePersistenceError(
                f"Workstation action receipt is invalid: {correlation_id}"
            ) from exc
        if (
            acknowledgement.correlation_id != correlation_id
            or acknowledgement.action_type != request_boundary["action_type"]
        ):
            raise WorkspacePersistenceError(
                f"Workstation action receipt boundary is invalid: {correlation_id}"
            )
        self._reconcile_audit_side_effects(
            request_boundary=request_boundary,
            acknowledgement=acknowledgement,
        )
        return acknowledgement

    def _canonical_action_for_correlation(
        self,
        *,
        correlation_id: str,
        request_boundary: dict[str, Any],
    ) -> tuple[AlbertMission, LocalAgentSession | None] | None:
        matches: list[
            tuple[AlbertMission, LocalAgentSession | None, dict[str, Any]]
        ] = []
        for mission in self._snapshots._missions.values():
            if mission.runtime_path.exists():
                with mission._runtime_lock(exclusive=False):
                    mission._load_runtime()
            durable_marker = mission.workstation_actions.get(correlation_id)
            if durable_marker is not None:
                matches.append(
                    (
                        mission,
                        None,
                        self._normalized_canonical_action_boundary(
                            mission=mission,
                            correlation_id=correlation_id,
                            marker=durable_marker,
                        ),
                    )
                )
            for session in mission.sessions.values():
                marker = session.task_packet.get("workstation_action")
                if not isinstance(marker, dict):
                    continue
                if marker.get("correlation_id") != correlation_id:
                    continue
                matches.append(
                    (
                        mission,
                        session,
                        self._normalized_canonical_action_boundary(
                            mission=mission,
                            correlation_id=correlation_id,
                            marker=marker,
                        ),
                    )
                )
        if not matches:
            return None
        if len(matches) != 1:
            raise WorkspacePersistenceError(
                f"Workstation action correlation id is not unique: {correlation_id}"
            )
        mission, session, persisted_boundary = matches[0]
        if persisted_boundary != request_boundary:
            raise AlbertError(
                "Workstation action correlation id was already used for a different "
                "request boundary."
            )
        return mission, session

    def _canonical_retirement_action_for_correlation(
        self,
        *,
        correlation_id: str,
        request_boundary: dict[str, Any],
    ) -> tuple[AlbertMission, LocalAgentSession | None] | None:
        matches: list[tuple[AlbertMission, LocalAgentSession]] = []
        for mission in self._snapshots._missions.values():
            if mission.runtime_path.exists():
                with mission._runtime_lock(exclusive=False):
                    mission._load_runtime()
            for session in mission.sessions.values():
                receipt = session.retirement.get("action_receipts", {}).get(
                    correlation_id
                )
                if receipt is not None:
                    matches.append((mission, session))
        if not matches:
            return None
        if len(matches) != 1:
            raise WorkspacePersistenceError(
                f"Retirement action correlation id is not unique: {correlation_id}"
            )
        mission, session = matches[0]
        receipt = session.retirement["action_receipts"][correlation_id]
        action_type = str(request_boundary.get("action_type", ""))
        expected_action = {
            "retirement-pin": "snapshot-pin",
            "retirement-retry": "retry",
            "retirement-export": "export",
            "retirement-discard": "discard",
        }.get(action_type)
        boundary_matches = bool(
            expected_action
            and receipt.get("action") == expected_action
            and request_boundary.get("actor") == "mission-commander"
            and request_boundary.get("mission_id") == mission.mission_id
            and request_boundary.get("target_kind") == "agent-session"
            and request_boundary.get("target_id") == session.session_id
            and request_boundary.get("session_id") == session.session_id
            and request_boundary.get("issue_id") in {"", session.issue_id}
            and request_boundary.get("expected_revision")
            == receipt.get("expected_revision")
        )
        if action_type == "retirement-pin":
            boundary_matches = boundary_matches and (
                request_boundary.get("pin_state") is receipt.get("pinned")
            )
        elif action_type == "retirement-export":
            destination = request_boundary.get("destination")
            boundary_matches = boundary_matches and bool(
                isinstance(destination, str)
                and destination
                and receipt.get("destination")
                == str(Path(destination) / "repository")
            )
        elif action_type == "retirement-discard":
            boundary_matches = boundary_matches and (
                request_boundary.get("confirmation") == receipt.get("confirmation")
                and request_boundary.get("reason") == receipt.get("reason")
            )
        if not boundary_matches:
            raise AlbertError(
                "Retirement action correlation id was already used for a different "
                "request boundary."
            )
        return mission, session

    @staticmethod
    def _normalized_canonical_action_boundary(
        *,
        mission: AlbertMission,
        correlation_id: str,
        marker: dict[str, Any],
    ) -> dict[str, Any]:
        expected_request_fields = {
            "issue_id",
            "session_id",
            "agent_id",
            "reason",
            "allowed_paths",
            "command_policy",
        }
        request = marker.get("request")
        if (
            marker.get("correlation_id") != correlation_id
            or not isinstance(marker.get("action_type"), str)
            or not isinstance(marker.get("target_kind"), str)
            or not isinstance(marker.get("target_id"), str)
            or not isinstance(request, dict)
            or set(request) != expected_request_fields
        ):
            raise WorkspacePersistenceError(
                f"Workstation action recovery marker is invalid: {correlation_id}"
            )
        if not isinstance(request.get("allowed_paths"), list) or not isinstance(
            request.get("command_policy"),
            dict,
        ):
            raise WorkspacePersistenceError(
                f"Workstation action recovery boundary is invalid: {correlation_id}"
            )
        return {
            "action_type": marker["action_type"],
            "actor": "mission-commander",
            "mission_id": mission.mission_id,
            "target_kind": marker["target_kind"],
            "target_id": marker["target_id"],
            **request,
        }

    @staticmethod
    def _validate_issue_target(*, target_kind: str, target_id: str, issue_id: str) -> None:
        if target_kind != "issue-slice":
            raise AlbertError("Workstation action target kind must be issue-slice")
        if not issue_id.strip():
            raise AlbertError("Workstation action issue id must not be empty")
        if target_id != issue_id:
            raise AlbertError("Workstation action target id must match issue id")

    @staticmethod
    def _validate_session_target(*, target_kind: str, target_id: str, session_id: str) -> None:
        if target_kind != "agent-session":
            raise AlbertError("Workstation action target kind must be agent-session")
        if not session_id.strip():
            raise AlbertError("Workstation action session id must not be empty")
        if target_id != session_id:
            raise AlbertError("Workstation action target id must match session id")


class WorkingContextService:
    """Reconstructs bounded model input without changing governed mission state."""

    _recent_message_limit = 6
    _content_character_limit = 4_000
    _dispositions = {"included", "pinned", "excluded"}

    def __init__(self, snapshots: WorkspaceSnapshotService):
        self._snapshots = snapshots
        self._history = AgentConsoleHistoryService(snapshots)
        self._curation_path = snapshots.preferences_path.parent / "working-context-curation.json"

    @property
    def curation_path(self) -> Path:
        return self._curation_path

    def inspect(self) -> WorkingContextProjection:
        snapshot = self._snapshots.snapshot()
        curation = self._load_curation()
        messages = self._history.history()
        scoped_messages = [message for message in messages if message.scope == snapshot.conversation_scope]
        recent = scoped_messages[-self._recent_message_limit :]
        recent_ids = {message.message_id for message in recent}
        pinned_ids = set(curation["pinned_source_ids"])
        excluded_ids = set(curation["excluded_source_ids"])
        sources: list[WorkingContextSource] = [
            WorkingContextSource(
                source_id=f"workspace-session:{snapshot.workspace_session.id}",
                kind="workspace-session",
                label=f"Workspace Session {snapshot.workspace_session.id}",
                content=(
                    f"Workspace: {snapshot.workspace_session.workspace_path}; "
                    f"status: {snapshot.workspace_session.status}."
                ),
                governed=True,
                eligible=False,
                disposition="required",
            ),
            self._shared_context_source(snapshot),
        ]
        sources.extend(self._unresolved_sources(snapshot, pinned_ids, excluded_ids))
        for message in recent:
            source_id = f"message:{message.message_id}"
            sources.append(
                self._message_source(
                    message,
                    kind="recent-conversation",
                    disposition=self._disposition(source_id, pinned_ids, excluded_ids),
                )
            )
        for message in scoped_messages:
            source_id = f"message:{message.message_id}"
            if source_id in pinned_ids and message.message_id not in recent_ids:
                sources.append(
                    self._message_source(
                        message,
                        kind="deliberate-reference",
                        disposition="pinned",
                    )
                )
        bounded = self._bound_content(sources)
        return WorkingContextProjection(
            schema_version=1,
            revision=curation["revision"],
            scope=snapshot.conversation_scope,
            sources=tuple(bounded),
            content_character_count=sum(
                len(source.content) for source in bounded if source.disposition != "excluded"
            ),
        )

    @_atomic_workspace_action("_curation_path")
    def curate(
        self,
        *,
        source_id: str,
        disposition: Literal["included", "pinned", "excluded"],
        expected_revision: int,
    ) -> WorkingContextAcknowledgement:
        if disposition not in self._dispositions:
            raise AlbertError(f"Unknown Working Context disposition: {disposition}")
        current = self._load_curation()
        if expected_revision != current["revision"]:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=current["revision"],
            )
        if source_id not in self._eligible_source_ids():
            raise WorkingContextCurationError(source_id)
        pinned = set(current["pinned_source_ids"])
        excluded = set(current["excluded_source_ids"])
        pinned.discard(source_id)
        excluded.discard(source_id)
        if disposition == "pinned":
            pinned.add(source_id)
        elif disposition == "excluded":
            excluded.add(source_id)
        revision = current["revision"] + 1
        WorkspaceSnapshotService._write_json_atomically(
            self._curation_path,
            {
                "schema_version": 1,
                "revision": revision,
                "pinned_source_ids": sorted(pinned),
                "excluded_source_ids": sorted(excluded),
            },
        )
        return WorkingContextAcknowledgement(outcome="acknowledged", revision=revision)

    def _shared_context_source(self, snapshot: WorkspaceSnapshot) -> WorkingContextSource:
        mission = self._mission_for_scope(snapshot)
        content = f"Mission: {mission.prd_title}. Scope: {snapshot.conversation_scope.label}."
        if snapshot.conversation_scope.kind == "issue-slice":
            issue = mission.issues[snapshot.conversation_scope.target_id]
            criteria = "; ".join(issue.acceptance_criteria)
            content = (
                f"Mission: {mission.prd_title}. Issue {issue.id}: {issue.title}. "
                f"Accepted work: {issue.what_to_build}. Acceptance: {criteria}."
            )
        return WorkingContextSource(
            source_id=(
                f"shared-context:{mission.mission_id}:"
                f"{snapshot.conversation_scope.kind}:{snapshot.conversation_scope.target_id}"
            ),
            kind="shared-context",
            label=f"Shared Context — {snapshot.conversation_scope.label}",
            content=content,
            governed=True,
            eligible=False,
            disposition="required",
        )

    def _unresolved_sources(
        self,
        snapshot: WorkspaceSnapshot,
        pinned_ids: set[str],
        excluded_ids: set[str],
    ) -> list[WorkingContextSource]:
        mission = self._mission_for_scope(snapshot)
        if snapshot.conversation_scope.kind == "issue-slice":
            issue_ids = [snapshot.conversation_scope.target_id]
        else:
            issue_ids = mission.ordered_issue_ids()
        sources = []
        for issue_id in issue_ids:
            issue = mission.issues[issue_id]
            if issue.tracker_status.lower() in {"complete", "completed", "merged"}:
                continue
            source_id = f"issue:{mission.mission_id}:{issue_id}"
            sources.append(
                WorkingContextSource(
                    source_id=source_id,
                    kind="unresolved-item",
                    label=f"{issue.id} — {issue.title}",
                    content=(
                        f"Status: {issue.tracker_status}; review: {issue.review_state}; "
                        f"blockers: {', '.join(issue.blocked_by) if issue.blocked_by else 'none'}."
                    ),
                    governed=False,
                    eligible=True,
                    disposition=self._disposition(source_id, pinned_ids, excluded_ids),
                )
            )
        return sources

    @staticmethod
    def _message_source(
        message: AgentConsoleMessage,
        *,
        kind: Literal["recent-conversation", "deliberate-reference"],
        disposition: WorkingContextDisposition,
    ) -> WorkingContextSource:
        return WorkingContextSource(
            source_id=f"message:{message.message_id}",
            kind=kind,
            label=f"Agent Console message {message.sequence}",
            content=f"{message.source} ({message.outcome}): {message.content}",
            governed=False,
            eligible=True,
            disposition=disposition,
        )

    @staticmethod
    def _disposition(
        source_id: str, pinned_ids: set[str], excluded_ids: set[str]
    ) -> WorkingContextDisposition:
        if source_id in pinned_ids:
            return "pinned"
        if source_id in excluded_ids:
            return "excluded"
        return "included"

    def _eligible_source_ids(self) -> set[str]:
        snapshot = self._snapshots.snapshot()
        mission = self._mission_for_scope(snapshot)
        message_ids = {
            f"message:{message.message_id}"
            for message in self._history.history()
            if message.scope == snapshot.conversation_scope
        }
        if snapshot.conversation_scope.kind == "issue-slice":
            issue_ids = {
                f"issue:{mission.mission_id}:{snapshot.conversation_scope.target_id}"
            }
        else:
            issue_ids = {
                f"issue:{mission.mission_id}:{issue_id}" for issue_id in mission.issues
            }
        return message_ids | issue_ids

    def _mission_for_scope(self, snapshot: WorkspaceSnapshot) -> AlbertMission:
        mission_id = snapshot.conversation_scope.mission_id
        if mission_id is None and snapshot.active_mission is not None:
            mission_id = snapshot.active_mission.id
        mission = self._snapshots._missions.get(mission_id or "")
        if mission is None:
            raise WorkspacePersistenceError(
                f"Conversation Scope references unknown Mission: {mission_id}"
            )
        return mission

    def _bound_content(self, sources: list[WorkingContextSource]) -> list[WorkingContextSource]:
        remaining = self._content_character_limit
        bounded = []
        for source in sources:
            if source.disposition == "excluded":
                bounded.append(source)
                continue
            content = source.content[:remaining]
            remaining -= len(content)
            bounded.append(
                WorkingContextSource(
                    source_id=source.source_id,
                    kind=source.kind,
                    label=source.label,
                    content=content,
                    governed=source.governed,
                    eligible=source.eligible,
                    disposition=source.disposition,
                )
            )
        return bounded

    def _load_curation(self) -> dict[str, Any]:
        if not self._curation_path.exists():
            return {
                "schema_version": 1,
                "revision": 1,
                "pinned_source_ids": [],
                "excluded_source_ids": [],
            }
        try:
            data = json.loads(self._curation_path.read_text(encoding="utf-8"))
            if data["schema_version"] != 1:
                raise ValueError("unsupported Working Context curation schema")
            if not isinstance(data["revision"], int) or data["revision"] < 1:
                raise ValueError("Working Context revision must be positive")
            pinned = data["pinned_source_ids"]
            excluded = data["excluded_source_ids"]
            if not isinstance(pinned, list) or not all(isinstance(item, str) for item in pinned):
                raise ValueError("pinned source ids must be strings")
            if not isinstance(excluded, list) or not all(
                isinstance(item, str) for item in excluded
            ):
                raise ValueError("excluded source ids must be strings")
            if any(not item.strip() for item in [*pinned, *excluded]):
                raise ValueError("curated source ids must not be empty")
            if len(pinned) != len(set(pinned)) or len(excluded) != len(set(excluded)):
                raise ValueError("curated source ids must be unique")
            if set(pinned) & set(excluded):
                raise ValueError("a source cannot be both pinned and excluded")
            return data
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(
                f"Working Context curation persistence read failed: {exc}"
            ) from exc


class ActivityJournalService:
    """Persists the append-only Activity Journal for meaningful Workspace Session actions."""

    _actors = {"mission-commander", "orchestrator", "frontier-model", "local-agent"}

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots
        self._journal_path = snapshots.preferences_path.parent / "activity-journal.json"

    @property
    def journal_path(self) -> Path:
        return self._journal_path

    def inspect(
        self,
        *,
        search: str = "",
        mission_id: str = "",
        actor: ActivityActor | str = "",
        action_type: str = "",
        started_at: str = "",
        ended_at: str = "",
    ) -> ActivityJournalProjection:
        journal = self._load_journal()
        started = (
            self._parse_activity_timestamp(started_at, label="started_at")
            if started_at
            else None
        )
        ended = (
            self._parse_activity_timestamp(ended_at, label="ended_at") if ended_at else None
        )
        if started is not None and ended is not None and started > ended:
            raise AlbertError("Activity Journal started_at must be before ended_at")
        entries = tuple(
            entry
            for entry in journal["entries"]
            if self._matches_entry(
                entry,
                search=search,
                mission_id=mission_id,
                actor=actor,
                action_type=action_type,
                started_at=started,
                ended_at=ended,
            )
        )
        return ActivityJournalProjection(
            schema_version=1,
            revision=journal["revision"],
            entries=entries,
        )

    def record_workspace_action(
        self,
        *,
        correlation_id: str,
        snapshot: WorkspaceSnapshot,
    ) -> ActivityJournalEntry:
        if not correlation_id.strip():
            raise AlbertError("Activity Journal correlation id must not be empty")
        active = snapshot.active_mission
        affected_entities = (
            ActivityAffectedEntity(
                entity_type="workspace-session",
                entity_id=snapshot.workspace_session.id,
                label=snapshot.workspace_session.id,
            ),
            *(
                (
                    ActivityAffectedEntity(
                        entity_type="mission",
                        entity_id=active.id,
                        label=active.title,
                    ),
                )
                if active is not None
                else ()
            ),
        )
        return self._append(
            actor="mission-commander",
            action_type="operations-view-selected",
            summary=(
                f"Mission Commander selected Operations Workspace view "
                f"{self._view_label(snapshot.operations_view)}."
            ),
            affected_entities=affected_entities,
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_shell_command_approved(
        self,
        *,
        correlation_id: str,
        snapshot: WorkspaceSnapshot,
        command_record: dict[str, Any],
        approver: str,
    ) -> ActivityJournalEntry:
        actor: ActivityActor = (
            "frontier-model" if approver == "frontier-model" else "mission-commander"
        )
        approver_label = (
            "Frontier Model" if actor == "frontier-model" else "Mission Commander"
        )
        return self._append(
            actor=actor,
            action_type="shell-command-approved",
            summary=(
                f"{approver_label} approved Shell Terminal command "
                f"{command_record['command_id']}."
            ),
            affected_entities=self._shell_command_entities(
                snapshot=snapshot,
                command_record=command_record,
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_review_decision(
        self,
        *,
        correlation_id: str,
        mission: AlbertMission,
        issue_id: str,
        issue_title: str,
        session_id: str,
        review_outcome: str,
        next_action: str,
        evidence_links: tuple[str, ...],
    ) -> ActivityJournalEntry:
        if not correlation_id.strip():
            raise AlbertError("Activity Journal correlation id must not be empty")
        issue_entity_type = "issue-slice" if issue_id.startswith("ISS-") else "ad-hoc-delegation"
        evidence_href = (
            evidence_links[0]
            if evidence_links
            else f"app-local://missions/{mission.mission_id}/sessions/{session_id}/evidence"
        )
        affected_entities = (
            ActivityAffectedEntity(
                entity_type="mission",
                entity_id=mission.mission_id,
                label=mission.prd_title,
                href=f"app-local://missions/{mission.mission_id}",
            ),
            ActivityAffectedEntity(
                entity_type=issue_entity_type,
                entity_id=issue_id,
                label=issue_title,
                href=f"app-local://missions/{mission.mission_id}/issues/{issue_id}",
            ),
            ActivityAffectedEntity(
                entity_type="local-agent-session",
                entity_id=session_id,
                label=session_id,
                href=f"app-local://missions/{mission.mission_id}/sessions/{session_id}",
            ),
            ActivityAffectedEntity(
                entity_type="evidence-package",
                entity_id=session_id,
                label=f"Evidence Package {session_id}",
                href=evidence_href,
            ),
        )
        return self._append(
            actor="mission-commander",
            action_type="review-decision",
            summary=(
                f"Mission Commander recorded Review Workspace decision {review_outcome} "
                f"for {issue_id} from {session_id}; next action is {next_action}."
            ),
            affected_entities=affected_entities,
            evidence_links=evidence_links,
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_workspace_queue_decision(
        self,
        *,
        correlation_id: str,
        item: WorkspaceQueueItem,
        item_status: WorkspaceQueueItemStatus,
        effect_summary: str,
    ) -> ActivityJournalEntry:
        if not correlation_id.strip():
            raise AlbertError("Activity Journal correlation id must not be empty")
        mission = self._snapshots._missions[item.mission_id]
        governed_entity_type = (
            "issue-slice" if item.issue_id.startswith("ISS-") else "ad-hoc-delegation"
        )
        governed_label = (
            mission.issues[item.issue_id].title
            if item.issue_id in mission.issues
            else item.issue_id
        )
        queue_href = f"workspace-queue#{item.item_id}"
        status_label = item_status.title()
        return self._append(
            actor="mission-commander",
            action_type="workspace-queue-decision",
            summary=(
                f"Mission Commander recorded {status_label} Workspace Queue item "
                f"{item.item_id}. {effect_summary}"
            ),
            affected_entities=(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=mission.mission_id,
                    label=mission.prd_title,
                    href=f"app-local://missions/{mission.mission_id}",
                ),
                ActivityAffectedEntity(
                    entity_type=governed_entity_type,
                    entity_id=item.issue_id,
                    label=governed_label,
                    href=(
                        f"app-local://missions/{mission.mission_id}/issues/"
                        f"{item.issue_id}"
                    ),
                ),
                ActivityAffectedEntity(
                    entity_type="workspace-queue-item",
                    entity_id=item.item_id,
                    label=item.requested_action,
                    href=queue_href,
                ),
                ActivityAffectedEntity(
                    entity_type="workspace-queue-decision",
                    entity_id=correlation_id,
                    label=f"{status_label} {item.item_id}",
                    href=queue_href,
                ),
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_mission_draft_created(
        self,
        *,
        correlation_id: str,
        draft: MissionDraft,
        effect_summary: str,
    ) -> ActivityJournalEntry:
        if not correlation_id.strip():
            raise AlbertError("Activity Journal correlation id must not be empty")
        mission = self._snapshots._missions[draft.mission_id]
        return self._append(
            actor="mission-commander",
            action_type="mission-draft-created",
            summary=f"Mission Commander created {draft.draft_id}. {effect_summary}",
            affected_entities=(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=mission.mission_id,
                    label=mission.prd_title,
                    href=f"app-local://missions/{mission.mission_id}",
                ),
                ActivityAffectedEntity(
                    entity_type="mission-draft",
                    entity_id=draft.draft_id,
                    label=draft.proposed_goal,
                    href=f"workspace-queue#{draft.draft_id}",
                ),
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_mission_draft_confirmed(
        self,
        *,
        correlation_id: str,
        draft: MissionDraft,
        issue: IssueSlice,
        effect_summary: str,
    ) -> ActivityJournalEntry:
        if not correlation_id.strip():
            raise AlbertError("Activity Journal correlation id must not be empty")
        mission = self._snapshots._missions[draft.mission_id]
        return self._append(
            actor="mission-commander",
            action_type="mission-draft-confirmed",
            summary=f"Mission Commander confirmed {draft.draft_id}. {effect_summary}",
            affected_entities=(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=mission.mission_id,
                    label=mission.prd_title,
                    href=f"app-local://missions/{mission.mission_id}",
                ),
                ActivityAffectedEntity(
                    entity_type="mission-draft",
                    entity_id=draft.draft_id,
                    label=draft.proposed_goal,
                    href=f"workspace-queue#{draft.draft_id}",
                ),
                ActivityAffectedEntity(
                    entity_type="issue-slice",
                    entity_id=issue.id,
                    label=issue.title,
                    href=(
                        f"app-local://missions/{mission.mission_id}/issues/"
                        f"{issue.id}"
                    ),
                ),
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_mission_draft_updated(
        self,
        *,
        correlation_id: str,
        draft: MissionDraft,
        effect_summary: str,
    ) -> ActivityJournalEntry:
        if not correlation_id.strip():
            raise AlbertError("Activity Journal correlation id must not be empty")
        mission = self._snapshots._missions[draft.mission_id]
        return self._append(
            actor="mission-commander",
            action_type="mission-draft-updated",
            summary=f"Mission Commander updated {draft.draft_id}. {effect_summary}",
            affected_entities=(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=mission.mission_id,
                    label=mission.prd_title,
                    href=f"app-local://missions/{mission.mission_id}",
                ),
                ActivityAffectedEntity(
                    entity_type="mission-draft",
                    entity_id=draft.draft_id,
                    label=draft.proposed_goal,
                    href=f"workspace-queue#{draft.draft_id}",
                ),
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_mission_draft_abandoned(
        self,
        *,
        correlation_id: str,
        draft: MissionDraft,
        effect_summary: str,
    ) -> ActivityJournalEntry:
        if not correlation_id.strip():
            raise AlbertError("Activity Journal correlation id must not be empty")
        mission = self._snapshots._missions[draft.mission_id]
        return self._append(
            actor="mission-commander",
            action_type="mission-draft-abandoned",
            summary=f"Mission Commander abandoned {draft.draft_id}. {effect_summary}",
            affected_entities=(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=mission.mission_id,
                    label=mission.prd_title,
                    href=f"app-local://missions/{mission.mission_id}",
                ),
                ActivityAffectedEntity(
                    entity_type="mission-draft",
                    entity_id=draft.draft_id,
                    label=draft.proposed_goal,
                    href=f"workspace-queue#{draft.draft_id}",
                ),
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_workstation_action(
        self,
        *,
        correlation_id: str,
        actor: ActivityActor,
        action_type: str,
        mission: AlbertMission,
        issue_id: str,
        session_id: str,
        effect_summary: str,
    ) -> ActivityJournalEntry:
        if not correlation_id.strip():
            raise AlbertError("Activity Journal correlation id must not be empty")
        issue_entity_type = "issue-slice" if issue_id.startswith("ISS-") else "ad-hoc-delegation"
        issue_label = (
            mission.issues[issue_id].title
            if issue_id in mission.issues
            else issue_id or "No issue"
        )
        affected_entities = [
            ActivityAffectedEntity(
                entity_type="mission",
                entity_id=mission.mission_id,
                label=mission.prd_title,
                href=f"app-local://missions/{mission.mission_id}",
            )
        ]
        if issue_id:
            affected_entities.append(
                ActivityAffectedEntity(
                    entity_type=issue_entity_type,
                    entity_id=issue_id,
                    label=issue_label,
                    href=f"app-local://missions/{mission.mission_id}/issues/{issue_id}",
                )
            )
        if session_id:
            affected_entities.append(
                ActivityAffectedEntity(
                    entity_type="local-agent-session",
                    entity_id=session_id,
                    label=session_id,
                    href=f"app-local://missions/{mission.mission_id}/sessions/{session_id}",
                )
            )
        return self._append(
            actor=actor,
            action_type=action_type,
            summary=effect_summary,
            affected_entities=tuple(affected_entities),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_frontier_confirmation_requested(
        self,
        *,
        correlation_id: str,
        item: WorkspaceQueueItem,
    ) -> ActivityJournalEntry:
        if not correlation_id.strip():
            raise AlbertError("Activity Journal correlation id must not be empty")
        mission = self._snapshots._missions[item.mission_id]
        queue_href = f"workspace-queue#{item.item_id}"
        return self._append(
            actor="frontier-model",
            action_type="frontier-confirmation-requested",
            summary=(
                f"Frontier Model requested {item.item_id}: {item.requested_action}. "
                f"{item.consequence}"
            ),
            affected_entities=(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=mission.mission_id,
                    label=mission.prd_title,
                    href=f"app-local://missions/{mission.mission_id}",
                ),
                ActivityAffectedEntity(
                    entity_type="issue-slice",
                    entity_id=item.issue_id,
                    label=mission.issues[item.issue_id].title,
                    href=(
                        f"app-local://missions/{mission.mission_id}/issues/"
                        f"{item.issue_id}"
                    ),
                ),
                ActivityAffectedEntity(
                    entity_type="workspace-queue-item",
                    entity_id=item.item_id,
                    label=item.requested_action,
                    href=queue_href,
                ),
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_orchestrator_session_launched(
        self,
        *,
        correlation_id: str,
        item: WorkspaceQueueItem,
        session: LocalAgentSession,
    ) -> ActivityJournalEntry:
        if not correlation_id.strip():
            raise AlbertError("Activity Journal correlation id must not be empty")
        mission = self._snapshots._missions[item.mission_id]
        return self._append(
            actor="orchestrator",
            action_type="local-agent-session-launched",
            summary=(
                f"Orchestrator queued {session.session_id} for {item.issue_id} "
                f"within the acknowledged Workspace Queue boundaries."
            ),
            affected_entities=(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=mission.mission_id,
                    label=mission.prd_title,
                    href=f"app-local://missions/{mission.mission_id}",
                ),
                ActivityAffectedEntity(
                    entity_type="ad-hoc-delegation",
                    entity_id=item.issue_id,
                    label=session.task_packet.get("goal", item.issue_id),
                    href=(
                        f"app-local://missions/{mission.mission_id}/issues/"
                        f"{item.issue_id}"
                    ),
                ),
                ActivityAffectedEntity(
                    entity_type="local-agent-session",
                    entity_id=session.session_id,
                    label=session.session_id,
                    href=(
                        f"app-local://missions/{mission.mission_id}/sessions/"
                        f"{session.session_id}"
                    ),
                ),
                ActivityAffectedEntity(
                    entity_type="workspace-queue-item",
                    entity_id=item.item_id,
                    label=item.requested_action,
                    href=f"workspace-queue#{item.item_id}",
                ),
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_shell_command_approval_requested(
        self,
        *,
        correlation_id: str,
        snapshot: WorkspaceSnapshot,
        command_record: dict[str, Any],
        required_approver: str,
    ) -> ActivityJournalEntry:
        return self._append(
            actor="orchestrator",
            action_type="shell-command-approval-requested",
            summary=(
                f"Orchestrator requested {required_approver} approval for "
                f"Shell Terminal command {command_record['command_id']}."
            ),
            affected_entities=self._shell_command_entities(
                snapshot=snapshot,
                command_record=command_record,
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_shell_command_denied(
        self,
        *,
        correlation_id: str,
        snapshot: WorkspaceSnapshot,
        command_record: dict[str, Any],
        reason: str,
    ) -> ActivityJournalEntry:
        return self._append(
            actor="mission-commander",
            action_type="shell-command-denied",
            summary=(
                f"Mission Commander denied Shell Terminal command "
                f"{command_record['command_id']}: {reason}"
            ),
            affected_entities=self._shell_command_entities(
                snapshot=snapshot,
                command_record=command_record,
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_shell_command_finished(
        self,
        *,
        correlation_id: str,
        snapshot: WorkspaceSnapshot,
        command_record: dict[str, Any],
    ) -> ActivityJournalEntry:
        status = command_record["status"]
        action_type = "shell-command-completed" if status == "completed" else "shell-command-failed"
        return self._append(
            actor="orchestrator",
            action_type=action_type,
            summary=(
                f"Orchestrator recorded Shell Terminal command "
                f"{command_record['command_id']} {status} with exit code "
                f"{command_record['exit_code']}."
            ),
            affected_entities=self._shell_command_entities(
                snapshot=snapshot,
                command_record=command_record,
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_shell_command_outcome_unknown(
        self,
        *,
        correlation_id: str,
        snapshot: WorkspaceSnapshot,
        command_record: dict[str, Any],
    ) -> ActivityJournalEntry:
        return self._append(
            actor="orchestrator",
            action_type="shell-command-outcome-unknown",
            summary=(
                f"Orchestrator recorded that Shell Terminal command "
                f"{command_record['command_id']} started but its final outcome is unknown; "
                "it will not be retried automatically."
            ),
            affected_entities=self._shell_command_entities(
                snapshot=snapshot,
                command_record=command_record,
            ),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_additional_path_grant_created(
        self,
        *,
        correlation_id: str,
        snapshot: WorkspaceSnapshot,
        grant: AdditionalPathGrant,
    ) -> ActivityJournalEntry:
        active = snapshot.active_mission
        entities = [
            ActivityAffectedEntity(
                entity_type="workspace-session",
                entity_id=snapshot.workspace_session.id,
                label=snapshot.workspace_session.id,
            )
        ]
        if active is not None:
            entities.append(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=active.id,
                    label=active.title,
                    href=f"app-local://missions/{active.id}",
                )
            )
        entities.append(
            ActivityAffectedEntity(
                entity_type="additional-path-grant",
                entity_id=grant.grant_id,
                label=grant.path,
                href=f"app-local://workspace/shell-terminal#grant-{grant.grant_id}",
            )
        )
        return self._append(
            actor="mission-commander",
            action_type="additional-path-grant-created",
            summary=(
                f"Mission Commander created {grant.access_level} Additional Path Grant "
                f"{grant.grant_id} for {grant.path} for {grant.duration_seconds} seconds "
                f"via {correlation_id}."
            ),
            affected_entities=tuple(entities),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_additional_path_grant_denied(
        self,
        *,
        correlation_id: str,
        snapshot: WorkspaceSnapshot,
        denial: AdditionalPathGrantDenial,
    ) -> ActivityJournalEntry:
        active = snapshot.active_mission
        entities = [
            ActivityAffectedEntity(
                entity_type="workspace-session",
                entity_id=snapshot.workspace_session.id,
                label=snapshot.workspace_session.id,
            )
        ]
        if active is not None:
            entities.append(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=active.id,
                    label=active.title,
                    href=f"app-local://missions/{active.id}",
                )
            )
        entities.append(
            ActivityAffectedEntity(
                entity_type="additional-path-grant-request",
                entity_id=denial.request_id,
                label=denial.path,
                href=(
                    "app-local://workspace/shell-terminal#"
                    f"grant-request-{denial.request_id}"
                ),
            )
        )
        return self._append(
            actor="mission-commander",
            action_type="additional-path-grant-denied",
            summary=(
                f"Mission Commander denied {denial.access_level} Additional Path Grant "
                f"request {denial.request_id} for {denial.path} for "
                f"{denial.duration_seconds} seconds; affected action: "
                f"{denial.affected_action}. Reason: {denial.reason}"
            ),
            affected_entities=tuple(entities),
            evidence_links=(),
            correlation_id=correlation_id,
            replay_existing=True,
        )

    def record_local_agent_evidence(
        self,
        mission_id: str,
        session: LocalAgentSession,
        evidence: EvidencePackage,
    ) -> ActivityJournalEntry:
        mission = self._snapshots._missions[mission_id]
        issue_entity_type = (
            "issue-slice" if session.issue_id.startswith("ISS-") else "ad-hoc-delegation"
        )
        issue_label = (
            mission.issues[session.issue_id].title
            if session.issue_id in mission.issues
            else str(session.task_packet.get("goal", session.issue_id))
        )
        evidence_links = tuple(mission.review_artifact_links(session))
        evidence_href = (
            evidence_links[0]
            if evidence_links
            else f"app-local://missions/{mission_id}/sessions/{session.session_id}/evidence"
        )
        return self._append(
            actor="local-agent",
            action_type="evidence-package-submitted",
            summary=(
                f"Local Agent {session.assigned_agent} submitted validated evidence "
                f"for {session.session_id}."
            ),
            affected_entities=(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=mission_id,
                    label=mission.prd_title,
                    href=f"app-local://missions/{mission_id}",
                ),
                ActivityAffectedEntity(
                    entity_type=issue_entity_type,
                    entity_id=session.issue_id,
                    label=issue_label,
                    href=f"app-local://missions/{mission_id}/issues/{session.issue_id}",
                ),
                ActivityAffectedEntity(
                    entity_type="local-agent-session",
                    entity_id=session.session_id,
                    label=session.session_id,
                    href=(
                        f"app-local://missions/{mission_id}/sessions/"
                        f"{session.session_id}"
                    ),
                ),
                ActivityAffectedEntity(
                    entity_type="evidence-package",
                    entity_id=session.session_id,
                    label=f"Evidence Package {session.session_id}",
                    href=evidence_href,
                ),
            ),
            evidence_links=evidence_links,
            correlation_id=session.evidence_correlation_id,
            replay_existing=True,
        )

    def _shell_command_entities(
        self,
        *,
        snapshot: WorkspaceSnapshot,
        command_record: dict[str, Any],
    ) -> tuple[ActivityAffectedEntity, ...]:
        mission_id = str(command_record.get("mission_id", ""))
        mission = self._snapshots._missions.get(mission_id)
        active = snapshot.active_mission
        entities = [
            ActivityAffectedEntity(
                entity_type="workspace-session",
                entity_id=snapshot.workspace_session.id,
                label=snapshot.workspace_session.id,
            )
        ]
        if mission is not None:
            entities.append(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=mission.mission_id,
                    label=mission.prd_title,
                    href=f"app-local://missions/{mission.mission_id}",
                )
            )
        elif active is not None:
            entities.append(
                ActivityAffectedEntity(
                    entity_type="mission",
                    entity_id=active.id,
                    label=active.title,
                    href=f"app-local://missions/{active.id}",
                )
            )
        entities.append(
            ActivityAffectedEntity(
                entity_type="shell-command",
                entity_id=command_record["command_id"],
                label=command_record["classification"],
                href=(
                    "app-local://workspace/shell-terminal#"
                    f"{command_record['command_id']}"
                ),
            )
        )
        return tuple(entities)

    @_causal_chronology
    def _append(
        self,
        *,
        actor: ActivityActor,
        action_type: str,
        summary: str,
        affected_entities: tuple[ActivityAffectedEntity, ...],
        evidence_links: tuple[str, ...],
        correlation_id: str,
        replay_existing: bool = False,
    ) -> ActivityJournalEntry:
        if actor not in self._actors:
            raise AlbertError(f"Unknown Activity Journal actor: {actor}")
        if not action_type.strip():
            raise AlbertError("Activity Journal action type must not be empty")
        if not summary.strip():
            raise AlbertError("Activity Journal summary must not be empty")
        if any(not isinstance(link, str) or not link.strip() for link in evidence_links):
            raise AlbertError("Activity Journal evidence links must not be empty")
        if not (
            action_type.startswith("shell-")
            or action_type.startswith("additional-path-grant-")
        ):
            ShellTerminalService(self._snapshots).reconcile_audit()
        try:
            with WorkspaceSnapshotService._json_store_lock(self._journal_path):
                journal = self._load_journal()
                if replay_existing:
                    existing = [
                        entry
                        for entry in journal["entries"]
                        if entry.correlation_id == correlation_id
                        and entry.action_type == action_type
                    ]
                    if len(existing) > 1:
                        raise WorkspacePersistenceError(
                            "Activity Journal contains duplicate audit phases for "
                            f"{correlation_id}: {action_type}."
                        )
                    if existing:
                        entry = existing[0]
                        if (
                            entry.actor != actor
                            or entry.summary != summary
                            or tuple(
                                (
                                    entity.entity_type,
                                    entity.entity_id,
                                    entity.href,
                                )
                                for entity in entry.affected_entities
                            )
                            != tuple(
                                (
                                    entity.entity_type,
                                    entity.entity_id,
                                    entity.href,
                                )
                                for entity in affected_entities
                            )
                            or entry.evidence_links != evidence_links
                        ):
                            raise AlbertError(
                                "Activity Journal correlation id and action type were already "
                                "used for a different audit effect."
                            )
                        return entry
                sequence = len(journal["entries"]) + 1
                entry = ActivityJournalEntry(
                    entry_id=f"activity-{sequence:06d}",
                    sequence=sequence,
                    recorded_at=datetime.now(timezone.utc)
                    .isoformat(timespec="seconds")
                    .replace("+00:00", "Z"),
                    actor=actor,
                    action_type=action_type,
                    summary=summary,
                    affected_entities=affected_entities,
                    evidence_links=evidence_links,
                    correlation_id=correlation_id,
                )
                revision = journal["revision"] + 1
                WorkspaceSnapshotService._write_json_atomically(
                    self._journal_path,
                    {
                        "schema_version": 1,
                        "revision": revision,
                        "entries": [asdict(item) for item in [*journal["entries"], entry]],
                    },
                )
        except OSError as exc:
            raise WorkspacePersistenceError(
                f"Activity Journal persistence write failed: {exc}"
            ) from exc
        return entry

    def _load_journal(self) -> dict[str, Any]:
        if not self._journal_path.exists():
            return {"schema_version": 1, "revision": 0, "entries": []}
        try:
            payload = json.loads(self._journal_path.read_text(encoding="utf-8"))
            if payload["schema_version"] != 1:
                raise ValueError("unsupported Activity Journal schema")
            if not isinstance(payload["revision"], int) or payload["revision"] < 0:
                raise ValueError("Activity Journal revision must be a non-negative integer")
            if not isinstance(payload["entries"], list):
                raise ValueError("Activity Journal entries must be a list")
            entries = [self._parse_entry(item) for item in payload["entries"]]
            sequences = [entry.sequence for entry in entries]
            if sequences != list(range(1, len(entries) + 1)):
                raise ValueError("Activity Journal entry sequence must be contiguous")
            expected_ids = [
                f"activity-{sequence:06d}" for sequence in range(1, len(entries) + 1)
            ]
            if [entry.entry_id for entry in entries] != expected_ids:
                raise ValueError("Activity Journal entry ids must match sequence")
            if payload["revision"] != len(entries):
                raise ValueError("Activity Journal revision must match entry count")
            return {"schema_version": 1, "revision": payload["revision"], "entries": entries}
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(
                f"Activity Journal persistence read failed: {exc}"
            ) from exc

    def _parse_entry(self, item: dict[str, Any]) -> ActivityJournalEntry:
        actor = item["actor"]
        if actor not in self._actors:
            raise ValueError(f"unknown Activity Journal actor: {actor}")
        affected_entities = tuple(
            ActivityAffectedEntity(
                entity_type=entity["entity_type"],
                entity_id=entity["entity_id"],
                label=entity["label"],
                href=entity.get("href", ""),
            )
            for entity in item["affected_entities"]
        )
        if any(
            not entity.entity_type.strip()
            or not entity.entity_id.strip()
            or not entity.label.strip()
            for entity in affected_entities
        ):
            raise ValueError("Activity Journal affected entities must be named")
        evidence_links = tuple(item.get("evidence_links", ()))
        if any(not isinstance(link, str) or not link.strip() for link in evidence_links):
            raise ValueError("Activity Journal evidence links must be non-empty strings")
        self._parse_activity_timestamp(item["recorded_at"], label="recorded_at")
        return ActivityJournalEntry(
            entry_id=item["entry_id"],
            sequence=item["sequence"],
            recorded_at=item["recorded_at"],
            actor=actor,
            action_type=item["action_type"],
            summary=item["summary"],
            affected_entities=affected_entities,
            evidence_links=evidence_links,
            correlation_id=item["correlation_id"],
        )

    @staticmethod
    def _matches_entry(
        entry: ActivityJournalEntry,
        *,
        search: str,
        mission_id: str,
        actor: ActivityActor | str,
        action_type: str,
        started_at: datetime | None,
        ended_at: datetime | None,
    ) -> bool:
        if actor and entry.actor != actor:
            return False
        if action_type and entry.action_type != action_type:
            return False
        recorded_at = ActivityJournalService._parse_activity_timestamp(
            entry.recorded_at, label="recorded_at"
        )
        if started_at is not None and recorded_at < started_at:
            return False
        if ended_at is not None and recorded_at > ended_at:
            return False
        if mission_id and not any(
            entity.entity_type == "mission" and entity.entity_id == mission_id
            for entity in entry.affected_entities
        ):
            return False
        query = search.strip().casefold()
        if query:
            haystack = " ".join(
                [
                    entry.entry_id,
                    entry.correlation_id,
                    entry.actor,
                    entry.action_type,
                    entry.summary,
                    *(
                        f"{entity.entity_type} {entity.entity_id} {entity.label}"
                        for entity in entry.affected_entities
                    ),
                    *entry.evidence_links,
                ]
            ).casefold()
            if query not in haystack:
                return False
        return True

    @staticmethod
    def _parse_activity_timestamp(value: Any, *, label: str) -> datetime:
        if not isinstance(value, str) or not value.strip():
            raise ValueError(f"Activity Journal {label} must be an ISO timestamp")
        try:
            parsed = datetime.fromisoformat(value.strip().replace("Z", "+00:00"))
        except ValueError as exc:
            raise ValueError(f"Activity Journal {label} must be an ISO timestamp") from exc
        if parsed.tzinfo is None:
            raise ValueError(f"Activity Journal {label} must include a timezone")
        return parsed.astimezone(timezone.utc)

    @staticmethod
    def _view_label(operations_view: str) -> str:
        return {
            "mission-board": "Mission Board",
            "review-workspace": "Review Workspace",
            "workspace-queue": "Workspace Queue",
            "activity": "Activity",
        }.get(operations_view, operations_view)


class WorkspaceSyncService:
    """Applies correlated semantic actions against acknowledged workspace state."""

    def __init__(self, snapshots: WorkspaceSnapshotService):
        self._snapshots = snapshots

    @_atomic_workspace_action()
    def submit_action(self, action: WorkspaceAction) -> WorkspaceActionAcknowledgement:
        current = self._snapshots.snapshot()
        if action.expected_revision != current.revision:
            raise WorkspaceStaleActionError(
                expected_revision=action.expected_revision,
                current_revision=current.revision,
            )
        updated = self._snapshots._update_preferences_locked(
            active_mission_id=action.active_mission_id,
            conversation_scope=action.conversation_scope,
            operations_view=action.operations_view,
            event_metadata={"correlation_id": action.correlation_id},
        )
        ActivityJournalService(self._snapshots).record_workspace_action(
            correlation_id=action.correlation_id,
            snapshot=updated,
        )
        return WorkspaceActionAcknowledgement(
            correlation_id=action.correlation_id,
            outcome="acknowledged",
            revision=updated.revision,
        )

    def updates_after(self, revision: int) -> WorkspaceUpdateBatch:
        if revision < 0:
            raise AlbertError("Workspace revision must not be negative.")
        current = self._snapshots.snapshot().revision
        if revision > current:
            raise WorkspaceRevisionGapError(
                requested_revision=revision,
                current_revision=current,
            )
        events = tuple(event for event in self._snapshots.events() if event.revision > revision)
        expected_revisions = list(range(revision + 1, current + 1))
        if [event.revision for event in events] != expected_revisions:
            raise WorkspaceRevisionGapError(
                requested_revision=revision,
                current_revision=current,
            )
        return WorkspaceUpdateBatch(
            after_revision=revision,
            current_revision=current,
            events=events,
        )


class WorkspaceSnapshotService:
    """Builds the desktop projection from authoritative Orchestrator state."""

    def __init__(
        self, mission: AlbertMission, *, missions: tuple[AlbertMission, ...] = ()
    ):
        all_missions = (mission, *missions)
        mission_ids = [item.mission_id for item in all_missions]
        if len(mission_ids) != len(set(mission_ids)):
            raise AlbertError("Workspace Mission ids must be unique")
        if any(item.target_repo != mission.target_repo for item in all_missions):
            raise AlbertError("Workspace Missions must target the same open repository")
        if any(item.runtime_root != mission.runtime_root for item in all_missions):
            raise AlbertError("Workspace Missions must share the same runtime root")
        self._primary_mission = mission
        self._mission = mission
        self._missions = {item.mission_id: item for item in all_missions}
        self._preferences_path = mission.runtime_dir / "workspace-preferences.json"
        self._action_lock_target = mission.runtime_dir / "workspace-action-transaction"
        evidence_activity_recorder = ActivityJournalService(self).record_local_agent_evidence
        for item in all_missions:
            item._workspace_preferences_path = self._preferences_path
            item._evidence_activity_recorder = evidence_activity_recorder

    @property
    def preferences_path(self) -> Path:
        return self._preferences_path

    def active_retirement_storage_inspection(self) -> dict[str, Any]:
        """Inspect storage for the Active Mission, never the primary fallback."""

        preferences = self._load_preferences()
        mission = self._missions.get(preferences["active_mission_id"])
        if mission is None:
            raise WorkspacePersistenceError(
                "Workspace preferences reference an unknown Active Mission."
            )
        return mission.retirement_storage_inspection()

    @measured_stage("S6", workflows={"startup"})
    def snapshot(self) -> WorkspaceSnapshot:
        AgentConsoleHistoryService(self).reconcile_supervision_receipts()
        preferences = self._load_preferences()
        active = self._missions.get(preferences["active_mission_id"])
        if active is None:
            raise WorkspacePersistenceError(
                f"Workspace preferences reference unknown Active Mission: "
                f"{preferences['active_mission_id']}"
            )
        board = active.board_summary()
        status: Literal["ready", "empty"] = "ready" if board["issue_count"] else "empty"
        active_mission = MissionSummary(
            id=active.mission_id,
            title=active.prd_title,
            issue_count=board["issue_count"],
        )
        scope = self._qualify_scope(
            ConversationScope(**preferences["conversation_scope"]),
            active_mission_id=active.mission_id,
        )
        try:
            queue = WorkspaceQueueService(self)._load_queue()
        except WorkspacePersistenceError:
            queue = None
        try:
            journal_entries = ActivityJournalService(self).inspect().entries
        except WorkspacePersistenceError:
            journal_entries = ()
        try:
            workspace_events = self.events()
        except WorkspacePersistenceError:
            workspace_events = ()
        return WorkspaceSnapshot(
            schema_version=1,
            revision=preferences["revision"],
            workspace_session=WorkspaceSessionSummary(
                id=self._primary_mission.project_key,
                workspace_path=str(self._primary_mission.target_repo),
                status=status,
            ),
            active_mission=active_mission,
            conversation_scope=scope,
            operations_view=preferences["operations_view"],
            mission_board=board,
            missions=tuple(
                self._mission_summary(
                    item,
                    is_active=item.mission_id == active.mission_id,
                    preferences=preferences,
                    queue=queue,
                    journal_entries=journal_entries,
                    workspace_events=workspace_events,
                )
                for item in self._missions.values()
            ),
        )

    def update_preferences(
        self,
        *,
        active_mission_id: str,
        conversation_scope: ConversationScope,
        operations_view: str,
        event_metadata: dict[str, str] | None = None,
    ) -> WorkspaceSnapshot:
        with self._action_store_lock(self._preferences_path):
            return self._update_preferences_locked(
                active_mission_id=active_mission_id,
                conversation_scope=conversation_scope,
                operations_view=operations_view,
                event_metadata=event_metadata,
            )

    def _update_preferences_locked(
        self,
        *,
        active_mission_id: str,
        conversation_scope: ConversationScope,
        operations_view: str,
        event_metadata: dict[str, str] | None = None,
        workstation_receipt: dict[str, Any] | None = None,
    ) -> WorkspaceSnapshot:
        if active_mission_id not in self._missions:
            raise AlbertError(f"Unknown Active Mission: {active_mission_id}")
        if operations_view not in {"mission-board", "review-workspace", "workspace-queue", "activity"}:
            raise AlbertError(f"Unknown Operations Workspace view: {operations_view}")
        if (
            conversation_scope.kind == "working-directory"
            and Path(conversation_scope.target_id).resolve()
            != self._primary_mission.target_repo.resolve()
        ):
            raise AlbertError("Working directory Conversation Scope must target the open repository")
        conversation_scope = self._qualify_scope(
            conversation_scope, active_mission_id=active_mission_id
        )
        current = self._load_preferences()
        revision = current["revision"] + 1
        events = list(current["events"])
        workstation_receipts = list(current["workstation_receipts"])
        if event_metadata is not None:
            correlation_id = event_metadata["correlation_id"]
            events.append(
                {
                    "event_id": f"workspace-{revision}-{correlation_id}",
                    "correlation_id": correlation_id,
                    "revision": revision,
                    "kind": "workspace-preferences-updated",
                    "active_mission_id": active_mission_id,
                    "conversation_scope": asdict(conversation_scope),
                    "operations_view": operations_view,
                }
            )
        if workstation_receipt is not None:
            workstation_receipts.append(workstation_receipt)
        data = {
            "revision": revision,
            "active_mission_id": active_mission_id,
            "conversation_scope": asdict(conversation_scope),
            "operations_view": operations_view,
            "events": events,
            "workstation_receipts": workstation_receipts,
        }
        self._write_json_atomically(self._preferences_path, data)
        return self.snapshot()

    def events(self) -> tuple[WorkspaceEvent, ...]:
        persisted = self._load_preferences()
        raw_events = persisted["events"]
        try:
            events = tuple(
                WorkspaceEvent(
                    event_id=event["event_id"],
                    correlation_id=event["correlation_id"],
                    revision=event["revision"],
                    kind=event["kind"],
                    active_mission_id=event["active_mission_id"],
                    conversation_scope=ConversationScope(**event["conversation_scope"]),
                    operations_view=event["operations_view"],
                )
                for event in raw_events
            )
            revisions = [event.revision for event in events]
            event_ids = [event.event_id for event in events]
            if revisions != sorted(set(revisions)):
                raise ValueError("event revisions must be unique and strictly ordered")
            if any(revision < 2 or revision > persisted["revision"] for revision in revisions):
                raise ValueError("event revision is outside acknowledged workspace state")
            if len(event_ids) != len(set(event_ids)):
                raise ValueError("event ids must be unique")
            return events
        except (KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(f"Workspace event persistence read failed: {exc}") from exc

    def _load_preferences(self) -> dict[str, Any]:
        if not self._preferences_path.exists():
            return {
                "revision": 1,
                "active_mission_id": self._mission.mission_id,
                "conversation_scope": asdict(
                    ConversationScope(
                        kind="working-directory",
                        target_id=str(self._mission.target_repo),
                        label=self._mission.target_repo.name,
                    )
                ),
                "operations_view": "mission-board",
                "events": [],
                "workstation_receipts": [],
            }
        try:
            data = json.loads(self._preferences_path.read_text(encoding="utf-8"))
            ConversationScope(**data["conversation_scope"])
            if not isinstance(data["revision"], int) or data["revision"] < 1:
                raise ValueError("revision must be a positive integer")
            if "events" not in data:
                data["events"] = []
            if not isinstance(data["events"], list):
                raise ValueError("events must be a list")
            if "workstation_receipts" not in data:
                data["workstation_receipts"] = []
            if not isinstance(data["workstation_receipts"], list) or any(
                not isinstance(receipt, dict)
                for receipt in data["workstation_receipts"]
            ):
                raise ValueError("Workstation action receipts must be a list of objects")
            return data
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(f"Workspace persistence read failed: {exc}") from exc

    def _qualify_scope(
        self, scope: ConversationScope, *, active_mission_id: str
    ) -> ConversationScope:
        if scope.kind == "working-directory":
            return ConversationScope(
                kind=scope.kind,
                target_id=scope.target_id,
                label=scope.label,
            )
        if scope.kind == "mission":
            owner = scope.mission_id or scope.target_id
            if owner not in self._missions or scope.target_id != owner:
                raise AlbertError(f"Unknown Mission Conversation Scope: {scope.target_id}")
            return ConversationScope(
                kind=scope.kind,
                target_id=scope.target_id,
                label=scope.label,
                mission_id=owner,
            )
        owner = scope.mission_id or active_mission_id
        mission = self._missions.get(owner)
        if mission is None or scope.target_id not in mission.issues:
            raise AlbertError(f"Unknown Issue Slice Conversation Scope: {scope.target_id}")
        return ConversationScope(
            kind=scope.kind,
            target_id=scope.target_id,
            label=scope.label,
            mission_id=owner,
        )

    def _canonical_session_launch_correlation(
        self,
        mission: AlbertMission,
        session: LocalAgentSession,
        *,
        preferences: dict[str, Any],
        queue: dict[str, Any] | None,
    ) -> str:
        queue_approval = session.task_packet.get("queue_approval")
        if isinstance(queue_approval, dict) and queue is not None:
            correlation_id = queue_approval.get("correlation_id")
            request = queue_approval.get("request")
            matches = [
                receipt
                for receipt in queue["receipts"]
                if isinstance(correlation_id, str)
                and receipt.get("correlation_id") == correlation_id
                and receipt.get("request_kind") == "workspace-queue-decision"
                and receipt.get("request") == request
                and isinstance(receipt.get("acknowledgement"), dict)
                and receipt["acknowledgement"].get("correlation_id") == correlation_id
                and receipt["acknowledgement"].get("outcome") == "acknowledged"
                and receipt["acknowledgement"].get("item_status") == "approved"
                and receipt["acknowledgement"].get("session_id") == session.session_id
            ]
            if len(matches) != 1:
                return ""
            item_id = matches[0]["acknowledgement"].get("item_id")
            items = [item for item in queue["items"] if item.item_id == item_id]
            if (
                len(items) != 1
                or items[0].mission_id != mission.mission_id
                or items[0].issue_id != session.issue_id
                or items[0].decision_correlation_id != correlation_id
            ):
                return ""
            return correlation_id.strip()

        workstation_action = session.task_packet.get("workstation_action")
        if not isinstance(workstation_action, dict):
            return ""
        correlation_id = workstation_action.get("correlation_id")
        marker_request = workstation_action.get("request")
        if not isinstance(correlation_id, str) or not isinstance(marker_request, dict):
            return ""
        expected_request = {
            "action_type": workstation_action.get("action_type"),
            "actor": "mission-commander",
            "mission_id": mission.mission_id,
            "target_kind": workstation_action.get("target_kind"),
            "target_id": workstation_action.get("target_id"),
            **marker_request,
        }
        matches = [
            receipt
            for receipt in preferences["workstation_receipts"]
            if receipt.get("correlation_id") == correlation_id
            and receipt.get("request") == expected_request
            and isinstance(receipt.get("acknowledgement"), dict)
            and receipt["acknowledgement"].get("correlation_id") == correlation_id
            and receipt["acknowledgement"].get("outcome") == "acknowledged"
            and receipt["acknowledgement"].get("action_type")
            == workstation_action.get("action_type")
            and receipt["acknowledgement"].get("session_id") == session.session_id
            and isinstance(receipt["acknowledgement"].get("revision"), int)
            and 1
            <= receipt["acknowledgement"]["revision"]
            <= preferences["revision"]
            and isinstance(receipt["acknowledgement"].get("effect_summary"), str)
            and bool(receipt["acknowledgement"]["effect_summary"].strip())
        ]
        return correlation_id.strip() if len(matches) == 1 else ""

    def _canonical_session_review_correlation(
        self,
        mission: AlbertMission,
        session: LocalAgentSession,
        review: ReviewDecision | None,
        *,
        journal_entries: tuple[ActivityJournalEntry, ...],
        workspace_events: tuple[WorkspaceEvent, ...],
    ) -> str:
        if review is None or not isinstance(review.workspace_action, dict):
            return ""
        correlation_id = review.workspace_action.get("correlation_id")
        request = review.workspace_action.get("request")
        if not isinstance(correlation_id, str) or not correlation_id.strip():
            return ""
        decision_outcomes = {
            "accept": {"Approved", "Approved with limitations"},
            "repair": {"Needs repair"},
            "escalate-human": {"Needs human review"},
        }
        if (
            not isinstance(request, dict)
            or set(request)
            != {"mission_id", "session_id", "decision", "reason", "failure_type"}
            or request.get("mission_id") != mission.mission_id
            or request.get("session_id") != session.session_id
            or review.issue_id != session.issue_id
            or review.outcome
            not in decision_outcomes.get(str(request.get("decision")), set())
        ):
            return ""
        matching_reviews = [
            candidate
            for candidate_mission in self._missions.values()
            for candidate in candidate_mission.reviews
            if candidate.workspace_action.get("correlation_id") == correlation_id
        ]
        matching_events = [
            event
            for event in workspace_events
            if event.correlation_id == correlation_id
        ]
        matching_journal = [
            entry
            for entry in journal_entries
            if entry.correlation_id == correlation_id
            and entry.action_type == "review-decision"
            and any(
                entity.entity_type == "local-agent-session"
                and entity.entity_id == session.session_id
                for entity in entry.affected_entities
            )
        ]
        if not (
            len(matching_reviews) == len(matching_events) == len(matching_journal) == 1
        ):
            return ""
        return correlation_id.strip()

    def _mission_summary(
        self,
        mission: AlbertMission,
        *,
        is_active: bool,
        preferences: dict[str, Any],
        queue: dict[str, Any] | None,
        journal_entries: tuple[ActivityJournalEntry, ...],
        workspace_events: tuple[WorkspaceEvent, ...],
    ) -> WorkspaceMissionSummary:
        def validated_parent_session_id(session: LocalAgentSession) -> str:
            repair_context = session.task_packet.get("repair_context")
            if not isinstance(repair_context, dict):
                return ""
            candidate = repair_context.get("prior_session_id")
            if not isinstance(candidate, str) or not candidate.strip():
                return ""
            current_id = session.session_id
            visited = {current_id}
            parent_id = candidate.strip()
            while parent_id:
                if parent_id in visited:
                    return ""
                visited.add(parent_id)
                parent = mission.sessions.get(parent_id)
                if parent is None or parent.issue_id != session.issue_id:
                    return ""
                parent_context = parent.task_packet.get("repair_context")
                next_parent = (
                    parent_context.get("prior_session_id")
                    if isinstance(parent_context, dict)
                    else ""
                )
                parent_id = next_parent.strip() if isinstance(next_parent, str) else ""
            return candidate.strip()

        def summarize_session(session: Any) -> MissionSessionSummary:
            canonical = mission._session_summary(session)
            evidence = session.evidence
            issue = mission.issues.get(session.issue_id)
            latest_review = mission._latest_review_for_session(session.session_id)
            launch_correlation_id = self._canonical_session_launch_correlation(
                mission,
                session,
                preferences=preferences,
                queue=queue,
            )
            review_correlation_id = self._canonical_session_review_correlation(
                mission,
                session,
                latest_review,
                journal_entries=journal_entries,
                workspace_events=workspace_events,
            )
            review_workspace_repair = WorkstationActionService._review_workspace_repair(
                mission,
                session.session_id,
            )
            repair_action_available = (
                review_workspace_repair is not None
                and WorkstationActionService._repair_child_for_session(
                    mission,
                    session.session_id,
                )
                is None
            )
            supervision_receipt = mission.supervision.get("receipts", {}).get(
                session.supervision_receipt_id
            )
            supervision_outcome = (
                str(supervision_receipt.get("outcome", ""))
                if isinstance(supervision_receipt, dict)
                else ""
            )
            return MissionSessionSummary(
                session_id=session.session_id,
                issue_id=session.issue_id,
                assigned_agent=session.assigned_agent,
                status=session.status,
                last_activity_at=_latest_session_activity_at(session),
                runner_started_at=_valid_session_activity_at(session.runner_started_at),
                role=str(
                    (session.task_packet.get("agent_config") or {}).get(
                        "role", "local-agent"
                    )
                ),
                provider=str(
                    (session.task_packet.get("agent_config") or {}).get(
                        "provider", "unconfigured"
                    )
                ),
                model=str(
                    (session.task_packet.get("agent_config") or {}).get(
                        "model", session.assigned_agent
                    )
                ),
                task_title=(
                    issue.title
                    if issue is not None
                    else str(session.task_packet.get("goal", session.issue_id))
                ),
                operation_status=str(canonical["operation_status"]),
                failure=str(canonical["failure"]),
                changed_files=tuple(evidence.changed_files) if evidence else (),
                commands_run=tuple(evidence.commands_run) if evidence else (),
                test_results=evidence.test_results if evidence else "",
                risks=evidence.known_risks if evidence else "",
                artifact_links=tuple(mission.review_artifact_links(session)),
                launch_correlation_id=launch_correlation_id,
                evidence_correlation_id=(
                    session.evidence_correlation_id
                    if session.evidence_valid
                    and evidence is not None
                    and session.evidence_correlation_id
                    == f"evidence:{mission.mission_id}:{session.session_id}"
                    else ""
                ),
                review_correlation_id=review_correlation_id,
                review_outcome=latest_review.outcome if latest_review else "",
                review_next_action=latest_review.next_action if latest_review else "",
                repair_action_available=repair_action_available,
                supervision_receipt_id=session.supervision_receipt_id,
                supervision_outcome=supervision_outcome,
                automatic_recovery_count=session.automatic_recovery_count,
                repair_task_packet=(
                    self._repair_task_packet_preview(
                        mission,
                        session,
                        review_workspace_repair,
                    )
                    if repair_action_available and review_workspace_repair is not None
                    else None
                ),
                work_kind=str(
                    session.task_packet.get(
                        "work_kind",
                        "issue-slice" if issue is not None else "ad-hoc-delegation",
                    )
                ),
                parent_session_id=validated_parent_session_id(session),
                session_revision=session.revision,
                retirement_phase=str(session.retirement.get("phase", "active")),
                retirement_blocked_reason=str(session.retirement.get("blocked_reason", "")),
                retirement_runner_boundary=dict(session.retirement.get("runner_boundary", {})),
                preservation_budget=dict(session.preservation_budget),
                retirement_record=(
                    dict(session.retirement["snapshot"]) if session.retirement.get("snapshot") else None
                ),
                retirement_actions=mission.inspect_retirement_unit(session.session_id)["actions"],
            )

        sessions = tuple(
            summarize_session(session)
            for session in sorted(mission.sessions.values(), key=lambda item: item.session_id)
        )
        attention: list[WorkspaceQueueAttention] = []
        for issue_id, decision in sorted(mission.delegations.items()):
            if decision.requires_approval and not decision.approved:
                attention.append(
                    WorkspaceQueueAttention(
                        attention_id=f"delegation-{mission.mission_id}-{issue_id}",
                        mission_id=mission.mission_id,
                        kind="delegation-approval",
                        label=f"{issue_id} delegation approval required",
                        queue_link=(
                            f"workspace-queue#delegation-{mission.mission_id}-{issue_id}"
                        ),
                        entity_id=issue_id,
                    )
                )
        for issue_id, issue in sorted(mission.issues.items()):
            if issue.review_state == "needs-human-review":
                attention.append(
                    WorkspaceQueueAttention(
                        attention_id=f"clarification-{mission.mission_id}-{issue_id}",
                        mission_id=mission.mission_id,
                        kind="clarification",
                        label=f"{issue_id} clarification required",
                        queue_link=(
                            f"workspace-queue#clarification-{mission.mission_id}-{issue_id}"
                        ),
                        entity_id=issue_id,
                    )
                )
        for raw_attention in mission.supervision.get("attentions", {}).values():
            if not isinstance(raw_attention, dict):
                raise WorkspacePersistenceError(
                    "Mission supervision attention projection is invalid."
                )
            if raw_attention.get("disposition") != "open" or raw_attention.get(
                "next_effect"
            ) != "mission-commander-decision":
                continue
            session_id = raw_attention.get("session_id")
            attention_id = raw_attention.get("attention_id")
            detail = raw_attention.get("detail")
            if not all(
                isinstance(value, str) and value.strip()
                for value in (session_id, attention_id, detail)
            ):
                raise WorkspacePersistenceError(
                    "Mission supervision attention identity is invalid."
                )
            attention.append(
                WorkspaceQueueAttention(
                    attention_id=attention_id,
                    mission_id=mission.mission_id,
                    kind="runner-supervision",
                    label=f"{session_id} supervision decision required: {detail}",
                    queue_link=f"mission-work#session-{session_id}",
                    entity_id=session_id,
                )
            )
        for item in WorkspaceQueueService(self).inspect(mission_id=mission.mission_id).items:
            if item.status != "pending":
                continue
            label = (
                f"{item.issue_id} Issue Change Proposal pending"
                if item.item_type == "issue-change-proposal"
                else (
                    f"{item.issue_id} Frontier Confirmation pending"
                    if item.item_type == "frontier-confirmation"
                    else f"{item.issue_id} Ad Hoc Delegation pending"
                )
            )
            attention.append(
                WorkspaceQueueAttention(
                    attention_id=item.item_id,
                    mission_id=mission.mission_id,
                    kind=item.item_type,
                    label=label,
                    queue_link=f"workspace-queue#{item.item_id}",
                    entity_id=item.issue_id,
                    queue_item_id=item.item_id,
                )
            )
        board = mission.board_summary()
        storage_attention = mission.retirement_storage.get("attention", {})
        if storage_attention.get("active") is True:
            attention.append(
                WorkspaceQueueAttention(
                    attention_id=(
                        f"retirement-storage-{mission.mission_id}-"
                        f"{storage_attention.get('code', 'attention')}"
                    ),
                    mission_id=mission.mission_id,
                    kind="retirement-storage",
                    label=str(
                        storage_attention.get(
                            "message",
                            "Snapshot storage requires Mission Commander attention.",
                        )
                    ),
                    queue_link="mission-work#retirement-storage",
                    entity_id=mission.mission_id,
                )
            )
        return WorkspaceMissionSummary(
            id=mission.mission_id,
            title=mission.prd_title,
            issue_count=board["issue_count"],
            is_active=is_active,
            sessions=sessions,
            attention=tuple(attention),
            archived_issue_ids=tuple(sorted(mission.archived_issue_ids)),
        )

    @staticmethod
    def _repair_task_packet_preview(
        mission: AlbertMission,
        session: LocalAgentSession,
        review: ReviewDecision,
    ) -> RepairTaskPacketPreview:
        issue = mission.issues.get(session.issue_id)
        packet = session.task_packet
        goal = issue.what_to_build if issue is not None else str(packet.get("goal", session.issue_id))
        acceptance_criteria = (
            issue.acceptance_criteria
            if issue is not None
            else [
                item
                for item in packet.get("acceptance_criteria", [])
                if isinstance(item, str)
            ]
        )
        evidence_requirements = (
            issue.evidence_requirements or mission.default_evidence_requirements()
            if issue is not None
            else [
                item
                for item in packet.get("evidence_requirements", mission.default_evidence_requirements())
                if isinstance(item, str)
            ]
        )
        return RepairTaskPacketPreview(
            issue_id=session.issue_id,
            goal=goal,
            acceptance_criteria=tuple(acceptance_criteria),
            allowed_paths=tuple(
                item for item in packet.get("allowed_paths", []) if isinstance(item, str)
            ),
            command_policy={
                command: policy
                for command, policy in packet.get("command_policy", {}).items()
                if isinstance(command, str) and isinstance(policy, str)
            }
            if isinstance(packet.get("command_policy", {}), dict)
            else {},
            evidence_requirements=tuple(evidence_requirements),
            assigned_agent=session.assigned_agent,
            review_reason=review.reason,
        )

    @staticmethod
    @contextmanager
    def _json_store_lock(path: Path):
        path.parent.mkdir(parents=True, exist_ok=True)
        lock_path = path.with_name(f".{path.name}.lock")
        with lock_path.open("a+", encoding="utf-8") as lock_file:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)

    @contextmanager
    def _action_store_lock(self, store_path: Path):
        with self._json_store_lock(self._action_lock_target):
            with self._json_store_lock(store_path):
                yield

    @staticmethod
    def _write_json_atomically(path: Path, data: dict[str, Any]) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary_path: Path | None = None
        try:
            with tempfile.NamedTemporaryFile(
                mode="w",
                encoding="utf-8",
                dir=path.parent,
                prefix=f".{path.name}.",
                suffix=".tmp",
                delete=False,
            ) as temporary:
                temporary.write(json.dumps(data, indent=2, sort_keys=True) + "\n")
                temporary.flush()
                os.fsync(temporary.fileno())
                temporary_path = Path(temporary.name)
            temporary_path.replace(path)
            if os.name == "posix":
                directory_fd = os.open(path.parent, os.O_RDONLY)
                try:
                    os.fsync(directory_fd)
                finally:
                    os.close(directory_fd)
        finally:
            if temporary_path is not None and temporary_path.exists():
                temporary_path.unlink()
