from __future__ import annotations

from dataclasses import dataclass

from .core import AlbertError, AlbertMission


@dataclass(frozen=True)
class TuiBoardRow:
    issue_id: str
    title: str
    lifecycle: str
    readiness: str
    assigned_agent: str
    blockers: list[str]
    next_actions: list[str]


@dataclass(frozen=True)
class TuiIssueDetail:
    issue_id: str
    title: str
    tracker_status: str
    runtime_status: str
    review_state: str
    assigned_agent: str
    notes: str
    acceptance_criteria: list[str]
    blocked_by: list[str]
    next_actions: list[str]
    available_agents: list[str]


@dataclass(frozen=True)
class TuiReviewItem:
    session_id: str
    issue_id: str
    status: str
    evidence_valid: bool
    artifact_links: list[str]


@dataclass(frozen=True)
class TuiPrItem:
    issue_id: str
    ready: bool
    reason: str
    merge_approved: bool = False


@dataclass(frozen=True)
class TuiBoardState:
    title: str
    mission_id: str
    next_action: str
    rows: list[TuiBoardRow]
    selected: TuiIssueDetail
    review_queue: list[TuiReviewItem]
    pr_queue: list[TuiPrItem]


@dataclass(frozen=True)
class TuiActionResult:
    action: str
    issue_id: str
    message: str
    session_id: str = ""
    next_action: str = ""
    branch_name: str = ""
    body: str = ""
    create_command: str = ""
    merge_approved: bool = False


def build_tui_state(mission: AlbertMission, selected_issue_id: str | None = None) -> TuiBoardState:
    rows: list[TuiBoardRow] = []
    ordered = mission.ordered_issue_ids()
    for issue_id in ordered:
        detail = mission.issue_detail(issue_id)
        blockers_waiting = [blocker["issue_id"] for blocker in detail["blockers"] if not blocker["satisfied"]]
        rows.append(
            TuiBoardRow(
                issue_id=issue_id,
                title=detail["title"],
                lifecycle=detail["review_state"],
                readiness="blocked" if blockers_waiting else "ready",
                assigned_agent=detail["assigned_agent"],
                blockers=[blocker["issue_id"] for blocker in detail["blockers"]],
                next_actions=detail["next_actions"],
            )
        )
    selected_id = selected_issue_id if selected_issue_id in mission.issues else ordered[0]
    selected_detail = _issue_detail_from_dict(mission.issue_detail(selected_id))
    return TuiBoardState(
        title=mission.prd_title,
        mission_id=mission.mission_id,
        next_action=_recommended_action(rows),
        rows=rows,
        selected=selected_detail,
        review_queue=_review_queue(mission),
        pr_queue=_pr_queue(mission),
    )


def render_tui_state(state: TuiBoardState) -> str:
    width = 92
    lines = [
        "ALBERT // MISSION CONTROL",
        f"MISSION {state.mission_id} :: {state.title}",
        f"NEXT: {state.next_action}",
        "=" * width,
        f"{'ID':<8} {'STATE':<18} {'READY':<8} {'AGENT':<22} BLOCKERS",
        "-" * width,
    ]
    for row in state.rows:
        readiness = row.readiness.upper()
        blockers = ", ".join(row.blockers) if row.blockers else "clear"
        lines.append(f"{row.issue_id:<8} {row.lifecycle:<18} {readiness:<8} {row.assigned_agent:<22} {blockers}")
    lines.extend(
        [
            "=" * width,
            f"DETAIL // {state.selected.issue_id} :: {state.selected.title}",
            f"Tracker: {state.selected.tracker_status} | Runtime: {state.selected.runtime_status} | Review: {state.selected.review_state}",
            f"Agent: {state.selected.assigned_agent}",
            f"Available agents: {', '.join(state.selected.available_agents) or 'metadata fallback'}",
            f"Next actions: {', '.join(state.selected.next_actions)}",
            "Acceptance criteria:",
        ]
    )
    lines.extend(f"- {item}" for item in state.selected.acceptance_criteria)
    if state.selected.blocked_by:
        lines.append(f"Blocked by: {', '.join(state.selected.blocked_by)}")
    else:
        lines.append("Blocked by: clear")
    if state.selected.notes:
        lines.append(f"Notes: {state.selected.notes}")
    if state.review_queue:
        lines.extend(["=" * width, "REVIEW QUEUE"])
        for item in state.review_queue:
            marker = "valid evidence" if item.evidence_valid else "evidence missing"
            lines.append(f"- {item.session_id}: {item.issue_id} {item.status} ({marker})")
    if state.pr_queue:
        lines.extend(["=" * width, "PR READINESS"])
        for item in state.pr_queue:
            marker = "ready" if item.ready else "blocked"
            lines.append(f"- {item.issue_id}: {marker} ({item.reason})")
    return "\n".join(lines) + "\n"


def render_tui_error(message: str) -> str:
    return "\n".join(
        [
            "ALBERT // MISSION CONTROL",
            "TUI cannot load mission state",
            "=" * 92,
            message,
            "Check the tracker directory, PRD, and issue markdown files, then reload.",
            "",
        ]
    )


def perform_tui_action(
    mission: AlbertMission,
    action: str,
    issue_id: str,
    *,
    agent_id: str = "",
    notes: str = "",
    allowed_paths: list[str] | None = None,
    session_id: str = "",
    outcome: str = "",
    reason: str = "",
    failure_type: str = "",
    expected_revision: int | None = None,
    gh_available: bool = False,
) -> TuiActionResult:
    if action == "assign":
        if not agent_id:
            raise AlbertError("TUI assign action requires --agent.")
        mission.assign_issue(issue_id, agent_id, notes=notes)
        return TuiActionResult(action=action, issue_id=issue_id, message=f"{issue_id} assigned to {agent_id}.")
    if action == "approve":
        mission.approve_issue(issue_id)
        return TuiActionResult(action=action, issue_id=issue_id, message=f"{issue_id} approved and locked.")
    if action == "launch":
        session = mission.launch_issue(issue_id, allowed_paths=allowed_paths or [])
        return TuiActionResult(
            action=action,
            issue_id=issue_id,
            message=f"Launched {issue_id} as {session.session_id}.",
            session_id=session.session_id,
        )
    if action == "repair":
        if not session_id:
            raise AlbertError("TUI repair action requires --session.")
        if expected_revision is None:
            raise AlbertError(
                "TUI repair action requires --expected-session-revision."
            )
        session = mission.launch_repair(
            session_id,
            agent_id=agent_id,
            allowed_paths=allowed_paths if allowed_paths else None,
            expected_revision=expected_revision,
        )
        return TuiActionResult(
            action=action,
            issue_id=session.issue_id,
            message=f"Launched repair for {session.issue_id} as {session.session_id}.",
            session_id=session.session_id,
        )
    if action == "review":
        if not session_id:
            raise AlbertError("TUI review action requires --session.")
        if not outcome:
            raise AlbertError("TUI review action requires --outcome.")
        if not reason:
            raise AlbertError("TUI review action requires --reason.")
        if expected_revision is None:
            raise AlbertError(
                "TUI review action requires --expected-session-revision."
            )
        decision = mission.record_frontier_review(
            session_id,
            outcome,
            reason=reason,
            failure_type=failure_type,
            expected_revision=expected_revision,
        )
        return TuiActionResult(
            action=action,
            issue_id=decision.issue_id,
            message=f"{decision.issue_id} review: {decision.outcome}; next action {decision.next_action}.",
            session_id=session_id,
            next_action=decision.next_action,
        )
    if action == "prepare-pr":
        pr = mission.prepare_pr(issue_id, gh_available=gh_available)
        return TuiActionResult(
            action=action,
            issue_id=issue_id,
            message=f"{issue_id} PR instructions prepared.",
            branch_name=pr.branch_name,
            body=pr.body,
            create_command=pr.create_command,
            merge_approved=pr.merge_approved,
        )
    raise AlbertError(f"Unknown TUI action: {action}")


def _issue_detail_from_dict(detail: dict[str, object]) -> TuiIssueDetail:
    return TuiIssueDetail(
        issue_id=str(detail["issue_id"]),
        title=str(detail["title"]),
        tracker_status=str(detail["tracker_status"]),
        runtime_status=str(detail["runtime_status"]),
        review_state=str(detail["review_state"]),
        assigned_agent=str(detail["assigned_agent"]),
        notes=str(detail["notes"]),
        acceptance_criteria=list(detail["acceptance_criteria"]),
        blocked_by=[str(blocker["issue_id"]) for blocker in detail["blockers"]],
        next_actions=list(detail["next_actions"]),
        available_agents=list(detail["available_agents"]),
    )


def _recommended_action(rows: list[TuiBoardRow]) -> str:
    for action in ["approve-delegation", "route", "launch", "approve", "prepare-pr", "reopen", "record-review"]:
        for row in rows:
            if row.readiness == "ready" and action in row.next_actions:
                return f"{_label(action)} {row.issue_id}"
    for row in rows:
        if row.next_actions:
            return f"{_label(row.next_actions[0])} {row.issue_id}"
    return "No action available"


def _review_queue(mission: AlbertMission) -> list[TuiReviewItem]:
    queue: list[TuiReviewItem] = []
    for session in mission.sessions.values():
        if session.status == "reviewed":
            continue
        if not session.evidence_valid:
            continue
        artifact_links = mission.review_artifact_links(session)
        queue.append(
            TuiReviewItem(
                session_id=session.session_id,
                issue_id=session.issue_id,
                status=session.status,
                evidence_valid=session.evidence_valid,
                artifact_links=artifact_links,
            )
        )
    return queue


def _pr_queue(mission: AlbertMission) -> list[TuiPrItem]:
    result: list[TuiPrItem] = []
    for issue_id in mission.ordered_issue_ids():
        ready = any(
            review.issue_id == issue_id and review.outcome in {"Approved", "Approved with limitations"}
            for review in mission.reviews
        )
        if ready:
            result.append(TuiPrItem(issue_id=issue_id, ready=True, reason="PR-ready"))
        else:
            result.append(TuiPrItem(issue_id=issue_id, ready=False, reason=f"{issue_id} is not PR-ready."))
    return result


def _label(action: str) -> str:
    return {
        "approve": "Approve",
        "approve-delegation": "Approve delegation for",
        "launch": "Launch",
        "prepare-pr": "Prepare PR for",
        "route": "Route",
        "reopen": "Reopen",
        "record-review": "Record review for",
    }.get(action, action.replace("-", " ").title())
