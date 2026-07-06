from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from hashlib import sha1
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import tempfile
from typing import Any, Callable

from .agents import AgentConfig, AgentRegistry, load_agent_registry


class AlbertError(Exception):
    """Base error for user-actionable MVP failures."""


class LockedFieldError(AlbertError):
    """Raised when an approved Issue Slice contract is edited while locked."""


class LaunchBlockedError(AlbertError):
    """Raised when an Issue Slice cannot be launched."""


class EvidenceValidationError(AlbertError):
    """Raised when an Evidence Package is incomplete."""


def _command_invocation(command: str) -> str | list[str]:
    """Preserve native Windows command-line parsing and POSIX argv parsing."""
    return command if os.name == "nt" else shlex.split(command)


@dataclass
class IssueSlice:
    id: str
    slug: str
    title: str
    status: str
    tracker_status: str
    type: str
    risk: str
    suggested_agent: str
    assigned_agent: str
    what_to_build: str
    acceptance_criteria: list[str]
    blocked_by: list[str]
    source_path: str
    evidence_requirements: list[str] = field(default_factory=list)
    review_state: str = "needs-review"
    locked: bool = False
    notes: str = ""
    launch_order: int | None = None
    contract_overridden: bool = False

    def to_runtime(self) -> dict[str, Any]:
        data = {
            "assigned_agent": self.assigned_agent,
            "locked": self.locked,
            "notes": self.notes,
            "review_state": self.review_state,
            "status": self.status,
            "launch_order": self.launch_order,
            "contract_overridden": self.contract_overridden,
        }
        if self.contract_overridden:
            data.update(
                {
                    "acceptance_criteria": self.acceptance_criteria,
                    "blocked_by": self.blocked_by,
                    "evidence_requirements": self.evidence_requirements,
                    "type": self.type,
                    "risk": self.risk,
                    "what_to_build": self.what_to_build,
                }
            )
        return data

    def apply_runtime(self, data: dict[str, Any]) -> None:
        tracker_complete = self.tracker_status.lower() in {"complete", "completed"}
        for field_name in [
            "assigned_agent",
            "notes",
            "launch_order",
            "contract_overridden",
        ]:
            if field_name in data:
                setattr(self, field_name, data[field_name])
        if not tracker_complete:
            for field_name in ["locked", "review_state", "status"]:
                if field_name in data:
                    setattr(self, field_name, data[field_name])
        else:
            self.status = "complete"
            self.review_state = "complete"
            self.locked = True
        if self.contract_overridden:
            for field_name in [
                "acceptance_criteria",
                "blocked_by",
                "evidence_requirements",
                "type",
                "risk",
                "what_to_build",
            ]:
                if field_name in data:
                    setattr(self, field_name, data[field_name])


@dataclass
class EvidencePackage:
    changed_files: list[str] = field(default_factory=list)
    diff_summary: str = ""
    commands_run: list[str] = field(default_factory=list)
    test_results: str = ""
    known_risks: str = ""
    proposed_context_updates: str = ""
    artifact_links: list[str] = field(default_factory=list)

    def missing_fields(self) -> list[str]:
        missing: list[str] = []
        if not self.changed_files:
            missing.append("changed_files")
        if not self.diff_summary.strip():
            missing.append("diff_summary")
        if not self.commands_run:
            missing.append("commands_run")
        if not self.test_results.strip():
            missing.append("test_results")
        if not self.known_risks.strip():
            missing.append("known_risks")
        if not self.proposed_context_updates.strip():
            missing.append("proposed_context_updates")
        return missing

    def to_dict(self) -> dict[str, Any]:
        return {
            "changed_files": self.changed_files,
            "diff_summary": self.diff_summary,
            "commands_run": self.commands_run,
            "test_results": self.test_results,
            "known_risks": self.known_risks,
            "proposed_context_updates": self.proposed_context_updates,
            "artifact_links": self.artifact_links,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any] | None) -> "EvidencePackage | None":
        if not data:
            return None
        return cls(
            changed_files=list(data.get("changed_files", [])),
            diff_summary=str(data.get("diff_summary", "")),
            commands_run=list(data.get("commands_run", [])),
            test_results=str(data.get("test_results", "")),
            known_risks=str(data.get("known_risks", "")),
            proposed_context_updates=str(data.get("proposed_context_updates", "")),
            artifact_links=list(data.get("artifact_links", [])),
        )


@dataclass
class LocalAgentSession:
    session_id: str
    issue_id: str
    assigned_agent: str
    worktree_path: Path
    task_packet: dict[str, Any]
    status: str = "launched"
    cleanup_eligible: bool = False
    evidence: EvidencePackage | None = None
    evidence_valid: bool = False
    artifacts: dict[str, str] = field(default_factory=dict)
    runner_exit_status: int | None = None
    runner_started_at: str = ""
    runner_ended_at: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "session_id": self.session_id,
            "issue_id": self.issue_id,
            "assigned_agent": self.assigned_agent,
            "worktree_path": str(self.worktree_path),
            "task_packet": self.task_packet,
            "status": self.status,
            "cleanup_eligible": self.cleanup_eligible,
            "evidence": self.evidence.to_dict() if self.evidence else None,
            "evidence_valid": self.evidence_valid,
            "artifacts": self.artifacts,
            "runner_exit_status": self.runner_exit_status,
            "runner_started_at": self.runner_started_at,
            "runner_ended_at": self.runner_ended_at,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "LocalAgentSession":
        return cls(
            session_id=data["session_id"],
            issue_id=data["issue_id"],
            assigned_agent=data["assigned_agent"],
            worktree_path=Path(data["worktree_path"]),
            task_packet=dict(data.get("task_packet", {})),
            status=data.get("status", "launched"),
            cleanup_eligible=bool(data.get("cleanup_eligible", False)),
            evidence=EvidencePackage.from_dict(data.get("evidence")),
            evidence_valid=bool(data.get("evidence_valid", False)),
            artifacts=dict(data.get("artifacts", {})),
            runner_exit_status=data.get("runner_exit_status"),
            runner_started_at=data.get("runner_started_at", ""),
            runner_ended_at=data.get("runner_ended_at", ""),
        )


@dataclass
class ReviewDecision:
    session_id: str
    issue_id: str
    outcome: str
    reason: str
    next_action: str
    limitations: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {
            "session_id": self.session_id,
            "issue_id": self.issue_id,
            "outcome": self.outcome,
            "reason": self.reason,
            "next_action": self.next_action,
            "limitations": self.limitations,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "ReviewDecision":
        return cls(
            session_id=data["session_id"],
            issue_id=data["issue_id"],
            outcome=data["outcome"],
            reason=data.get("reason", ""),
            next_action=data.get("next_action", ""),
            limitations=list(data.get("limitations", [])),
        )


@dataclass
class DelegationDecision:
    issue_id: str
    router_agent: str
    recommended_agent: str
    complexity: str
    reason: str
    requires_approval: bool
    approved: bool = False

    def to_dict(self) -> dict[str, Any]:
        return {
            "issue_id": self.issue_id,
            "router_agent": self.router_agent,
            "recommended_agent": self.recommended_agent,
            "complexity": self.complexity,
            "reason": self.reason,
            "requires_approval": self.requires_approval,
            "approved": self.approved,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "DelegationDecision":
        return cls(
            issue_id=data["issue_id"],
            router_agent=data["router_agent"],
            recommended_agent=data["recommended_agent"],
            complexity=data.get("complexity", ""),
            reason=data.get("reason", ""),
            requires_approval=bool(data.get("requires_approval", False)),
            approved=bool(data.get("approved", False)),
        )


@dataclass
class PrPreparation:
    issue_id: str
    branch_name: str
    title: str
    body: str
    create_command: str
    merge_approved: bool = False


class AlbertMission:
    def __init__(
        self,
        target_repo: Path,
        tracker_dir: Path,
        runtime_root: Path,
        mission_id: str = "mission-001",
        agent_config_path: Path | None = None,
        allow_empty_tracker: bool = False,
        issues_dir: Path | None = None,
    ):
        self.target_repo = target_repo.resolve()
        self.tracker_dir = tracker_dir.resolve()
        self.issues_dir = (issues_dir or (self.tracker_dir / "issues")).resolve()
        self.runtime_root = runtime_root.resolve()
        self.mission_id = mission_id
        self.agent_config_path = (agent_config_path or (self.target_repo / ".albert" / "agents.json")).resolve()
        self.allow_empty_tracker = allow_empty_tracker
        runtime_identity = f"{self.target_repo}\n{self.tracker_dir}\n{self.issues_dir}\n{self.mission_id}"
        digest = sha1(runtime_identity.encode("utf-8")).hexdigest()[:8]
        self.project_key = f"{self.target_repo.name}-{digest}"
        self.prd_title = ""
        self.issues: dict[str, IssueSlice] = {}
        self.sessions: dict[str, LocalAgentSession] = {}
        self.reviews: list[ReviewDecision] = []
        self.delegations: dict[str, DelegationDecision] = {}
        self.command_policy: dict[str, str] = {}
        self.timeline: list[str] = []
        self.agent_registry = AgentRegistry(agents=[], source_path=self.agent_config_path)
        self._evidence_activity_recorder: (
            Callable[[str, LocalAgentSession, EvidencePackage], None] | None
        ) = None

    @property
    def runtime_dir(self) -> Path:
        return self.runtime_root / self.project_key

    @property
    def runtime_path(self) -> Path:
        return self.runtime_dir / "runtime.json"

    def load(self) -> "AlbertMission":
        self.agent_registry = load_agent_registry(self.agent_config_path)
        prd_path = self.tracker_dir / "PRD.md"
        if self.allow_empty_tracker and not prd_path.exists():
            self.prd_title = "Untracked Workspace"
        else:
            self.prd_title = self._load_prd_title()
        self.issues = self._load_issues()
        self._load_runtime()
        self._persist()
        return self

    def board_summary(self) -> dict[str, Any]:
        ordered = self.ordered_issue_ids()
        ready = [issue_id for issue_id in ordered if self._issue_launch_eligible(self.issues[issue_id])]
        return {
            "prd_title": self.prd_title,
            "issue_count": len(self.issues),
            "ordered_issue_ids": ordered,
            "ready_issue_ids": ready,
            "approved_issue_ids": [issue_id for issue_id in ordered if self.issues[issue_id].review_state == "approved"],
            "issue_slices": [self._issue_summary(issue_id) for issue_id in ordered],
        }

    def _issue_summary(self, issue_id: str) -> dict[str, Any]:
        issue = self.issues[issue_id]
        blockers = [
            {
                "issue_id": blocker_id,
                "title": self.issues[blocker_id].title,
                "lifecycle": self._issue_lifecycle(self.issues[blocker_id]),
                "satisfied": self._lifecycle_satisfies_blocker(self.issues[blocker_id]),
            }
            for blocker_id in issue.blocked_by
        ]
        sessions = [
            self._session_summary(session)
            for session in sorted(self.sessions.values(), key=lambda item: item.session_id)
            if session.issue_id == issue.id
        ]
        latest_evidence = next(
            (session["evidence"] for session in reversed(sessions) if session.get("evidence")),
            None,
        )
        return {
            "issue_id": issue.id,
            "title": issue.title,
            "lifecycle": self._issue_lifecycle(issue),
            "progress": self._issue_progress(issue),
            "launch_eligible": self._issue_launch_eligible(issue),
            "blockers": blockers,
            "accepted_boundary": {
                "what_to_build": issue.what_to_build,
                "acceptance_criteria": issue.acceptance_criteria,
                "evidence_requirements": issue.evidence_requirements or self.default_evidence_requirements(),
                "source_path": issue.source_path,
            },
            "sessions": sessions,
            "provenance": self._issue_provenance(issue, sessions),
            "model_assignment": self._model_assignment(issue, sessions),
            "evidence": latest_evidence
            or {
                "state": "missing",
                "changed_files": [],
                "commands_run": [],
                "test_results": "No evidence package recorded.",
                "risks": "None recorded.",
                "artifact_links": [],
            },
            "working_context_sources": [
                {
                    "source_id": f"shared-context:{self.mission_id}:issue-slice:{issue.id}",
                    "kind": "shared-context",
                    "label": f"Shared Context — {issue.title}",
                },
                {
                    "source_id": f"issue:{self.mission_id}:{issue.id}",
                    "kind": "unresolved-item",
                    "label": f"{issue.id} — {issue.title}",
                },
            ],
        }

    def _issue_launch_eligible(self, issue: IssueSlice) -> bool:
        return (
            issue.review_state == "approved"
            and self._assignment_available(issue)
            and "launch" in self._next_actions_for_issue(issue)
        )

    def _issue_lifecycle(self, issue: IssueSlice) -> str:
        if issue.tracker_status.lower() in {"merged"}:
            return "Merged"
        if issue.review_state == "complete":
            return "Merged"
        if issue.review_state == "pr-ready":
            return "Complete"
        if self._issue_launch_eligible(issue):
            return "Ready"
        labels = {
            "approved": "Approved",
            "needs-review": "Needs review",
            "needs-human-review": "Needs human review",
            "needs-repair": "Needs repair",
            "rejected": "Rejected",
        }
        return labels.get(issue.review_state, issue.review_state.replace("-", " ").title())

    def _issue_progress(self, issue: IssueSlice) -> str:
        assignment = self._model_assignment(issue, [])
        if issue.review_state == "approved" and assignment["availability"] != "available":
            reason = assignment["availability_reason"] or assignment["availability"]
            return f"Assigned model unavailable: {reason}"
        if self._issue_launch_eligible(issue):
            return "Launch eligible"
        if issue.blocked_by:
            unsatisfied = [
                blocker
                for blocker in issue.blocked_by
                if not self._lifecycle_satisfies_blocker(self.issues[blocker])
            ]
            if unsatisfied:
                return f"Waiting on {', '.join(unsatisfied)}"
        if issue.review_state == "pr-ready":
            return "Evidence accepted and PR-ready"
        if issue.review_state == "complete":
            return "Merged"
        return self._issue_lifecycle(issue)

    def _session_summary(self, session: LocalAgentSession) -> dict[str, Any]:
        agent_config = session.task_packet.get("agent_config") or {}
        evidence = session.evidence.to_dict() if session.evidence else None
        disconnected = bool(
            session.runner_started_at
            and not session.runner_ended_at
            and session.status not in {"evidence-ready", "reviewed", "complete"}
        )
        return {
            "session_id": session.session_id,
            "assigned_agent": session.assigned_agent,
            "role": str(agent_config.get("role", "local-agent")),
            "provider": str(agent_config.get("provider", "unconfigured")),
            "model": str(agent_config.get("model", session.assigned_agent)),
            "status": session.status,
            "stale": session.status in {"failed", "needs-repair", "rejected"},
            "disconnected": disconnected,
            "operation_status": self._session_operation_status(session, disconnected),
            "failure": self._session_failure(session),
            "evidence": self._evidence_summary(evidence),
        }

    @staticmethod
    def _session_operation_status(session: LocalAgentSession, disconnected: bool) -> str:
        if disconnected:
            return "streaming"
        if session.status == "failed":
            return "failed"
        if session.runner_ended_at:
            return "completed"
        return "idle" if session.status == "launched" else session.status

    @staticmethod
    def _session_failure(session: LocalAgentSession) -> str:
        if session.status != "failed":
            return ""
        if session.evidence and session.evidence.known_risks:
            return session.evidence.known_risks
        return "Provider operation failed; inspect session evidence."

    @staticmethod
    def _evidence_summary(evidence: dict[str, Any] | None) -> dict[str, Any]:
        if evidence is None:
            return {
                "state": "missing",
                "changed_files": [],
                "commands_run": [],
                "test_results": "No evidence package recorded.",
                "risks": "None recorded.",
                "artifact_links": [],
            }
        return {
            "state": "accepted",
            "changed_files": evidence["changed_files"],
            "commands_run": evidence["commands_run"],
            "test_results": evidence["test_results"],
            "risks": evidence["known_risks"],
            "artifact_links": evidence["artifact_links"],
        }

    def _issue_provenance(
        self, issue: IssueSlice, sessions: list[dict[str, Any]]
    ) -> dict[str, str]:
        if sessions:
            latest = sessions[-1]
            return {
                "role": latest["role"],
                "provider": latest["provider"],
                "model": latest["model"],
            }
        agent = self.agent_registry.find(issue.assigned_agent)
        if agent:
            return {"role": agent.role, "provider": agent.provider, "model": agent.model}
        return {
            "role": "local-agent",
            "provider": "unconfigured",
            "model": issue.assigned_agent or issue.suggested_agent,
        }

    def _model_assignment(self, issue: IssueSlice, sessions: list[dict[str, Any]]) -> dict[str, str]:
        agent = self.agent_registry.find(issue.assigned_agent)
        if agent:
            role = agent.role
            provider = agent.provider
            model = agent.model
            availability = agent.availability
            availability_reason = agent.availability_reason
        else:
            role = "local-agent"
            provider = "unconfigured"
            model = issue.assigned_agent or issue.suggested_agent
            availability = "available"
            availability_reason = ""
        latest = sessions[-1] if sessions else None
        operation_status = "idle"
        failure = ""
        if latest:
            operation_status = str(latest.get("operation_status", "idle"))
            failure = str(latest.get("failure", ""))
        return {
            "agent_id": issue.assigned_agent,
            "role": role,
            "provider": provider,
            "model": model,
            "availability": availability,
            "availability_reason": availability_reason,
            "operation_status": operation_status,
            "failure": failure,
        }

    def _assignment_available(self, issue: IssueSlice) -> bool:
        agent = self.agent_registry.find(issue.assigned_agent)
        return not agent or agent.availability == "available"

    def issue_detail(self, issue_id: str) -> dict[str, Any]:
        issue = self._issue(issue_id)
        blockers = [
            {
                "issue_id": blocker,
                "review_state": self._issue(blocker).review_state,
                "satisfied": self._lifecycle_satisfies_blocker(self._issue(blocker)),
            }
            for blocker in issue.blocked_by
        ]
        return {
            "issue_id": issue.id,
            "title": issue.title,
            "tracker_status": issue.tracker_status,
            "runtime_status": issue.status,
            "review_state": issue.review_state,
            "locked": issue.locked,
            "assigned_agent": issue.assigned_agent,
            "notes": issue.notes,
            "blockers": blockers,
            "acceptance_criteria": issue.acceptance_criteria,
            "next_actions": self._next_actions_for_issue(issue),
            "available_agents": [agent.id for agent in self.assignment_agents()],
            "delegation": self.delegations.get(issue.id).to_dict() if issue.id in self.delegations else None,
            "source_path": issue.source_path,
        }

    def ordered_issue_ids(self) -> list[str]:
        result: list[str] = []
        visiting: set[str] = set()
        visited: set[str] = set()

        def visit(issue_id: str) -> None:
            if issue_id in visited:
                return
            if issue_id in visiting:
                raise AlbertError(f"Cycle detected in Issue Graph at {issue_id}.")
            if issue_id not in self.issues:
                raise AlbertError(f"Unknown blocker {issue_id}.")
            visiting.add(issue_id)
            for blocker in self.issues[issue_id].blocked_by:
                visit(blocker)
            visiting.remove(issue_id)
            visited.add(issue_id)
            result.append(issue_id)

        for issue_id in sorted(self.issues):
            visit(issue_id)
        return result

    def approve_issue(self, issue_id: str) -> None:
        issue = self._issue(issue_id)
        if issue.review_state == "complete":
            raise AlbertError(f"{issue_id} is already complete and cannot be approved for launch.")
        issue.review_state = "approved"
        issue.status = "approved"
        issue.locked = True
        self._record(f"{issue_id} approved and locked.")
        self._persist()

    def unlock_issue(self, issue_id: str, reason: str) -> None:
        issue = self._issue(issue_id)
        issue.locked = False
        issue.review_state = "needs-review"
        issue.status = "needs-review"
        self._record(f"{issue_id} unlocked for re-review: {reason}")
        self._persist()

    def reopen_issue(self, issue_id: str, reason: str) -> None:
        if not reason.strip():
            raise AlbertError("Reopen requires a reason.")
        issue = self._issue(issue_id)
        issue.locked = False
        issue.review_state = "needs-review"
        issue.status = "needs-review"
        issue.tracker_status = "needs-review"
        self._write_tracker_status(issue, "needs-review")
        self._record(f"{issue_id} reopened for re-review: {reason}")
        self._persist()

    def assign_issue(self, issue_id: str, assigned_agent: str, notes: str = "", launch_order: int | None = None) -> None:
        issue = self._issue(issue_id)
        if self.agent_registry.configured:
            agent = self.agent_registry.find(assigned_agent)
            if not agent:
                raise AlbertError(f"Unknown configured agent: {assigned_agent}")
            if agent.delegate_only:
                raise AlbertError(f"{assigned_agent} is delegate-only and must be selected by the Frontier router.")
        issue.assigned_agent = assigned_agent
        if notes:
            issue.notes = notes
        if launch_order is not None:
            issue.launch_order = launch_order
        self._record(f"{issue_id} assigned to {assigned_agent}.")
        self._persist()

    def update_issue_contract(
        self,
        issue_id: str,
        *,
        what_to_build: str | None = None,
        acceptance_criteria: list[str] | None = None,
        blocked_by: list[str] | None = None,
        type: str | None = None,
        risk: str | None = None,
        evidence_requirements: list[str] | None = None,
    ) -> None:
        issue = self._issue(issue_id)
        if issue.locked:
            raise LockedFieldError(f"{issue_id} is approved and locked. Unlock before editing contract fields.")
        if what_to_build is not None:
            issue.what_to_build = what_to_build
        if acceptance_criteria is not None:
            issue.acceptance_criteria = acceptance_criteria
        if blocked_by is not None:
            issue.blocked_by = blocked_by
        if type is not None:
            issue.type = type
        if risk is not None:
            issue.risk = risk
        if evidence_requirements is not None:
            issue.evidence_requirements = evidence_requirements
        issue.contract_overridden = True
        issue.review_state = "needs-review"
        issue.status = "needs-review"
        self._record(f"{issue_id} contract changed; re-review required.")
        self._persist()

    def launch_issue(
        self,
        issue_id: str,
        *,
        allowed_paths: list[str] | None = None,
        command_policy: dict[str, str] | None = None,
    ) -> LocalAgentSession:
        issue = self._issue(issue_id)
        if issue.review_state != "approved":
            raise LaunchBlockedError(f"{issue_id} must be approved before launch.")
        unsatisfied = [blocker for blocker in issue.blocked_by if not self._lifecycle_satisfies_blocker(self._issue(blocker))]
        if unsatisfied:
            raise LaunchBlockedError(f"{issue_id} is blocked by {', '.join(unsatisfied)}.")
        if not self._assignment_available(issue):
            assignment = self._model_assignment(issue, [])
            reason = assignment["availability_reason"] or assignment["availability"]
            raise LaunchBlockedError(f"{issue_id} assigned model is unavailable: {reason}.")
        if command_policy:
            self.command_policy.update(command_policy)
        agent_config = self.agent_registry.find(issue.assigned_agent)
        self._ensure_delegation_approved(issue, agent_config)
        if agent_config and agent_config.runner in {"command", "ollama"}:
            runner_command = self._runner_command(agent_config)
            policy = self.classify_command(runner_command)
            if policy != "auto-allowed":
                raise LaunchBlockedError(f"{issue_id} command runner policy is {policy}; auto-allowed is required.")
        session_id = f"session-{issue_id}-{len(self.sessions) + 1}"
        worktree_path = self.target_repo.parent / ".albert-worktrees" / self.target_repo.name / issue_id
        worktree_path.mkdir(parents=True, exist_ok=True)
        task_packet = {
            "issue_id": issue.id,
            "goal": issue.what_to_build,
            "acceptance_criteria": issue.acceptance_criteria,
            "allowed_paths": allowed_paths or [],
            "command_policy": command_policy or {},
            "evidence_requirements": issue.evidence_requirements or self.default_evidence_requirements(),
            "assigned_agent": issue.assigned_agent,
            "agent_config": self._agent_config_for(issue.assigned_agent),
            "notes": issue.notes,
        }
        if issue.id in self.delegations:
            task_packet["delegation"] = self.delegations[issue.id].to_dict()
        session = LocalAgentSession(
            session_id=session_id,
            issue_id=issue.id,
            assigned_agent=issue.assigned_agent,
            worktree_path=worktree_path,
            task_packet=task_packet,
        )
        self.sessions[session_id] = session
        if agent_config and agent_config.runner == "fake":
            self._run_fake_agent(session)
        elif agent_config and agent_config.runner == "command":
            self._run_command_agent(session, agent_config)
        elif agent_config and agent_config.runner == "ollama":
            self._run_ollama_agent(session, agent_config)
        self._record(f"{issue_id} launched as {session_id}.")
        self._persist()
        return session

    def route_issue(self, issue_id: str) -> DelegationDecision:
        issue = self._issue(issue_id)
        if issue.review_state != "approved":
            raise LaunchBlockedError(f"{issue_id} must be approved before routing.")
        router = self._router_agent()
        command = self._runner_command(router)
        if command and self.classify_command(command) != "auto-allowed":
            raise LaunchBlockedError(f"{issue_id} router command policy is {self.classify_command(command)}; auto-allowed is required.")
        prompt = self._delegation_prompt(issue, router)
        try:
            completed = subprocess.run(
                _command_invocation(command),
                input=prompt,
                cwd=self.target_repo,
                capture_output=True,
                text=True,
                check=False,
            )
            exit_status = completed.returncode
            output = completed.stdout
            stderr = completed.stderr
        except FileNotFoundError as exc:
            raise AlbertError(f"Router command failed: {exc}") from exc
        if exit_status != 0:
            raise AlbertError(f"Router command exited {exit_status}: {stderr.strip()}")
        data = _parse_delegation_decision(output)
        recommended_agent = data["recommended_agent"]
        agent = self.agent_registry.find(recommended_agent)
        if not agent:
            raise AlbertError(f"Router recommended unknown configured agent: {recommended_agent}")
        requires_approval = bool(data["requires_approval"] or agent.requires_approval or agent.model.endswith(":cloud"))
        decision = DelegationDecision(
            issue_id=issue.id,
            router_agent=router.id,
            recommended_agent=recommended_agent,
            complexity=data["complexity"],
            reason=data["reason"],
            requires_approval=requires_approval,
        )
        self.delegations[issue.id] = decision
        issue.assigned_agent = recommended_agent
        self._record(f"{issue.id} routed by {router.id} to {recommended_agent}: {decision.reason}")
        self._persist()
        return decision

    def approve_delegation(self, issue_id: str) -> DelegationDecision:
        issue = self._issue(issue_id)
        decision = self.delegations.get(issue.id)
        if not decision:
            raise AlbertError(f"{issue_id} has no delegation decision to approve.")
        agent = self.agent_registry.require(decision.recommended_agent)
        if not decision.requires_approval:
            decision.approved = True
            self._persist()
            return decision
        decision.approved = True
        command = self._runner_command(agent)
        if command:
            self.command_policy[command] = "auto-allowed"
        self._record(f"{issue.id} delegation approved for {decision.recommended_agent}.")
        self._persist()
        return decision

    def launch_repair(
        self,
        session_id: str,
        *,
        agent_id: str = "",
        allowed_paths: list[str] | None = None,
        command_policy: dict[str, str] | None = None,
    ) -> LocalAgentSession:
        prior_session = self._session(session_id)
        issue = self._issue(prior_session.issue_id)
        review = self._latest_review_for_session(session_id)
        if not review or review.next_action not in {"same-local-agent-repair", "fresh-local-agent-repair"}:
            raise LaunchBlockedError(f"{session_id} does not have a repairable Frontier review.")
        unsatisfied = [blocker for blocker in issue.blocked_by if not self._lifecycle_satisfies_blocker(self._issue(blocker))]
        if unsatisfied:
            raise LaunchBlockedError(f"{issue.id} is blocked by {', '.join(unsatisfied)}.")
        if command_policy:
            self.command_policy.update(command_policy)
        assigned_agent = agent_id or prior_session.assigned_agent
        if self.agent_registry.configured and not self.agent_registry.find(assigned_agent):
            raise AlbertError(f"Unknown configured agent: {assigned_agent}")
        agent_config = self.agent_registry.find(assigned_agent)
        if agent_config and agent_config.availability != "available":
            reason = agent_config.availability_reason or agent_config.availability
            raise LaunchBlockedError(f"{issue.id} assigned model is unavailable: {reason}.")
        if agent_config and agent_config.runner in {"command", "ollama"}:
            runner_command = self._runner_command(agent_config)
            policy = self.classify_command(runner_command)
            if policy != "auto-allowed":
                raise LaunchBlockedError(f"{issue.id} command runner policy is {policy}; auto-allowed is required.")
        issue.assigned_agent = assigned_agent
        repair_context = {
            "prior_session_id": prior_session.session_id,
            "review_outcome": review.outcome,
            "review_reason": review.reason,
            "next_action": review.next_action,
            "prior_evidence": prior_session.evidence.to_dict() if prior_session.evidence else None,
            "prior_artifacts": prior_session.artifacts,
        }
        repair_session_id = f"session-{issue.id}-{len(self.sessions) + 1}"
        worktree_path = self.target_repo.parent / ".albert-worktrees" / self.target_repo.name / issue.id
        worktree_path.mkdir(parents=True, exist_ok=True)
        task_packet = {
            "issue_id": issue.id,
            "goal": issue.what_to_build,
            "acceptance_criteria": issue.acceptance_criteria,
            "allowed_paths": allowed_paths or [],
            "command_policy": command_policy or {},
            "evidence_requirements": issue.evidence_requirements or self.default_evidence_requirements(),
            "assigned_agent": assigned_agent,
            "agent_config": self._agent_config_for(assigned_agent),
            "notes": issue.notes,
            "repair_context": repair_context,
        }
        session = LocalAgentSession(
            session_id=repair_session_id,
            issue_id=issue.id,
            assigned_agent=assigned_agent,
            worktree_path=worktree_path,
            task_packet=task_packet,
        )
        self.sessions[repair_session_id] = session
        if agent_config and agent_config.runner == "fake":
            self._run_fake_agent(session)
        elif agent_config and agent_config.runner == "command":
            self._run_command_agent(session, agent_config)
        elif agent_config and agent_config.runner == "ollama":
            self._run_ollama_agent(session, agent_config)
        self._record(f"{issue.id} repair launched as {repair_session_id} from {session_id}.")
        self._persist()
        return session

    def launch_headless_work(
        self,
        *,
        work_kind: str,
        agent_id: str,
        prompt: str = "",
        review_session_id: str = "",
        allowed_paths: list[str] | None = None,
        command_policy: dict[str, str] | None = None,
    ) -> LocalAgentSession:
        if work_kind not in {"run", "review"}:
            raise AlbertError(f"Unknown headless work kind: {work_kind}")
        agent_config = self.agent_registry.require(agent_id)
        if agent_config.availability != "available":
            reason = agent_config.availability_reason or agent_config.availability
            raise LaunchBlockedError(f"{agent_id} assigned model is unavailable: {reason}.")
        if command_policy:
            self.command_policy.update(command_policy)
        if agent_config.runner in {"command", "ollama"}:
            runner_command = self._runner_command(agent_config)
            policy = self.classify_command(runner_command)
            if policy != "auto-allowed":
                raise LaunchBlockedError(
                    f"{agent_id} command runner policy is {policy}; auto-allowed is required."
                )
        work_id = f"headless-{work_kind}-{len(self.sessions) + 1:06d}"
        session_id = f"session-{work_id}"
        worktree_path = self.target_repo.parent / ".albert-worktrees" / self.target_repo.name / work_id
        worktree_path.mkdir(parents=True, exist_ok=True)
        goal = prompt.strip()
        review_context: dict[str, Any] | None = None
        if work_kind == "review":
            if review_session_id:
                prior_session = self._session(review_session_id)
                review_context = {
                    "session_id": prior_session.session_id,
                    "issue_id": prior_session.issue_id,
                    "assigned_agent": prior_session.assigned_agent,
                    "status": prior_session.status,
                    "evidence_valid": prior_session.evidence_valid,
                    "evidence": prior_session.evidence.to_dict() if prior_session.evidence else None,
                    "artifacts": prior_session.artifacts,
                    "runner_exit_status": prior_session.runner_exit_status,
                }
            goal = (
                f"Review session {review_session_id}."
                if review_session_id
                else "Review the current Alfredo workspace state."
            )
        task_packet = {
            "issue_id": work_id,
            "work_kind": f"headless-{work_kind}",
            "goal": goal,
            "prompt": prompt,
            "review_session_id": review_session_id,
            "acceptance_criteria": [
                "Return terminal-suitable lifecycle output.",
                "Respect Orchestrator governance and Evidence Package boundaries.",
            ],
            "allowed_paths": allowed_paths or [],
            "command_policy": command_policy or {},
            "evidence_requirements": self.default_evidence_requirements(),
            "assigned_agent": agent_id,
            "agent_config": self._agent_config_for(agent_id),
        }
        if review_context is not None:
            task_packet["review_context"] = review_context
        session = LocalAgentSession(
            session_id=session_id,
            issue_id=work_id,
            assigned_agent=agent_id,
            worktree_path=worktree_path,
            task_packet=task_packet,
        )
        self.sessions[session_id] = session
        if agent_config.runner == "fake":
            self._run_fake_agent(session)
        elif agent_config.runner == "command":
            self._run_command_agent(session, agent_config)
        elif agent_config.runner == "ollama":
            self._run_ollama_agent(session, agent_config)
        self._record(f"{work_id} launched as {session_id}.")
        self._persist()
        return session

    def classify_command(self, command: str) -> str:
        if command in self.command_policy:
            return self.command_policy[command]
        stripped = command.strip()
        if stripped.startswith("rm ") or " rm -rf" in f" {stripped}" or stripped.startswith("sudo "):
            return "human-required"
        if stripped.startswith("git push") or stripped.startswith("gh pr create"):
            return "frontier-approvable"
        if stripped.startswith("python -m unittest") or stripped.startswith("python3 -m unittest"):
            return "auto-allowed"
        if stripped.startswith("pytest") or stripped.startswith("npm test"):
            return "auto-allowed"
        if stripped.startswith("ollama run "):
            return "auto-allowed"
        return "human-required"

    def record_command_approval(self, command: str, level: str) -> None:
        if level not in {"auto-allowed", "frontier-approvable", "human-required"}:
            raise AlbertError(f"Unknown command policy level: {level}")
        self.command_policy[command] = level
        self._record(f"Command policy learned: {command} -> {level}.")
        self._persist()

    def classify_file_for_frontier(self, path: str) -> str:
        normalized = path.replace("\\", "/")
        if normalized.startswith("./"):
            normalized = normalized[2:]
        name = Path(normalized).name
        if name in {".env", ".env.local"} or normalized.endswith(".pem") or normalized.endswith(".key"):
            return "Blocked"
        if normalized.startswith(".local/") or "/.local/" in normalized:
            return "Local-only"
        return "Normal"

    def list_agents(self) -> list[AgentConfig]:
        return self.agent_registry.agents

    def assignment_agents(self) -> list[AgentConfig]:
        return [
            agent
            for agent in self.agent_registry.agents
            if not agent.delegate_only and agent.routing != "router" and agent.role != "frontier"
        ]

    def _runner_command(self, agent_config: AgentConfig) -> str:
        if agent_config.command:
            return agent_config.command
        if agent_config.runner == "ollama":
            return f"ollama run {agent_config.model} --think false --nowordwrap --format json"
        return ""

    def record_evidence(self, session_id: str, evidence: EvidencePackage) -> None:
        session = self._session(session_id)
        missing = evidence.missing_fields()
        if missing:
            raise EvidenceValidationError(f"Evidence Package is missing: {', '.join(missing)}")
        session.evidence = evidence
        session.evidence_valid = True
        session.status = "evidence-ready"
        self._record(f"{session.issue_id} evidence package validated for {session_id}.")
        self._persist()
        if self._evidence_activity_recorder is not None:
            self._evidence_activity_recorder(self.mission_id, session, evidence)

    def cancel_session(self, session_id: str, *, reason: str) -> LocalAgentSession:
        if not reason.strip():
            raise AlbertError("Session cancellation requires a reason.")
        session = self._session(session_id)
        if session.status in {
            "cancelled",
            "evidence-ready",
            "reviewed",
            "complete",
            "completed",
            "failed",
        }:
            raise AlbertError(f"{session_id} cannot be cancelled from {session.status}.")
        session.status = "cancelled"
        session.runner_ended_at = session.runner_ended_at or datetime.now(timezone.utc).isoformat(
            timespec="seconds"
        )
        self._record(f"{session.issue_id} session {session_id} cancelled: {reason}")
        self._persist()
        return session

    def record_frontier_review(
        self,
        session_id: str,
        outcome: str,
        *,
        reason: str,
        failure_type: str = "",
        limitations: list[str] | None = None,
    ) -> ReviewDecision:
        outcome = _normalize_review_outcome(outcome)
        session = self._session(session_id)
        if outcome in {"Approved", "Approved with limitations"} and not session.evidence_valid:
            raise EvidenceValidationError("Approved outcomes require a valid Evidence Package.")
        next_action = self._next_action_for_review(session_id, outcome, failure_type)
        decision = ReviewDecision(
            session_id=session_id,
            issue_id=session.issue_id,
            outcome=outcome,
            reason=reason,
            next_action=next_action,
            limitations=limitations or [],
        )
        self.reviews.append(decision)
        session.status = "reviewed"
        issue = self.issues.get(session.issue_id)
        if outcome in {"Approved", "Approved with limitations"}:
            session.cleanup_eligible = True
            if issue:
                issue.review_state = "pr-ready"
        elif issue and outcome == "Needs repair":
            issue.review_state = "needs-repair"
        elif issue and outcome == "Needs human review":
            issue.review_state = "needs-human-review"
        elif issue and outcome == "Rejected":
            issue.review_state = "rejected"
        self._record(f"{session.issue_id} frontier review: {outcome}; next action {next_action}.")
        self._persist()
        return decision

    def generate_mission_records(self) -> Path:
        mission_dir = self.target_repo / "docs" / "missions" / self.mission_id
        issue_dir = mission_dir / "issues"
        issue_dir.mkdir(parents=True, exist_ok=True)
        summary = self.board_summary()
        next_action = self._next_action()
        self._write(
            mission_dir / "README.md",
            "\n".join(
                [
                    f"# Mission {self.mission_id}",
                    "",
                    f"Product Requirements Document: {self.prd_title}",
                    f"Status: {len(summary['approved_issue_ids'])}/{summary['issue_count']} Issue Slices approved",
                    f"Next action: {next_action}",
                    "",
                    "## Issue Slices",
                    *[f"- {issue_id}: {self.issues[issue_id].title} ({self.issues[issue_id].review_state})" for issue_id in summary["ordered_issue_ids"]],
                    "",
                ]
            ),
        )
        self._write(mission_dir / "timeline.md", "# Timeline\n\n" + "\n".join(f"- {event}" for event in self.timeline) + "\n")
        self._write(
            mission_dir / "local-agent-tracker.md",
            "# Local Agent Tracker\n\n"
            + "\n".join(
                f"- {session.session_id}: {session.issue_id} assigned to {session.assigned_agent}; status {session.status}"
                for session in self.sessions.values()
            )
            + "\n",
        )
        self._write(
            mission_dir / "evidence-index.md",
            "# Evidence Index\n\n"
            + "\n".join(self._evidence_index_lines())
            + "\n",
        )
        self._write(
            mission_dir / "frontier-review-summary.md",
            "# Frontier Review Summary\n\n"
            + "\n".join(
                f"- {review.issue_id}: {review.outcome}; next action {review.next_action}; reason {review.reason}"
                for review in self.reviews
            )
            + "\n",
        )
        for issue in self.issues.values():
            session_lines = [
                f"- Session {session.session_id}: status {session.status}"
                for session in self.sessions.values()
                if session.issue_id == issue.id
            ]
            self._write(
                issue_dir / f"{issue.id}.md",
                "\n".join(
                    [
                        f"# {issue.id} - {issue.title}",
                        "",
                        f"Tracker issue: {issue.source_path}",
                        f"Execution status: {issue.review_state}",
                        f"Review status: {issue.review_state}",
                        "",
                        "## Local Agent Activity",
                        *(session_lines or ["- No Local Agent session yet."]),
                        "",
                    ]
                ),
            )
        self._persist()
        return mission_dir

    def prepare_pr(self, issue_id: str, *, gh_available: bool) -> PrPreparation:
        issue = self._issue(issue_id)
        reviews = [review for review in self.reviews if review.issue_id == issue_id]
        approved = [review for review in reviews if review.outcome in {"Approved", "Approved with limitations"}]
        if not approved:
            raise AlbertError(f"{issue_id} is not PR-ready.")
        branch_name = f"albert/{self.mission_id}/{issue.id}-{issue.slug}"
        title = f"{issue.id}: {issue.title}"
        evidence_lines = self._evidence_index_lines(issue_id=issue_id)
        body_lines = [
            f"# {title}",
            "",
            "## Issue Slice",
            issue.what_to_build,
            "",
            "## What Changed",
            "See the linked Evidence Package and Local Agent activity for implementation details.",
            "",
            "## Acceptance Criteria",
            *[f"- {criterion}" for criterion in issue.acceptance_criteria],
            "",
            "## Evidence",
            *(evidence_lines or ["- Evidence recorded in app-local runtime state."]),
            "",
            "## Frontier Review",
            *[f"- {review.outcome}: {review.reason}" for review in reviews],
            "",
            "## Local Agent Activity",
            *[
                f"- {session.session_id}: {session.status}"
                for session in self.sessions.values()
                if session.issue_id == issue_id
            ],
            "",
        ]
        if gh_available:
            create_command = f"gh pr create --head {branch_name} --title {json.dumps(title)} --body-file <generated-body-file>"
        else:
            body_lines.extend(
                [
                    "## Manual PR instructions",
                    f"Push branch `{branch_name}` and open a PR with this summary.",
                    "Do not auto-merge; final merge is human-only.",
                    "",
                ]
            )
            create_command = ""
        self._record(f"{issue_id} prepared for PR on {branch_name}.")
        self._persist()
        return PrPreparation(
            issue_id=issue_id,
            branch_name=branch_name,
            title=title,
            body="\n".join(body_lines),
            create_command=create_command,
            merge_approved=False,
        )

    @staticmethod
    def default_evidence_requirements() -> list[str]:
        return [
            "changed files",
            "diff summary",
            "commands run",
            "test results",
            "known risks",
            "proposed context updates",
        ]

    def _load_prd_title(self) -> str:
        prd_path = self.tracker_dir / "PRD.md"
        if not prd_path.exists():
            raise AlbertError(f"Missing Product Requirements Document: {prd_path}")
        for line in prd_path.read_text(encoding="utf-8").splitlines():
            if line.startswith("# "):
                return line.removeprefix("# ").strip()
        return prd_path.stem

    def _load_issues(self) -> dict[str, IssueSlice]:
        issues_dir = self.issues_dir
        if not issues_dir.exists():
            if self.allow_empty_tracker:
                return {}
            raise AlbertError(f"Missing issues directory: {issues_dir}")
        issues: dict[str, IssueSlice] = {}
        for path in sorted(issues_dir.glob("*.md")):
            if path.name.upper() in {"README.MD", "PRD.MD"}:
                continue
            if _record_type(path.read_text(encoding="utf-8")) == "prd":
                continue
            issue = self._parse_issue(path)
            issues[issue.id] = issue
        if not issues and not self.allow_empty_tracker:
            raise AlbertError(f"No Issue Slice records found in {issues_dir}")
        return issues

    def _parse_issue(self, path: Path) -> IssueSlice:
        match = re.match(r"(?P<num>\d+)-(?P<slug>.+)\.md$", path.name)
        if not match:
            raise AlbertError(f"Issue file must start with a number: {path.name}")
        issue_id = f"ISS-{int(match.group('num')):02d}"
        slug = _slug(match.group("slug"))
        text = path.read_text(encoding="utf-8")
        metadata = _metadata(text)
        sections = _sections(text)
        what = sections.get("What to build", "").strip()
        acceptance = _checklist_items(sections.get("Acceptance criteria", ""))
        blockers = _issue_refs(sections.get("Blocked by", ""), issues_dir=path.parent)
        if not what:
            raise AlbertError(f"{path.name} is missing a What to build section.")
        if not acceptance:
            raise AlbertError(f"{path.name} is missing acceptance criteria.")
        title = slug.replace("-", " ").title()
        evidence = _checklist_items(sections.get("Evidence required", ""))
        status = metadata.get("status", "ready-for-agent")
        review_state = {
            "complete": "complete",
            "completed": "complete",
            "approved": "approved",
            "pr-ready": "pr-ready",
        }.get(status.lower(), "needs-review")
        runtime_status = review_state
        return IssueSlice(
            id=issue_id,
            slug=slug,
            title=title,
            status=runtime_status,
            tracker_status=status,
            type=metadata.get("type", "AFK"),
            risk=metadata.get("risk", "Medium"),
            suggested_agent=metadata.get("suggested agent", "qwen-coder-local-1"),
            assigned_agent=metadata.get("assigned agent", metadata.get("suggested agent", "qwen-coder-local-1")),
            what_to_build=what,
            acceptance_criteria=acceptance,
            blocked_by=blockers,
            source_path=str(path),
            evidence_requirements=evidence,
            review_state=review_state,
            locked=review_state in {"approved", "pr-ready", "complete"},
        )

    def _load_runtime(self) -> None:
        if not self.runtime_path.exists():
            return
        data = json.loads(self.runtime_path.read_text(encoding="utf-8"))
        for issue_id, runtime in data.get("issues", {}).items():
            if issue_id in self.issues:
                self.issues[issue_id].apply_runtime(runtime)
        self.sessions = {
            session_id: LocalAgentSession.from_dict(session)
            for session_id, session in data.get("sessions", {}).items()
        }
        self.reviews = [ReviewDecision.from_dict(item) for item in data.get("reviews", [])]
        self.delegations = {
            issue_id: DelegationDecision.from_dict(item)
            for issue_id, item in data.get("delegations", {}).items()
        }
        self.command_policy = dict(data.get("command_policy", {}))
        self.timeline = list(data.get("timeline", []))

    def _persist(self) -> None:
        self.runtime_dir.mkdir(parents=True, exist_ok=True)
        data = {
            "mission_id": self.mission_id,
            "project_key": self.project_key,
            "prd_title": self.prd_title,
            "issues": {issue_id: issue.to_runtime() for issue_id, issue in self.issues.items()},
            "sessions": {session_id: session.to_dict() for session_id, session in self.sessions.items()},
            "reviews": [review.to_dict() for review in self.reviews],
            "delegations": {issue_id: decision.to_dict() for issue_id, decision in self.delegations.items()},
            "command_policy": self.command_policy,
            "timeline": self.timeline,
        }
        payload = json.dumps(data, indent=2, sort_keys=True)
        temporary_path: Path | None = None
        try:
            with tempfile.NamedTemporaryFile(
                mode="w",
                encoding="utf-8",
                dir=self.runtime_dir,
                prefix=f".{self.runtime_path.name}.",
                suffix=".tmp",
                delete=False,
            ) as temporary:
                temporary.write(payload)
                temporary_path = Path(temporary.name)
            temporary_path.replace(self.runtime_path)
        finally:
            if temporary_path is not None and temporary_path.exists():
                temporary_path.unlink()

    def _issue(self, issue_id: str) -> IssueSlice:
        if issue_id not in self.issues:
            raise AlbertError(f"Unknown Issue Slice: {issue_id}")
        return self.issues[issue_id]

    def _session(self, session_id: str) -> LocalAgentSession:
        if session_id not in self.sessions:
            raise AlbertError(f"Unknown Local Agent session: {session_id}")
        return self.sessions[session_id]

    def _agent_config_for(self, agent_id: str) -> dict[str, str] | None:
        agent = self.agent_registry.find(agent_id)
        return agent.to_dict() if agent else None

    def _router_agent(self) -> AgentConfig:
        for agent in self.agent_registry.agents:
            if agent.routing == "router":
                return agent
        for agent in self.agent_registry.agents:
            if agent.role == "frontier":
                return agent
        raise AlbertError("No Frontier router agent is configured.")

    def _has_router_agent(self) -> bool:
        return any(agent.routing == "router" for agent in self.agent_registry.agents)

    def _ensure_delegation_approved(self, issue: IssueSlice, agent_config: AgentConfig | None) -> None:
        if not agent_config:
            return
        decision = self.delegations.get(issue.id)
        approval_required = agent_config.requires_approval or agent_config.model.endswith(":cloud")
        if not approval_required:
            return
        if not decision or decision.recommended_agent != agent_config.id or not decision.approved:
            raise LaunchBlockedError(f"{issue.id} delegation requires approval before launch.")

    def _delegation_prompt(self, issue: IssueSlice, router: AgentConfig) -> str:
        candidates = [
            agent.to_dict()
            for agent in self.agent_registry.agents
            if agent.routing in {"worker", "delegate"} or agent.role in {"local-agent", "delegate-agent"}
        ]
        return "\n".join(
            [
                "You are Albert's Frontier router.",
                f"Router model: {router.model or router.id}",
                "Choose exactly one worker for this Issue Slice.",
                "Return only JSON with this schema:",
                '{"complexity": "low|medium|high|architectural", "recommended_agent": "agent id", "requires_approval": false, "reason": "short reason"}',
                "Use Gemma workers for low and medium work. Use qwen2.5-coder-14b for complex long-horizon coding. Use deepseek-r1-14b for architectural reasoning or repeated-failure work.",
                "",
                "Issue Slice:",
                json.dumps(
                    {
                        "issue_id": issue.id,
                        "title": issue.title,
                        "goal": issue.what_to_build,
                        "acceptance_criteria": issue.acceptance_criteria,
                        "risk": issue.risk,
                        "type": issue.type,
                    },
                    indent=2,
                    sort_keys=True,
                ),
                "",
                "Candidate agents:",
                json.dumps(candidates, indent=2, sort_keys=True),
                "",
            ]
        )

    def _run_fake_agent(self, session: LocalAgentSession) -> None:
        artifact_dir = self.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        task_packet_path = artifact_dir / "task-packet.json"
        log_path = artifact_dir / "fake-agent.log"
        completion_path = artifact_dir / "completion.json"
        result_path = session.worktree_path / "FAKE_AGENT_RESULT.md"
        self._write(task_packet_path, json.dumps(session.task_packet, indent=2, sort_keys=True) + "\n")
        self._write(
            log_path,
            "\n".join(
                [
                    f"Fake Local Agent: {session.assigned_agent}",
                    f"Issue Slice: {session.issue_id}",
                    "Result: deterministic completion",
                    "",
                ]
            ),
        )
        self._write(
            completion_path,
            json.dumps(
                {
                    "session_id": session.session_id,
                    "issue_id": session.issue_id,
                    "status": "completed",
                    "changed_files": ["FAKE_AGENT_RESULT.md"],
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
        )
        self._write(
            result_path,
            "\n".join(
                [
                    f"# Fake Agent Result for {session.issue_id}",
                    "",
                    "This deterministic artifact proves the local runner path executed.",
                    "",
                ]
            ),
        )
        session.evidence = EvidencePackage(
            changed_files=["FAKE_AGENT_RESULT.md"],
            diff_summary=f"Deterministic fake completion for {session.issue_id}.",
            commands_run=[f"fake-agent {session.assigned_agent}"],
            test_results="Not run: deterministic fake runner.",
            known_risks="Fake runner does not perform real code edits.",
            proposed_context_updates="None.",
            artifact_links=[str(task_packet_path), str(log_path), str(completion_path)],
        )
        session.evidence_valid = True
        session.status = "evidence-ready"
        self._record(f"{session.issue_id} fake runner produced evidence for {session.session_id}.")

    def _run_command_agent(self, session: LocalAgentSession, agent_config: AgentConfig) -> None:
        command = agent_config.command
        artifact_dir = self.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        task_packet_path = artifact_dir / "task-packet.json"
        stdout_path = artifact_dir / "stdout.log"
        stderr_path = artifact_dir / "stderr.log"
        result_path = artifact_dir / "runner-result.json"
        self._write(task_packet_path, json.dumps(session.task_packet, indent=2, sort_keys=True) + "\n")
        session.runner_started_at = _utc_now()
        env = os.environ.copy()
        env["ALBERT_TASK_PACKET"] = str(task_packet_path)
        env["ALBERT_SESSION_ID"] = session.session_id
        try:
            completed = subprocess.run(
                _command_invocation(command),
                cwd=session.worktree_path,
                capture_output=True,
                text=True,
                check=False,
                env=env,
            )
            exit_status = completed.returncode
            stdout = completed.stdout
            stderr = completed.stderr
        except FileNotFoundError as exc:
            exit_status = 127
            stdout = ""
            stderr = f"Unable to start {command!r}: {exc}"
        session.runner_ended_at = _utc_now()
        self._write(stdout_path, stdout)
        self._write(stderr_path, stderr)
        self._write(
            result_path,
            json.dumps(
                {
                    "session_id": session.session_id,
                    "issue_id": session.issue_id,
                    "command": command,
                    "exit_status": exit_status,
                    "started_at": session.runner_started_at,
                    "ended_at": session.runner_ended_at,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
        )
        session.runner_exit_status = exit_status
        session.status = "completed" if exit_status == 0 else "failed"
        session.artifacts = {
            "task_packet": str(task_packet_path),
            "stdout": str(stdout_path),
            "stderr": str(stderr_path),
            "result": str(result_path),
        }
        self._collect_automated_evidence(session, agent_config)
        self._record(f"{session.issue_id} command runner exited {exit_status} for {session.session_id}.")

    def _run_ollama_agent(self, session: LocalAgentSession, agent_config: AgentConfig) -> None:
        artifact_dir = self.runtime_dir / "sessions" / session.session_id
        artifact_dir.mkdir(parents=True, exist_ok=True)
        task_packet_path = artifact_dir / "task-packet.json"
        prompt_path = artifact_dir / "ollama-prompt.txt"
        output_path = artifact_dir / "ollama-output.txt"
        stderr_path = artifact_dir / "ollama-stderr.log"
        result_path = artifact_dir / "ollama-result.json"
        command = self._runner_command(agent_config)
        prompt = self._ollama_prompt(session, agent_config)
        self._write(task_packet_path, json.dumps(session.task_packet, indent=2, sort_keys=True) + "\n")
        self._write(prompt_path, prompt)
        session.runner_started_at = _utc_now()
        try:
            completed = subprocess.run(
                _command_invocation(command),
                input=prompt,
                cwd=session.worktree_path,
                capture_output=True,
                text=True,
                check=False,
            )
            exit_status = completed.returncode
            stdout = completed.stdout
            stderr = completed.stderr
        except FileNotFoundError as exc:
            exit_status = 127
            stdout = ""
            stderr = f"Unable to start {command!r}: {exc}"
        session.runner_ended_at = _utc_now()
        self._write(output_path, stdout)
        self._write(stderr_path, stderr)
        known_risk = ""
        if exit_status == 0:
            try:
                plan = _parse_model_file_plan(stdout)
                for file_plan in plan["files"]:
                    self._write_model_file(session.worktree_path, file_plan["path"], file_plan["content"])
            except AlbertError as exc:
                session.status = "failed"
                known_risk = f"Malformed Ollama output: {exc}"
        else:
            session.status = "failed"
            known_risk = f"Ollama command exited {exit_status}; inspect stderr artifact."
        self._write(
            result_path,
            json.dumps(
                {
                    "session_id": session.session_id,
                    "issue_id": session.issue_id,
                    "command": command,
                    "model": agent_config.model,
                    "exit_status": exit_status,
                    "started_at": session.runner_started_at,
                    "ended_at": session.runner_ended_at,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
        )
        session.runner_exit_status = exit_status
        session.artifacts = {
            "task_packet": str(task_packet_path),
            "ollama_prompt": str(prompt_path),
            "ollama_output": str(output_path),
            "stderr": str(stderr_path),
            "result": str(result_path),
        }
        self._collect_automated_evidence(session, agent_config, known_risk_override=known_risk)
        self._record(f"{session.issue_id} ollama runner exited {exit_status} for {session.session_id}.")

    def _ollama_prompt(self, session: LocalAgentSession, agent_config: AgentConfig) -> str:
        return "\n".join(
            [
                "You are Albert's local coding agent.",
                f"Model: {agent_config.model}",
                "Return only JSON with this schema:",
                '{"summary": "short text", "files": [{"path": "relative/path", "content": "file contents"}]}',
                "Do not include markdown fences or commentary.",
                "",
                "Task packet:",
                json.dumps(session.task_packet, indent=2, sort_keys=True),
                "",
            ]
        )

    def _write_model_file(self, worktree_path: Path, relative_path: str, content: str) -> None:
        path = Path(relative_path)
        if path.is_absolute() or ".." in path.parts:
            raise AlbertError(f"unsafe file path {relative_path!r}")
        self._write(worktree_path / path, content)

    def _collect_automated_evidence(
        self,
        session: LocalAgentSession,
        agent_config: AgentConfig,
        *,
        known_risk_override: str = "",
    ) -> None:
        changed_files = self._worktree_changed_files(session.worktree_path)
        if changed_files:
            diff_summary = "Changed files: " + ", ".join(changed_files)
        else:
            changed_files = ["No worktree file changes detected."]
            diff_summary = "No worktree file changes detected."
        test_results = self._collect_test_results(session, agent_config)
        known_risks = known_risk_override or "None."
        if not known_risk_override and session.runner_exit_status and session.runner_exit_status != 0:
            known_risks = f"Runner exited {session.runner_exit_status}; inspect stderr artifact."
        elif not known_risk_override and test_results.startswith("Test command failed"):
            known_risks = "Test command failed; inspect test stderr artifact."
        session.evidence = EvidencePackage(
            changed_files=changed_files,
            diff_summary=diff_summary,
            commands_run=[self._runner_command(agent_config)],
            test_results=test_results,
            known_risks=known_risks,
            proposed_context_updates="None.",
            artifact_links=list(session.artifacts.values()),
        )
        session.evidence_valid = True
        if session.status != "failed" and session.runner_exit_status == 0 and not test_results.startswith("Test command failed"):
            session.status = "evidence-ready"

    def _collect_test_results(self, session: LocalAgentSession, agent_config: AgentConfig) -> str:
        if not agent_config.test_command:
            return "Not applicable: no test command configured."
        policy = self.classify_command(agent_config.test_command)
        if policy != "auto-allowed":
            return f"Not applicable: test command policy is {policy}."
        artifact_dir = self.runtime_dir / "sessions" / session.session_id
        stdout_path = artifact_dir / "test-stdout.log"
        stderr_path = artifact_dir / "test-stderr.log"
        result_path = artifact_dir / "test-result.json"
        env = os.environ.copy()
        env["ALBERT_TASK_PACKET"] = session.artifacts.get("task_packet", "")
        env["ALBERT_SESSION_ID"] = session.session_id
        try:
            completed = subprocess.run(
                _command_invocation(agent_config.test_command),
                cwd=session.worktree_path,
                capture_output=True,
                text=True,
                check=False,
                env=env,
            )
            exit_status = completed.returncode
            stdout = completed.stdout
            stderr = completed.stderr
        except FileNotFoundError as exc:
            exit_status = 127
            stdout = ""
            stderr = f"Unable to start {agent_config.test_command!r}: {exc}"
        self._write(stdout_path, stdout)
        self._write(stderr_path, stderr)
        self._write(
            result_path,
            json.dumps(
                {
                    "command": agent_config.test_command,
                    "exit_status": exit_status,
                    "session_id": session.session_id,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
        )
        session.artifacts.update(
            {
                "test_stdout": str(stdout_path),
                "test_stderr": str(stderr_path),
                "test_result": str(result_path),
            }
        )
        if exit_status == 0:
            return f"Test command passed: {agent_config.test_command}"
        session.status = "failed"
        return f"Test command failed (exit {exit_status}): {agent_config.test_command}"

    @staticmethod
    def _worktree_changed_files(worktree_path: Path) -> list[str]:
        if not worktree_path.exists():
            return []
        files: list[str] = []
        for path in sorted(worktree_path.rglob("*")):
            if path.is_file():
                files.append(path.relative_to(worktree_path).as_posix())
        return files

    def _blockers_satisfied(self, issue_id: str) -> bool:
        return all(self._lifecycle_satisfies_blocker(self.issues[blocker]) for blocker in self.issues[issue_id].blocked_by)

    @staticmethod
    def _lifecycle_satisfies_blocker(issue: IssueSlice) -> bool:
        return issue.review_state in {"approved", "pr-ready", "complete"}

    def _next_actions_for_issue(self, issue: IssueSlice) -> list[str]:
        if issue.review_state == "complete":
            return ["reopen"]
        if issue.review_state in {"pr-ready"}:
            return ["prepare-pr", "reopen"]
        if issue.review_state in {"needs-repair", "rejected"}:
            return ["repair", "reopen", "record-review"]
        if issue.review_state == "approved":
            delegation = self.delegations.get(issue.id)
            if delegation and delegation.requires_approval and not delegation.approved:
                return ["approve-delegation", "reopen"]
            if not delegation and self._has_router_agent():
                return ["route", "launch", "reopen"]
            if self._blockers_satisfied(issue.id):
                return ["launch", "reopen"]
            return ["wait-for-blockers", "reopen"]
        if issue.review_state == "needs-human-review":
            return ["record-review", "reopen"]
        return ["approve"]

    def _write_tracker_status(self, issue: IssueSlice, status: str) -> None:
        path = Path(issue.source_path)
        text = path.read_text(encoding="utf-8")
        if re.search(r"^Status: .*$", text, flags=re.MULTILINE):
            text = re.sub(r"^Status: .*$", f"Status: {status}", text, count=1, flags=re.MULTILINE)
        else:
            text = f"Status: {status}\n{text}"
        path.write_text(text, encoding="utf-8")

    def _record(self, message: str) -> None:
        self.timeline.append(message)

    def _next_action_for_review(self, session_id: str, outcome: str, failure_type: str) -> str:
        if outcome in {"Approved", "Approved with limitations"}:
            if self._session(session_id).issue_id not in self.issues:
                return "complete"
            return "prepare-pr"
        if outcome == "Needs human review":
            return "user-review"
        if outcome == "Needs repair":
            return "same-local-agent-repair"
        if outcome != "Rejected":
            return "record-review"
        if failure_type in {"critical", "security", "merge-risk"}:
            return "user-escalation"
        prior_rejections = sum(1 for review in self.reviews if review.session_id == session_id and review.outcome == "Rejected")
        if failure_type == "architecture" or prior_rejections >= 2:
            return "frontier-architect-revision"
        if prior_rejections == 1:
            return "fresh-local-agent-repair"
        return "same-local-agent-repair"

    def _latest_review_for_session(self, session_id: str) -> ReviewDecision | None:
        for review in reversed(self.reviews):
            if review.session_id == session_id:
                return review
        return None

    def _next_action(self) -> str:
        for issue_id in self.ordered_issue_ids():
            issue = self.issues[issue_id]
            if issue.review_state != "approved" and issue.review_state != "pr-ready":
                return f"Review {issue_id}"
        for issue_id in self.ordered_issue_ids():
            issue = self.issues[issue_id]
            if issue.review_state == "approved":
                return f"Launch {issue_id}"
        return "Prepare PRs or wait for human merge"

    def _evidence_index_lines(self, issue_id: str | None = None) -> list[str]:
        lines: list[str] = []
        for session in self.sessions.values():
            if issue_id and session.issue_id != issue_id:
                continue
            if not session.evidence:
                continue
            links = session.evidence.artifact_links or [f"app-local://{self.project_key}/{session.session_id}/evidence"]
            for link in links:
                lines.append(f"- {session.issue_id} {session.session_id}: {link}")
        return lines

    @staticmethod
    def _write(path: Path, content: str) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def _metadata(text: str) -> dict[str, str]:
    metadata: dict[str, str] = {}
    for line in text.splitlines():
        if not line.strip():
            break
        if ":" in line:
            key, value = line.split(":", 1)
            metadata[key.strip().lower()] = value.strip()
    return metadata


def _record_type(text: str) -> str:
    for line in text.splitlines():
        if line.startswith("## "):
            break
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        if key.strip().lower() == "type":
            return value.strip().lower()
    return ""


def _sections(text: str) -> dict[str, str]:
    sections: dict[str, list[str]] = {}
    current: str | None = None
    for line in text.splitlines():
        if line.startswith("## "):
            current = line.removeprefix("## ").strip()
            sections[current] = []
            continue
        if current:
            sections[current].append(line)
    return {heading: "\n".join(lines).strip() for heading, lines in sections.items()}


def _checklist_items(text: str) -> list[str]:
    items: list[str] = []
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("- [ ] "):
            items.append(stripped.removeprefix("- [ ] ").strip())
        elif stripped.startswith("- "):
            items.append(stripped.removeprefix("- ").strip())
    return items


def _issue_refs(text: str, issues_dir: Path | None = None) -> list[str]:
    if "None - can start immediately" in text:
        return []
    refs: list[str] = []
    for match in re.finditer(r"ISS-(\d+)", text, flags=re.IGNORECASE):
        refs.append(f"ISS-{int(match.group(1)):02d}")
    for match in re.finditer(
        r"(?:^|[/`\s])((\d+)-[^/`\s]+\.md)(?=`|\s|$)",
        text,
        flags=re.IGNORECASE | re.MULTILINE,
    ):
        if issues_dir is not None and not (issues_dir / match.group(1)).exists():
            continue
        refs.append(f"ISS-{int(match.group(2)):02d}")
    deduped: list[str] = []
    for ref in refs:
        if ref not in deduped:
            deduped.append(ref)
    return deduped


def _slug(value: str) -> str:
    value = value.removesuffix(".md").lower()
    value = re.sub(r"[^a-z0-9]+", "-", value).strip("-")
    return value or "issue"


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _normalize_review_outcome(outcome: str) -> str:
    normalized = outcome.strip().lower().replace("_", " ").replace("-", " ")
    known = {
        "approved": "Approved",
        "approved with limitations": "Approved with limitations",
        "needs repair": "Needs repair",
        "needs human review": "Needs human review",
        "rejected": "Rejected",
    }
    if normalized not in known:
        raise AlbertError(f"Unknown review outcome: {outcome}")
    return known[normalized]


_ANSI_SEQUENCE_RE = re.compile(
    r"\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]|\x1b[@-Z\\-_]"
)
_JSON_FENCE_RE = re.compile(r"```(?:json)?\s*(.*?)```", flags=re.IGNORECASE | re.DOTALL)


def _clean_model_output(output: str) -> str:
    text = output.replace("\r\n", "\n").replace("\r", "\n")
    text = _ANSI_SEQUENCE_RE.sub("", text)
    return re.sub(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]", "", text)


def _validate_model_file_plan(data: Any) -> dict[str, Any]:
    if not isinstance(data, dict):
        raise AlbertError("model output must be a JSON object")
    files = data.get("files")
    if not isinstance(files, list) or not files:
        raise AlbertError("model output must include a non-empty files list")
    parsed_files: list[dict[str, str]] = []
    for index, item in enumerate(files, start=1):
        if not isinstance(item, dict):
            raise AlbertError(f"files[{index}] must be an object")
        path = str(item.get("path", "")).strip()
        content = item.get("content")
        if not path:
            raise AlbertError(f"files[{index}] is missing path")
        if not isinstance(content, str):
            raise AlbertError(f"files[{index}] is missing string content")
        parsed_files.append({"path": path, "content": content})
    return {"summary": str(data.get("summary", "")), "files": parsed_files}


def _balanced_json_candidates(text: str) -> list[str]:
    candidates: list[str] = []
    start: int | None = None
    depth = 0
    in_string = False
    escaped = False
    for index, char in enumerate(text):
        if in_string:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                in_string = False
            continue
        if char == '"':
            in_string = True
        elif char == "{":
            if depth == 0:
                start = index
            depth += 1
        elif char == "}" and depth:
            depth -= 1
            if depth == 0 and start is not None:
                candidates.append(text[start : index + 1])
                start = None
    return candidates


def _validate_delegation_decision(data: Any) -> dict[str, Any]:
    if not isinstance(data, dict):
        raise AlbertError("delegation output must be a JSON object")
    complexity = str(data.get("complexity", "")).strip().lower()
    if complexity not in {"low", "medium", "high", "architectural"}:
        raise AlbertError("delegation output must include complexity: low, medium, high, or architectural")
    recommended_agent = str(data.get("recommended_agent", "")).strip()
    if not recommended_agent:
        raise AlbertError("delegation output is missing recommended_agent")
    reason = str(data.get("reason", "")).strip()
    if not reason:
        raise AlbertError("delegation output is missing reason")
    requires_approval = bool(data.get("requires_cloud_approval", data.get("requires_approval", False)))
    return {
        "complexity": complexity,
        "recommended_agent": recommended_agent,
        "requires_approval": requires_approval,
        "reason": reason,
    }


def _load_valid_delegation_decision(candidate: str) -> dict[str, Any] | None:
    try:
        data = json.loads(candidate.strip())
        return _validate_delegation_decision(data)
    except (json.JSONDecodeError, AlbertError):
        return None


def _parse_delegation_decision(output: str) -> dict[str, Any]:
    text = _clean_model_output(output).strip()
    if not text:
        raise AlbertError("router returned empty output")
    try:
        data = json.loads(text)
        return _validate_delegation_decision(data)
    except json.JSONDecodeError:
        pass

    for match in reversed(list(_JSON_FENCE_RE.finditer(text))):
        decision = _load_valid_delegation_decision(match.group(1))
        if decision:
            return decision

    for candidate in reversed(_balanced_json_candidates(text)):
        decision = _load_valid_delegation_decision(candidate)
        if decision:
            return decision

    if "{" not in text or "}" not in text:
        raise AlbertError("router output is not JSON")
    try:
        json.loads(text)
    except json.JSONDecodeError as exc:
        raise AlbertError(f"router output is not valid JSON: {exc}") from exc
    raise AlbertError("router output must include complexity, recommended_agent, and reason")


def _load_valid_model_file_plan(candidate: str) -> dict[str, Any] | None:
    try:
        data = json.loads(candidate.strip())
        return _validate_model_file_plan(data)
    except (json.JSONDecodeError, AlbertError):
        return None


def _parse_model_file_plan(output: str) -> dict[str, Any]:
    text = _clean_model_output(output).strip()
    if not text:
        raise AlbertError("model returned empty output")
    try:
        data = json.loads(text)
        return _validate_model_file_plan(data)
    except json.JSONDecodeError:
        pass

    for match in reversed(list(_JSON_FENCE_RE.finditer(text))):
        plan = _load_valid_model_file_plan(match.group(1))
        if plan:
            return plan

    for candidate in reversed(_balanced_json_candidates(text)):
        plan = _load_valid_model_file_plan(candidate)
        if plan:
            return plan

    if "{" not in text or "}" not in text:
        raise AlbertError("model output is not JSON")
    try:
        json.loads(text)
    except json.JSONDecodeError as exc:
        raise AlbertError(f"model output is not valid JSON: {exc}") from exc
    raise AlbertError("model output must include a non-empty files list")
