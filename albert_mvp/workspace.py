from __future__ import annotations

from dataclasses import asdict, dataclass, replace
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import shlex
import subprocess
from typing import Any, Literal

from .core import AlbertError, AlbertMission, EvidenceValidationError, IssueSlice, LocalAgentSession


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
class MissionSessionSummary:
    session_id: str
    issue_id: str
    assigned_agent: str
    status: str
    role: str
    provider: str
    model: str


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
    ]
    label: str
    queue_link: str


@dataclass(frozen=True)
class WorkspaceMissionSummary:
    id: str
    title: str
    issue_count: int
    is_active: bool
    sessions: tuple[MissionSessionSummary, ...]
    attention: tuple[WorkspaceQueueAttention, ...]


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
    status: Literal["pending-approval", "completed", "failed", "denied"]
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


@dataclass(frozen=True)
class ShellTerminalCommandRecord:
    command_id: str
    correlation_id: str
    command: str
    classification: Literal["auto-allowed", "frontier-approvable", "human-required"]
    status: Literal["pending-approval", "completed", "failed", "denied"]
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


class ShellTerminalService:
    """Executes governed commands while keeping terminal bytes transient."""

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots
        self._terminal_path = snapshots.preferences_path.parent / "shell-terminal.json"

    @property
    def terminal_path(self) -> Path:
        return self._terminal_path

    def inspect(self) -> ShellTerminalProjection:
        terminal = self._load_terminal()
        return ShellTerminalProjection(
            schema_version=1,
            revision=terminal["revision"],
            commands=tuple(
                ShellTerminalCommandRecord(
                    command_id=item["command_id"],
                    correlation_id=item["correlation_id"],
                    command=item["command"],
                    classification=item["classification"],
                    status=item["status"],
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
        )

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
        snapshot = self._snapshots.snapshot()
        if snapshot.active_mission is None:
            raise AlbertError("Shell Terminal requires an Active Mission")
        mission = self._snapshots._missions[snapshot.active_mission.id]
        terminal = self._load_terminal()
        working_path = Path(working_directory).resolve()
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
            raise AlbertError(
                "Shell Terminal working directory is outside the workspace and has no "
                f"active {access_level} Additional Path Grant."
            )
        outside_paths = [
            str(Path(path).resolve())
            for path in requested_paths
            if not self._path_authorized(
                Path(path).resolve(),
                access_level=access_level,
                workspace=mission.target_repo,
                grants=terminal["grants"],
            )
        ]
        if outside_paths:
            raise AlbertError(
                "Shell Terminal requested path is outside the workspace and has no "
                f"active {access_level} Additional Path Grant: {outside_paths[0]}"
            )
        classification = mission.classify_command(command)
        command_id = f"terminal-command-{len(terminal['commands']) + 1:06d}"
        record = {
            "command_id": command_id,
            "correlation_id": correlation_id,
            "command": command,
            "classification": classification,
            "status": "pending-approval",
            "exit_code": None,
            "working_directory": str(working_path),
            "requested_paths": [str(Path(path).resolve()) for path in requested_paths],
            "access_level": access_level,
            "requester": requester,
        }
        if classification != "auto-allowed":
            self._persist_terminal(
                revision=terminal["revision"] + 1,
                commands=[*terminal["commands"], record],
                grants=terminal["grants"],
            )
            return ShellTerminalCommandResult(
                command_id=command_id,
                correlation_id=correlation_id,
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

    def create_path_grant(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        path: str,
        access_level: Literal["read", "write"],
        duration_seconds: int,
        requester: str,
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
        terminal = self._load_terminal()
        if expected_revision != terminal["revision"]:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=terminal["revision"],
            )
        resolved_path = Path(path).resolve()
        now = datetime.now(timezone.utc)
        granted_at = now.isoformat(timespec="seconds").replace("+00:00", "Z")
        expires_at = (now + timedelta(seconds=duration_seconds)).isoformat(
            timespec="seconds"
        ).replace("+00:00", "Z")
        grant = AdditionalPathGrant(
            grant_id=f"path-grant-{len(terminal['grants']) + 1:06d}",
            correlation_id=correlation_id,
            path=str(resolved_path),
            access_level=access_level,
            duration_seconds=duration_seconds,
            granted_by="mission-commander",
            granted_at=granted_at,
            expires_at=expires_at,
        )
        self._persist_terminal(
            revision=terminal["revision"] + 1,
            commands=terminal["commands"],
            grants=[*terminal["grants"], asdict(grant)],
        )
        return grant

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
        terminal = self._load_terminal()
        commands = list(terminal["commands"])
        index = next(
            (position for position, item in enumerate(commands) if item["command_id"] == command_id),
            None,
        )
        if index is None:
            raise AlbertError(f"Unknown Shell Terminal command: {command_id}")
        record = commands[index]
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
        terminal = self._load_terminal()
        commands = list(terminal["commands"])
        index = next(
            (position for position, item in enumerate(commands) if item["command_id"] == command_id),
            None,
        )
        if index is None:
            raise AlbertError(f"Unknown Shell Terminal command: {command_id}")
        record = commands[index]
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
        )
        return ShellTerminalCommandResult(
            command_id=record["command_id"],
            correlation_id=record["correlation_id"],
            classification=record["classification"],
            status="denied",
            exit_code=None,
            stdout="",
            stderr="",
        )

    def _execute(
        self,
        *,
        terminal: dict[str, Any],
        record: dict[str, Any],
        record_index: int | None,
    ) -> ShellTerminalCommandResult:
        completed = subprocess.run(
            shlex.split(record["command"]),
            cwd=record["working_directory"],
            capture_output=True,
            text=True,
            check=False,
        )
        status: Literal["completed", "failed"] = (
            "completed" if completed.returncode == 0 else "failed"
        )
        completed_record = {
            **record,
            "status": status,
            "exit_code": completed.returncode,
        }
        commands = list(terminal["commands"])
        if record_index is None:
            commands.append(completed_record)
        else:
            commands[record_index] = completed_record
        self._persist_terminal(
            revision=terminal["revision"] + 1,
            commands=commands,
            grants=terminal["grants"],
        )
        return ShellTerminalCommandResult(
            command_id=record["command_id"],
            correlation_id=record["correlation_id"],
            classification=record["classification"],
            status=status,
            exit_code=completed.returncode,
            stdout=completed.stdout,
            stderr=completed.stderr,
        )

    def _persist_terminal(
        self,
        *,
        revision: int,
        commands: list[dict[str, Any]],
        grants: list[dict[str, Any]],
    ) -> None:
        WorkspaceSnapshotService._write_json_atomically(
            self._terminal_path,
            {
                "schema_version": 1,
                "revision": revision,
                "commands": commands,
                "grants": grants,
            },
        )

    def _load_terminal(self) -> dict[str, Any]:
        if not self._terminal_path.exists():
            return {"schema_version": 1, "revision": 0, "commands": [], "grants": []}
        try:
            payload = json.loads(self._terminal_path.read_text(encoding="utf-8"))
            if payload["schema_version"] != 1:
                raise ValueError("unsupported Shell Terminal schema")
            if not isinstance(payload["revision"], int) or payload["revision"] < 0:
                raise ValueError("Shell Terminal revision must be non-negative")
            if not isinstance(payload["commands"], list):
                raise ValueError("Shell Terminal commands must be a list")
            if not isinstance(payload.get("grants", []), list):
                raise ValueError("Additional Path Grants must be a list")
            payload["grants"] = payload.get("grants", [])
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


@dataclass(frozen=True)
class AgentConsoleMessage:
    message_id: str
    sequence: int
    role: AgentConsoleRole
    content: str
    scope: ConversationScope
    outcome: AgentConsoleOutcome
    source: str


class AgentConsoleHistoryService:
    """Persists the continuous, scoped Agent Console record for a Workspace Session."""

    _roles = {"user", "assistant", "system"}
    _outcomes = {"proposed", "pending", "acknowledged", "rejected", "model-commentary"}

    def __init__(self, snapshots: WorkspaceSnapshotService):
        self._snapshots = snapshots
        self._history_path = snapshots.preferences_path.parent / "agent-console-history.json"

    @property
    def history_path(self) -> Path:
        return self._history_path

    def append(
        self,
        *,
        role: AgentConsoleRole,
        content: str,
        outcome: AgentConsoleOutcome,
        source: str,
        expected_revision: int | None = None,
        expected_scope: ConversationScope | None = None,
    ) -> AgentConsoleMessage:
        if role not in self._roles:
            raise AlbertError(f"Unknown Agent Console role: {role}")
        if outcome not in self._outcomes:
            raise AlbertError(f"Unknown Agent Console outcome: {outcome}")
        if not content.strip():
            raise AlbertError("Agent Console message content must not be empty")
        if not source.strip():
            raise AlbertError("Agent Console message source must not be empty")
        snapshot = self._snapshots.snapshot()
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
        messages = list(self.history())
        sequence = len(messages) + 1
        message = AgentConsoleMessage(
            message_id=f"console-{sequence:06d}",
            sequence=sequence,
            role=role,
            content=content,
            scope=snapshot.conversation_scope,
            outcome=outcome,
            source=source,
        )
        messages.append(message)
        WorkspaceSnapshotService._write_json_atomically(
            self._history_path,
            {"schema_version": 1, "messages": [asdict(item) for item in messages]},
        )
        return message

    def history(self) -> tuple[AgentConsoleMessage, ...]:
        if not self._history_path.exists():
            return ()
        try:
            payload = json.loads(self._history_path.read_text(encoding="utf-8"))
            if payload["schema_version"] != 1 or not isinstance(payload["messages"], list):
                raise ValueError("unsupported Agent Console history schema")
            messages = tuple(self._parse_message(item) for item in payload["messages"])
            if [item.sequence for item in messages] != list(range(1, len(messages) + 1)):
                raise ValueError("Agent Console message sequence must be contiguous")
            if [item.message_id for item in messages] != [
                f"console-{sequence:06d}" for sequence in range(1, len(messages) + 1)
            ]:
                raise ValueError("Agent Console message ids must match sequence")
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
        if not isinstance(item["source"], str) or not item["source"].strip():
            raise ValueError("Agent Console message source must not be empty")
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
        )


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


WorkstationActionType = Literal[
    "issue-launch",
    "issue-retry",
    "session-cancel",
    "model-assignment-change",
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
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        if not correlation_id.strip():
            raise AlbertError("Workspace Queue correlation id must not be empty")
        if not source.strip():
            raise AlbertError("Workspace Queue item source must not be empty")
        mission = self._mission_for_queue_action(snapshot, mission_id)
        if issue_id not in mission.issues:
            raise AlbertError(f"Unknown Issue Slice for Workspace Queue: {issue_id}")
        issue = mission.issues[issue_id]
        if not issue.locked:
            raise AlbertError(f"{issue_id} is not locked; edit the accepted contract directly.")
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

        queue = self._load_queue()
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
        )
        items = [*queue["items"], item]
        revision = queue["revision"] + 1
        WorkspaceSnapshotService._write_json_atomically(
            self._queue_path,
            {
                "schema_version": 1,
                "revision": revision,
                "items": [asdict(queue_item) for queue_item in items],
            },
        )
        return WorkspaceQueueAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            item_id=item_id,
            item_status="pending",
            effect_summary=(
                f"{issue_id} accepted contract is unchanged; proposal {item_id} is pending."
            ),
        )

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
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
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
        mission = self._mission_for_queue_action(snapshot, mission_id)
        if issue_id not in mission.issues:
            raise AlbertError(f"Unknown Issue Slice for Workspace Queue: {issue_id}")

        queue = self._load_queue()
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
        )
        revision = queue["revision"] + 1
        WorkspaceSnapshotService._write_json_atomically(
            self._queue_path,
            {
                "schema_version": 1,
                "revision": revision,
                "items": [
                    asdict(queue_item) for queue_item in [*queue["items"], item]
                ],
            },
        )
        acknowledgement = WorkspaceQueueAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            item_id=item_id,
            item_status="pending",
            effect_summary=f"Frontier Confirmation {item_id} is pending.",
        )
        ActivityJournalService(self._snapshots).record_frontier_confirmation_requested(
            correlation_id=correlation_id,
            item=item,
        )
        return acknowledgement

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
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
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
        mission = self._mission_for_queue_action(snapshot, mission_id)
        scope = self._snapshots._qualify_scope(scope, active_mission_id=mission.mission_id)

        queue = self._load_queue()
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
            },
        )
        revision = queue["revision"] + 1
        WorkspaceSnapshotService._write_json_atomically(
            self._queue_path,
            {
                "schema_version": 1,
                "revision": revision,
                "items": [asdict(queue_item) for queue_item in [*queue["items"], item]],
            },
        )
        return WorkspaceQueueAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            item_id=item_id,
            item_status="pending",
            effect_summary=f"Ad Hoc Delegation {work_id} is pending approval.",
        )

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
        if expected_revision != queue["revision"]:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=queue["revision"],
            )
        items = list(queue["items"])
        index = next(
            (position for position, item in enumerate(items) if item.item_id == item_id),
            None,
        )
        if index is None:
            raise AlbertError(f"Unknown Workspace Queue item: {item_id}")
        item = items[index]
        if item.status != "pending":
            raise AlbertError(f"Workspace Queue item is already {item.status}: {item_id}")

        item_status: WorkspaceQueueItemStatus
        launched_session: LocalAgentSession | None = None
        if decision == "approve":
            if item.item_type == "issue-change-proposal":
                self._apply_issue_change_proposal(item, reason=reason)
                effect_summary = (
                    f"Applied {item_id}; {item.issue_id} is reopened for re-review."
                )
            elif item.item_type == "ad-hoc-delegation":
                launched_session = self._launch_ad_hoc_delegation(item, reason=reason)
                effect_summary = (
                    f"Approved {item_id}; launched {item.issue_id} as "
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

        items[index] = replace(item, status=item_status)
        revision = queue["revision"] + 1
        WorkspaceSnapshotService._write_json_atomically(
            self._queue_path,
            {
                "schema_version": 1,
                "revision": revision,
                "items": [asdict(queue_item) for queue_item in items],
            },
        )
        acknowledgement = WorkspaceQueueAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            item_id=item_id,
            item_status=item_status,
            effect_summary=effect_summary,
        )
        ActivityJournalService(self._snapshots).record_workspace_queue_decision(
            correlation_id=correlation_id,
            item=item,
            item_status=item_status,
            effect_summary=effect_summary,
        )
        if launched_session is not None:
            ActivityJournalService(self._snapshots).record_orchestrator_session_launched(
                correlation_id=correlation_id,
                item=item,
                session=launched_session,
            )
        return acknowledgement

    def _launch_ad_hoc_delegation(
        self, item: WorkspaceQueueItem, *, reason: str
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
        command_policy = dict(proposed["command_policy"])
        denied_commands = [
            command
            for command, policy in command_policy.items()
            if policy != "auto-allowed" or mission.classify_command(command) != "auto-allowed"
        ]
        if denied_commands:
            raise AlbertError(
                f"Ad Hoc Delegation {item.issue_id} requires auto-allowed command policy "
                f"before launch: {', '.join(denied_commands)}"
            )

        session_id = f"session-{item.issue_id}-{len(mission.sessions) + 1}"
        worktree_path = (
            mission.target_repo.parent
            / ".albert-worktrees"
            / mission.target_repo.name
            / item.issue_id
        )
        worktree_path.mkdir(parents=True, exist_ok=True)
        assigned_agent = str(proposed["proposed_agent"])
        session = LocalAgentSession(
            session_id=session_id,
            issue_id=item.issue_id,
            assigned_agent=assigned_agent,
            worktree_path=worktree_path,
            task_packet={
                "issue_id": item.issue_id,
                "work_kind": "ad-hoc-delegation",
                "goal": f"Ad Hoc Delegation {item.issue_id}",
                "conversation_scope": dict(proposed["scope"]),
                "acceptance_criteria": list(proposed["acceptance_criteria"]),
                "allowed_paths": list(proposed["allowed_paths"]),
                "command_policy": command_policy,
                "assigned_agent": assigned_agent,
                "agent_config": mission._agent_config_for(assigned_agent),
                "originating_message_id": str(proposed["originating_message_id"]),
            },
        )
        mission.sessions[session_id] = session
        audit_reason = reason.strip() or "Workspace Queue ad hoc delegation approved."
        mission._record(f"{item.issue_id} launched as {session_id}: {audit_reason}")
        mission._persist()
        return session

    def _apply_issue_change_proposal(self, item: WorkspaceQueueItem, *, reason: str) -> None:
        if item.item_type != "issue-change-proposal":
            raise AlbertError(f"Workspace Queue item is not an Issue Change Proposal: {item.item_id}")
        if item.mission_id not in self._snapshots._missions:
            raise AlbertError(f"Unknown Mission for Workspace Queue item: {item.mission_id}")
        mission = self._snapshots._missions[item.mission_id]
        if item.issue_id not in mission.issues:
            raise AlbertError(f"Unknown Issue Slice for Workspace Queue item: {item.issue_id}")
        issue = mission.issues[item.issue_id]
        issue.locked = False
        for field_name, value in item.proposed_changes.items():
            if field_name in {"acceptance_criteria", "blocked_by", "evidence_requirements"}:
                setattr(issue, field_name, list(value))
            elif field_name in {"what_to_build", "type", "risk"}:
                setattr(issue, field_name, value)
            else:
                raise AlbertError(f"Unknown governed field in Issue Change Proposal: {field_name}")
        issue.contract_overridden = True
        issue.review_state = "needs-review"
        issue.status = "needs-review"
        audit_reason = reason.strip() or "Workspace Queue proposal approved."
        mission._record(f"{item.issue_id} changed through Workspace Queue: {audit_reason}")
        mission._persist()

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
            return {"schema_version": 1, "revision": 1, "items": []}
        try:
            data = json.loads(self._queue_path.read_text(encoding="utf-8"))
            if data["schema_version"] != 1:
                raise ValueError("unsupported Workspace Queue schema")
            if not isinstance(data["revision"], int) or data["revision"] < 1:
                raise ValueError("Workspace Queue revision must be positive")
            if not isinstance(data["items"], list):
                raise ValueError("Workspace Queue items must be a list")
            return {
                "schema_version": 1,
                "revision": data["revision"],
                "items": [self._parse_item(item) for item in data["items"]],
            }
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
            raise WorkspacePersistenceError(
                f"Workspace Queue persistence read failed: {exc}"
            ) from exc

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

    def inspect(self, *, mission_id: str | None = None) -> MissionDraftProjection:
        if mission_id is not None and mission_id not in self._snapshots._missions:
            raise AlbertError(f"Unknown Mission for Mission Draft filter: {mission_id}")
        drafts = self._load_drafts()
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
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        if not correlation_id.strip():
            raise AlbertError("Mission Draft correlation id must not be empty")
        mission = self._mission_for_draft_action(snapshot, mission_id)
        drafts = self._load_drafts()
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
        WorkspaceSnapshotService._write_json_atomically(
            self._drafts_path,
            {
                "schema_version": 1,
                "revision": revision,
                "drafts": [asdict(item) for item in [*drafts["drafts"], draft]],
            },
        )
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
        ActivityJournalService(self._snapshots).record_mission_draft_created(
            correlation_id=correlation_id,
            draft=draft,
            effect_summary=acknowledgement.effect_summary,
        )
        return acknowledgement

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
        WorkspaceSnapshotService._write_json_atomically(
            self._drafts_path,
            {
                "schema_version": 1,
                "revision": revision,
                "drafts": [asdict(item) for item in items],
            },
        )
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
        ActivityJournalService(self._snapshots).record_mission_draft_updated(
            correlation_id=correlation_id,
            draft=updated,
            effect_summary=acknowledgement.effect_summary,
        )
        return acknowledgement

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
        issue = self._create_confirmed_issue_slice(mission, draft)
        items[index] = replace(draft, status="confirmed")
        revision = drafts["revision"] + 1
        WorkspaceSnapshotService._write_json_atomically(
            self._drafts_path,
            {
                "schema_version": 1,
                "revision": revision,
                "drafts": [asdict(item) for item in items],
            },
        )
        mission._record(f"{issue.id} created from Mission Draft {draft_id}: {reason.strip()}")
        mission._persist()
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
        ActivityJournalService(self._snapshots).record_mission_draft_confirmed(
            correlation_id=correlation_id,
            draft=draft,
            issue=issue,
            effect_summary=acknowledgement.effect_summary,
        )
        return acknowledgement

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
        WorkspaceSnapshotService._write_json_atomically(
            self._drafts_path,
            {
                "schema_version": 1,
                "revision": revision,
                "drafts": [asdict(item) for item in items],
            },
        )
        acknowledgement = MissionDraftAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=revision,
            draft_id=draft_id,
            draft_status="abandoned",
            effect_summary=(
                f"Mission Draft {draft_id} abandoned; accepted Mission state is unchanged."
            ),
        )
        ActivityJournalService(self._snapshots).record_mission_draft_abandoned(
            correlation_id=correlation_id,
            draft=abandoned,
            effect_summary=acknowledgement.effect_summary,
        )
        return acknowledgement

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

    def _create_confirmed_issue_slice(
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
        acceptance = self._confirmed_acceptance_criteria(draft)
        source_path.parent.mkdir(parents=True, exist_ok=True)
        source_path.write_text(
            "\n".join(
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
            ),
            encoding="utf-8",
        )
        issue = IssueSlice(
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
        mission.issues[issue_id] = issue
        return issue

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
            return {"schema_version": 1, "revision": 1, "drafts": []}
        try:
            data = json.loads(self._drafts_path.read_text(encoding="utf-8"))
            if data["schema_version"] != 1:
                raise ValueError("unsupported Mission Draft schema")
            if not isinstance(data["revision"], int) or data["revision"] < 1:
                raise ValueError("Mission Draft revision must be positive")
            if not isinstance(data["drafts"], list):
                raise ValueError("Mission Draft drafts must be a list")
            return {
                "schema_version": 1,
                "revision": data["revision"],
                "drafts": [self._parse_draft(item) for item in data["drafts"]],
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
            if session.status not in {"reviewed", "complete"}
        )
        return ReviewWorkspaceProjection(
            schema_version=1,
            revision=snapshot.revision,
            mission_id=mission.mission_id,
            items=items,
        )

    def decide(
        self,
        *,
        correlation_id: str,
        expected_revision: int,
        session_id: str,
        decision: Literal["accept", "repair", "escalate-human"],
        reason: str = "",
        failure_type: str = "",
    ) -> ReviewWorkspaceDecisionAcknowledgement:
        snapshot = self._snapshots.snapshot()
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        if not correlation_id.strip():
            raise AlbertError("Review decision correlation id must not be empty")
        if snapshot.active_mission is None:
            raise AlbertError("Review Workspace requires an active Mission")
        mission = self._snapshots._missions[snapshot.active_mission.id]
        if session_id not in mission.sessions:
            raise AlbertError(f"Unknown Review Workspace session: {session_id}")
        session = mission.sessions[session_id]
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
            failure_type=failure_type,
        )
        issue = mission.issues.get(review.issue_id)
        updated = self._snapshots.update_preferences(
            active_mission_id=mission.mission_id,
            conversation_scope=snapshot.conversation_scope,
            operations_view=snapshot.operations_view,
            event_metadata={"correlation_id": correlation_id},
        )
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
            session_id=session_id,
            review_outcome=review.outcome,
            next_action=review.next_action,
            evidence_links=tuple(session.evidence.artifact_links) if session.evidence else (),
        )
        return ReviewWorkspaceDecisionAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=updated.revision,
            issue_id=review.issue_id,
            session_id=session_id,
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
                artifact_links=list(evidence.artifact_links) if evidence else [],
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
        "issue-launch",
        "issue-retry",
        "session-cancel",
        "model-assignment-change",
    }
    _actors = {"mission-commander"}

    def __init__(self, snapshots: "WorkspaceSnapshotService"):
        self._snapshots = snapshots

    def submit(
        self,
        *,
        correlation_id: str,
        action_type: WorkstationActionType | str,
        actor: str,
        expected_revision: int,
        target_kind: str,
        target_id: str,
        issue_id: str = "",
        session_id: str = "",
        agent_id: str = "",
        reason: str = "",
        allowed_paths: list[str] | None = None,
        command_policy: dict[str, str] | None = None,
    ) -> WorkstationActionAcknowledgement:
        snapshot = self._snapshots.snapshot()
        if expected_revision != snapshot.revision:
            raise WorkspaceStaleActionError(
                expected_revision=expected_revision,
                current_revision=snapshot.revision,
            )
        if not correlation_id.strip():
            raise AlbertError("Workstation action correlation id must not be empty")
        if actor not in self._actors:
            raise AlbertError(f"Unknown Workstation action actor: {actor}")
        if action_type not in self._actions:
            raise AlbertError(f"Unknown Workstation action type: {action_type}")
        if snapshot.active_mission is None:
            raise AlbertError("Workstation actions require an active Mission")
        mission = self._snapshots._missions[snapshot.active_mission.id]

        acknowledged_issue_id = issue_id
        acknowledged_session_id = session_id
        journal_actor: ActivityActor = "mission-commander"
        if action_type == "issue-launch":
            self._validate_issue_target(
                target_kind=target_kind,
                target_id=target_id,
                issue_id=issue_id,
            )
            session = mission.launch_issue(
                issue_id,
                allowed_paths=allowed_paths or [],
                command_policy=command_policy or {},
            )
            acknowledged_session_id = session.session_id
            effect_summary = (
                f"Orchestrator launched {issue_id} as {session.session_id}."
            )
            journal_actor = "orchestrator"
        elif action_type == "issue-retry":
            self._validate_session_target(
                target_kind=target_kind,
                target_id=target_id,
                session_id=session_id,
            )
            if not reason.strip():
                raise AlbertError("Retry requires a reason.")
            prior_session = mission.sessions.get(session_id)
            if prior_session is None:
                raise AlbertError(f"Unknown Workstation session: {session_id}")
            acknowledged_issue_id = issue_id or prior_session.issue_id
            if acknowledged_issue_id != prior_session.issue_id:
                raise AlbertError("issue id must match session issue id")
            session = mission.launch_repair(
                session_id,
                agent_id=agent_id,
                allowed_paths=allowed_paths or [],
                command_policy=command_policy or {},
            )
            acknowledged_session_id = session.session_id
            effect_summary = (
                f"Orchestrator retried {acknowledged_issue_id} as "
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
            cancelled = mission.cancel_session(session_id, reason=reason)
            acknowledged_session_id = cancelled.session_id
            effect_summary = (
                f"Orchestrator cancelled {cancelled.session_id} for "
                f"{acknowledged_issue_id}. Persisted session state is cancelled; "
                "runner process termination is not available in this MVP."
            )
            journal_actor = "orchestrator"
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
            mission.assign_issue(issue_id, agent_id, notes=reason)
            effect_summary = (
                f"Mission Commander assigned {issue_id} to {agent_id}: {reason}"
            )

        updated = self._snapshots.update_preferences(
            active_mission_id=mission.mission_id,
            conversation_scope=snapshot.conversation_scope,
            operations_view=snapshot.operations_view,
            event_metadata={"correlation_id": correlation_id},
        )
        ActivityJournalService(self._snapshots).record_workstation_action(
            correlation_id=correlation_id,
            actor=journal_actor,
            action_type=action_type,
            mission=mission,
            issue_id=acknowledged_issue_id,
            session_id=acknowledged_session_id,
            effect_summary=effect_summary,
        )
        return WorkstationActionAcknowledgement(
            correlation_id=correlation_id,
            outcome="acknowledged",
            revision=updated.revision,
            action_type=action_type,  # type: ignore[arg-type]
            issue_id=acknowledged_issue_id,
            session_id=acknowledged_session_id,
            effect_summary=effect_summary,
        )

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
                f"Orchestrator launched {session.session_id} for {item.issue_id} "
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
        evidence_links = tuple(evidence.artifact_links)
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
            correlation_id=f"evidence:{session.session_id}",
        )

    def _append(
        self,
        *,
        actor: ActivityActor,
        action_type: str,
        summary: str,
        affected_entities: tuple[ActivityAffectedEntity, ...],
        evidence_links: tuple[str, ...],
        correlation_id: str,
    ) -> ActivityJournalEntry:
        if actor not in self._actors:
            raise AlbertError(f"Unknown Activity Journal actor: {actor}")
        if not action_type.strip():
            raise AlbertError("Activity Journal action type must not be empty")
        if not summary.strip():
            raise AlbertError("Activity Journal summary must not be empty")
        if any(not isinstance(link, str) or not link.strip() for link in evidence_links):
            raise AlbertError("Activity Journal evidence links must not be empty")
        journal = self._load_journal()
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
        try:
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

    def submit_action(self, action: WorkspaceAction) -> WorkspaceActionAcknowledgement:
        current = self._snapshots.snapshot()
        if action.expected_revision != current.revision:
            raise WorkspaceStaleActionError(
                expected_revision=action.expected_revision,
                current_revision=current.revision,
            )
        updated = self._snapshots.update_preferences(
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
        evidence_activity_recorder = ActivityJournalService(self).record_local_agent_evidence
        for item in all_missions:
            item._evidence_activity_recorder = evidence_activity_recorder

    @property
    def preferences_path(self) -> Path:
        return self._preferences_path

    def snapshot(self) -> WorkspaceSnapshot:
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
                self._mission_summary(item, is_active=item.mission_id == active.mission_id)
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
        data = {
            "revision": revision,
            "active_mission_id": active_mission_id,
            "conversation_scope": asdict(conversation_scope),
            "operations_view": operations_view,
            "events": events,
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

    def _mission_summary(
        self, mission: AlbertMission, *, is_active: bool
    ) -> WorkspaceMissionSummary:
        sessions = tuple(
            MissionSessionSummary(
                session_id=session.session_id,
                issue_id=session.issue_id,
                assigned_agent=session.assigned_agent,
                status=session.status,
                role=str((session.task_packet.get("agent_config") or {}).get("role", "local-agent")),
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
            )
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
                )
            )
        board = mission.board_summary()
        return WorkspaceMissionSummary(
            id=mission.mission_id,
            title=mission.prd_title,
            issue_count=board["issue_count"],
            is_active=is_active,
            sessions=sessions,
            attention=tuple(attention),
        )

    @staticmethod
    def _write_json_atomically(path: Path, data: dict[str, Any]) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_suffix(path.suffix + ".tmp")
        temporary.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        temporary.replace(path)
