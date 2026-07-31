import type {
  WorkspaceIssueSessionDetail,
  WorkspaceIssueEvidenceSummary,
  WorkspaceIssueSliceSummary,
  WorkspaceMissionSummary,
  WorkspaceQueueDecision,
  WorkspaceQueueItem,
  WorkspaceQueueProjection,
  WorkspaceQueueAttention,
  WorkspaceSnapshot,
  ReviewDecision,
} from "./contracts";

export type WorkstationCardStatus =
  | "queued"
  | "thinking"
  | "running"
  | "idle"
  | "waiting-approval"
  | "blocked"
  | "reviewing"
  | "review-ready"
  | "done"
  | "failed";

export interface WorkstationPendingIntent {
  readonly id: string;
  readonly label: string;
  readonly expectedRevision: number;
}

export interface WorkstationToolActivity {
  readonly kind: "command-summary" | "operation-summary" | "failure-summary" | "queue-summary";
  readonly label: string;
  readonly summary: string;
}

export interface WorkstationFileDetail {
  readonly path: string;
  readonly status: string;
}

export interface WorkstationDiffLink {
  readonly label: string;
  readonly path: string;
  readonly href: string;
  readonly missionId: string;
  readonly cardId: string;
  readonly sessionId: string;
}

export interface WorkstationEvidenceLink {
  readonly label: string;
  readonly href: string;
  readonly sessionId: string;
}

export interface WorkstationTerminalExcerpt {
  readonly label: string;
  readonly excerpt: string;
  readonly sessionId: string;
}

export interface WorkstationReviewState {
  readonly evidenceState: string;
  readonly lifecycle: string;
  readonly risks: string;
  readonly reviewReady: boolean;
}

export interface WorkstationGovernedAction {
  readonly label: string;
  readonly target:
    | "workspace-queue"
    | "review-workspace"
    | "mission-board"
    | "shell-terminal"
    | "agent-console"
    | "activity"
    | "none";
  readonly requiresReason: boolean;
  readonly actionType?:
    | "workspace-queue-decision"
    | "issue-approve"
    | "issue-launch"
    | "issue-retry"
    | "session-cancel"
    | "path-grant-decision"
    | "review-decision"
    | "model-assignment-change"
    | "mission-draft-decision"
    | "conversation-scope-change";
  readonly actor?: "mission-commander";
  readonly missionId?: string;
  readonly itemId?: string;
  readonly issueId?: string;
  readonly sessionId?: string;
  readonly decision?: WorkspaceQueueDecision;
  readonly reviewDecision?: ReviewDecision;
  readonly expectedRevision?: number;
  readonly disabledReason?: string;
  readonly recoveryPath?: string;
  readonly targetIdentity?: {
    readonly kind:
      | "workspace-queue-item"
      | "issue-slice"
      | "agent-session"
      | "path-grant"
      | "mission-draft"
      | "conversation-scope";
    readonly id: string;
  };
}

export interface WorkstationCardDetail {
  readonly originatingSessionId: string | null;
  readonly issueId: string | null;
  readonly toolActivity: readonly WorkstationToolActivity[];
  readonly filesTouched: readonly WorkstationFileDetail[];
  readonly diffs: readonly WorkstationDiffLink[];
  readonly evidenceLinks: readonly WorkstationEvidenceLink[];
  readonly terminalExcerpts: readonly WorkstationTerminalExcerpt[];
  readonly reviewState: WorkstationReviewState;
  readonly governedActions: readonly WorkstationGovernedAction[];
}

export interface WorkstationCardProjection {
  readonly id: string;
  readonly missionId: string;
  readonly missionTitle: string;
  readonly name: string;
  readonly sessionId: string | null;
  readonly issueId: string | null;
  readonly model: string;
  readonly role: string;
  readonly currentTask: string;
  readonly status: WorkstationCardStatus;
  readonly phase: string;
  readonly progress: string;
  readonly lastActivity: string;
  readonly approvalBlockers: readonly string[];
  readonly filesTouched: number;
  readonly latestCommandOrTest: string;
  readonly nextAction: string;
  readonly acceptedRevision: number;
  readonly attention: boolean;
  readonly tone: "attention" | "active" | "failed" | "muted";
  readonly detail: WorkstationCardDetail;
}

export interface WorkstationCardGroup {
  readonly id: string;
  readonly bucket: "active" | "done";
  readonly scopeId: string | null;
  readonly label: string;
  readonly cards: readonly WorkstationCardProjection[];
}

export interface WorkstationProjection {
  readonly revision: number;
  readonly groups: readonly WorkstationCardGroup[];
  readonly pendingIntent: WorkstationPendingIntent | null;
}

export type IssueAssignmentRowState =
  | "needs-review"
  | "unassigned-ready"
  | "assigned"
  | "blocked"
  | "active"
  | "review-ready"
  | "complete"
  | "merged"
  | "failed";

export type IssueAssignmentState = "unassigned" | "assigned" | "active";

export type IssueAssignmentBlockerState = "clear" | "blocked" | "satisfied";

export interface IssueAssignmentBoardRow {
  readonly issueId: string;
  readonly title: string;
  readonly owner: string;
  readonly assignmentState: IssueAssignmentState;
  readonly state: IssueAssignmentRowState;
  readonly lifecycleState: string;
  readonly readinessState: string;
  readonly blockerState: IssueAssignmentBlockerState;
  readonly blockerSummaries: readonly string[];
  readonly workstationSessionId: string | null;
  readonly workstationAgent: string | null;
  readonly workstationStatus: string | null;
  readonly scope: {
    readonly kind: "issue-slice";
    readonly target_id: string;
    readonly label: string;
    readonly mission_id?: string | null;
  };
  readonly scopeDisabledReason: string | null;
  readonly governedActions: readonly WorkstationGovernedAction[];
}

export interface IssueAssignmentBoardProjection {
  readonly revision: number;
  readonly rows: readonly IssueAssignmentBoardRow[];
}

interface ProjectWorkstationOptions {
  readonly pendingIntent?: WorkstationPendingIntent | null;
  readonly workspaceQueue?: WorkspaceQueueProjection | null;
}

export function projectWorkstationCards(
  snapshot: WorkspaceSnapshot,
  options: ProjectWorkstationOptions = {},
): WorkstationProjection {
  const activeMissionId = snapshot.active_mission?.id ?? null;
  const issueSlices = new Map(
    snapshot.mission_board.issue_slices?.map((issue) => [
      missionIssueKey(activeMissionId, issue.issue_id),
      issue,
    ]),
  );
  const cards =
    snapshot.missions?.flatMap((mission) => [
      ...mission.attention.map((attention) =>
        projectAttentionCard(snapshot, mission, attention, options.workspaceQueue ?? null),
      ),
      ...mission.sessions.map((session) => {
        const issue = issueSlices.get(missionIssueKey(mission.id, session.issue_id));
        const detail = issue?.sessions.find((item) => item.session_id === session.session_id);
        return projectSessionCard(snapshot, mission, session, issue, detail);
      }),
    ]) ?? [];

  const groups = groupCards(cards);
  return {
    revision: snapshot.revision,
    groups,
    pendingIntent: options.pendingIntent ?? null,
  };
}

function missionIssueKey(missionId: string | null, issueId: string): string {
  return JSON.stringify([missionId, issueId]);
}

export function projectIssueAssignmentBoard(snapshot: WorkspaceSnapshot): IssueAssignmentBoardProjection {
  const issueSlices = new Map(
    snapshot.mission_board.issue_slices?.map((issue) => [issue.issue_id, issue]),
  );
  const rows = snapshot.mission_board.ordered_issue_ids.flatMap((issueId) => {
    const issue = issueSlices.get(issueId);
    if (issue) {
      return isActiveAfkAssignmentIssue(issue)
        ? [projectIssueAssignmentRow(snapshot, issue)]
        : [];
    }
    return [];
  });

  return {
    revision: snapshot.revision,
    rows,
  };
}

const TERMINAL_TRACKER_STATUSES = new Set([
  "canceled",
  "cancelled",
  "closed",
  "complete",
  "completed",
  "done",
  "merged",
  "rejected",
  "wont-fix",
  "wontfix",
]);

function isActiveAfkAssignmentIssue(issue: WorkspaceIssueSliceSummary): boolean {
  const workType = issue.work_type?.trim().toLowerCase() ?? "";
  if (workType !== "afk") return false;
  const trackerStatus = (issue.tracker_status ?? "").trim().toLowerCase();
  if (!trackerStatus) return false;
  if (trackerStatus === "ready-for-human" || trackerStatus === "needs-human-review") {
    return false;
  }
  if (TERMINAL_TRACKER_STATUSES.has(trackerStatus)) return false;
  const currentSession = latestSession(issue.sessions);
  if (
    currentSession &&
    canonicalStatus(
      currentSession.status,
      currentSession.operation_status,
      currentSession.failure,
    ) !== "done"
  ) {
    return true;
  }
  const lifecycle = issue.lifecycle.trim().toLowerCase();
  return !lifecycle.includes("complete") && !lifecycle.includes("merged");
}

function projectIssueAssignmentRow(
  snapshot: WorkspaceSnapshot,
  issue: WorkspaceIssueSliceSummary,
): IssueAssignmentBoardRow {
  const ownerMissionId = snapshot.active_mission?.id ?? null;
  const missionSession = missionSessionForIssue(snapshot, issue.issue_id, ownerMissionId);
  const issueSession =
    (missionSession
      ? issue.sessions.find((session) => session.session_id === missionSession.session_id)
      : null) ?? latestSession(issue.sessions);
  const workstationStatus = canonicalStatus(
    issueSession?.status ?? missionSession?.status ?? "",
    issueSession?.operation_status ?? issue.model_assignment.operation_status,
    issueSession?.failure ?? issue.model_assignment.failure,
  );
  const owner =
    missionSession?.assigned_agent ||
    issueSession?.assigned_agent ||
    issue.model_assignment.agent_id ||
    "Unassigned";
  const assignmentState = assignmentStateForIssue(owner, missionSession, issueSession);
  const state = issueAssignmentRowState(issue, workstationStatus, assignmentState);
  const workstationSessionId = missionSession?.session_id ?? issueSession?.session_id ?? null;
  return {
    issueId: issue.issue_id,
    title: issue.title,
    owner,
    assignmentState,
    state,
    lifecycleState: issue.lifecycle,
    readinessState: readinessLabel(state),
    blockerState: blockerState(issue.blockers),
    blockerSummaries: issue.blockers.map(blockerSummary),
    workstationSessionId,
    workstationAgent: missionSession?.assigned_agent ?? issueSession?.assigned_agent ?? null,
    workstationStatus: (missionSession?.status ?? issueSession?.status) || null,
    scope: {
      kind: "issue-slice",
      target_id: issue.issue_id,
      label: issue.title,
      mission_id: ownerMissionId,
    },
    scopeDisabledReason: scopeDisabledReason(snapshot, issue.issue_id, ownerMissionId),
    governedActions: issueAssignmentGovernedActions(
      issue,
      state,
      assignmentState,
      workstationSessionId,
      snapshot.revision,
      ownerMissionId ?? undefined,
    ),
  };
}

function issueAssignmentGovernedActions(
  issue: WorkspaceIssueSliceSummary,
  state: IssueAssignmentRowState,
  assignmentState: IssueAssignmentState,
  workstationSessionId: string | null,
  expectedRevision: number,
  missionId?: string,
): readonly WorkstationGovernedAction[] {
  if (workstationSessionId || issue.blockers.some((blocker) => !blocker.satisfied)) {
    return [];
  }
  const trackerStatus = issue.tracker_status?.toLowerCase() ?? "";
  const lifecycle = issue.lifecycle.toLowerCase();
  if (state === "needs-review") {
    return trackerStatus === "ready-for-agent"
      ? [
          {
            label: "Approve for launch",
            target: "mission-board",
            requiresReason: false,
            actionType: "issue-approve",
            actor: "mission-commander",
            missionId,
            issueId: issue.issue_id,
            expectedRevision,
            targetIdentity: { kind: "issue-slice", id: issue.issue_id },
            recoveryPath: "Refresh the Issue Assignment Board and approve the current Issue Slice state.",
          },
        ]
      : [];
  }

  const launchable =
    issue.launch_eligible && (state === "unassigned-ready" || state === "assigned");
  const assignmentManageable =
    launchable || (trackerStatus === "ready-for-agent" && lifecycle.includes("approved"));
  if (!launchable && !assignmentManageable) return [];

  const actions: WorkstationGovernedAction[] = [];
  if (launchable) {
    actions.push({
      label: "Launch",
      target: "mission-board",
      requiresReason: false,
      actionType: "issue-launch",
      actor: "mission-commander",
      missionId,
      issueId: issue.issue_id,
      expectedRevision,
      targetIdentity: { kind: "issue-slice", id: issue.issue_id },
      recoveryPath: "Refresh the Issue Assignment Board and retry from the current Issue Slice state.",
    });
  }
  if (assignmentManageable) {
    actions.push({
      label: assignmentState === "assigned" ? "Change model assignment" : "Assign model",
      target: "mission-board",
      requiresReason: true,
      actionType: "model-assignment-change",
      actor: "mission-commander",
      missionId,
      issueId: issue.issue_id,
      expectedRevision,
      targetIdentity: { kind: "issue-slice", id: issue.issue_id },
      recoveryPath: "Provide the target agent id and reason, then retry from the acknowledged row state.",
    });
  }
  return actions;
}

function missionSessionForIssue(
  snapshot: WorkspaceSnapshot,
  issueId: string,
  missionId: string | null,
): WorkspaceMissionSummary["sessions"][number] | null {
  const mission = (snapshot.missions ?? []).find((candidate) => candidate.id === missionId);
  return latestSession(
    mission?.sessions.filter((candidate) => candidate.issue_id === issueId) ?? [],
  );
}

function latestSession<T extends { readonly session_id: string }>(
  sessions: readonly T[],
): T | null {
  return sessions.reduce<T | null>((latest, candidate) => {
    if (latest === null) return candidate;
    const latestSequence = sessionSequence(latest.session_id);
    const candidateSequence = sessionSequence(candidate.session_id);
    return candidateSequence >= latestSequence ? candidate : latest;
  }, null);
}

function sessionSequence(sessionId: string): number {
  const match = sessionId.match(/-(\d+)$/);
  return match ? Number.parseInt(match[1], 10) : -1;
}

function assignmentStateForIssue(
  owner: string,
  missionSession: WorkspaceMissionSummary["sessions"][number] | null,
  issueSession: WorkspaceIssueSessionDetail | null,
): IssueAssignmentState {
  if (missionSession || issueSession) return "active";
  return owner === "Unassigned" ? "unassigned" : "assigned";
}

function issueAssignmentRowState(
  issue: WorkspaceIssueSliceSummary,
  status: WorkstationCardStatus,
  assignmentState: IssueAssignmentState,
): IssueAssignmentRowState {
  const lifecycle = issue.lifecycle.toLowerCase();
  if (status === "failed" || lifecycle.includes("failed")) return "failed";
  if (lifecycle.includes("merged")) return "merged";
  if (
    status === "done" ||
    lifecycle.includes("complete") ||
    issue.evidence.state === "accepted"
  ) {
    return "complete";
  }
  if (status === "review-ready") return "review-ready";
  if (issue.blockers.some((blocker) => !blocker.satisfied)) return "blocked";
  if (lifecycle.includes("needs review") || lifecycle.includes("needs-review")) {
    return "needs-review";
  }
  if (assignmentState === "active") return "active";
  if (issue.launch_eligible && assignmentState === "unassigned") return "unassigned-ready";
  if (assignmentState === "assigned") return "assigned";
  return issue.launch_eligible ? "unassigned-ready" : "blocked";
}

function blockerState(
  blockers: readonly WorkspaceIssueSliceSummary["blockers"][number][],
): IssueAssignmentBlockerState {
  if (blockers.some((blocker) => !blocker.satisfied)) return "blocked";
  return blockers.length > 0 ? "satisfied" : "clear";
}

function blockerSummary(blocker: WorkspaceIssueSliceSummary["blockers"][number]): string {
  return `${blocker.issue_id} ${blocker.lifecycle} ${blocker.satisfied ? "satisfied" : "open"} - ${blocker.title}`;
}

function readinessLabel(state: IssueAssignmentRowState): string {
  const labels: Record<IssueAssignmentRowState, string> = {
    "needs-review": "Needs review",
    "unassigned-ready": "Ready",
    assigned: "Assigned",
    blocked: "Blocked",
    active: "Active",
    "review-ready": "Review ready",
    complete: "Complete",
    merged: "Merged",
    failed: "Failed",
  };
  return labels[state];
}

function scopeDisabledReason(
  snapshot: WorkspaceSnapshot,
  issueId: string,
  missionId: string | null,
): string | null {
  const scopedMissionId = snapshot.conversation_scope.mission_id ?? snapshot.active_mission?.id ?? null;
  return snapshot.conversation_scope.kind === "issue-slice" &&
    snapshot.conversation_scope.target_id === issueId &&
    scopedMissionId === missionId
    ? `Conversation Scope already targets ${issueId}.`
    : null;
}

function projectAttentionCard(
  snapshot: WorkspaceSnapshot,
  mission: WorkspaceMissionSummary,
  attention: WorkspaceQueueAttention,
  workspaceQueue: WorkspaceQueueProjection | null,
): WorkstationCardProjection {
  const queueItem = pendingQueueItemForAttention(workspaceQueue, attention);
  return {
    id: `attention:${mission.id}:${attention.attention_id}`,
    missionId: mission.id,
    missionTitle: mission.title,
    name: attention.label,
    sessionId: null,
    issueId: null,
    model: "orchestrator",
    role: "governance",
    currentTask: attention.kind,
    status: "waiting-approval",
    phase: "Approval",
    progress: "Workspace Queue item pending",
    lastActivity: "",
    approvalBlockers: [attention.label],
    filesTouched: 0,
    latestCommandOrTest: "No command or test summary",
    nextAction: "Open Workspace Queue",
    acceptedRevision: snapshot.revision,
    attention: true,
    tone: "attention",
    detail: {
      originatingSessionId: null,
      issueId: null,
      toolActivity: [
        {
          kind: "queue-summary",
          label: "Workspace Queue",
          summary: `${attention.kind}: ${attention.label}`,
        },
      ],
      filesTouched: [],
      diffs: [],
      evidenceLinks: [],
      terminalExcerpts: [],
      reviewState: {
        evidenceState: "not-applicable",
        lifecycle: "Waiting approval",
        risks: "No evidence package attached to this queue item.",
        reviewReady: false,
      },
      governedActions: queueItem
        ? workspaceQueueDecisionActions(queueItem)
        : [
            {
              label: "Open Workspace Queue",
              target: "workspace-queue",
              requiresReason: false,
            },
          ],
    },
  };
}

function pendingQueueItemForAttention(
  workspaceQueue: WorkspaceQueueProjection | null,
  attention: WorkspaceQueueAttention,
): WorkspaceQueueItem | null {
  if (!workspaceQueue) return null;
  const itemId = attention.queue_link.split("#").at(1);
  if (!itemId) return null;
  const item = workspaceQueue.items.find((candidate) => candidate.item_id === itemId);
  return item?.status === "pending" ? item : null;
}

function workspaceQueueDecisionActions(
  item: WorkspaceQueueItem,
): readonly WorkstationGovernedAction[] {
  return [
    { decision: "approve" as const, label: "Approve", requiresReason: false },
    { decision: "reject" as const, label: "Reject", requiresReason: true },
    { decision: "defer" as const, label: "Defer", requiresReason: true },
  ].map((action) => ({
    ...action,
    target: "workspace-queue" as const,
    actionType: "workspace-queue-decision" as const,
    actor: "mission-commander" as const,
    missionId: item.mission_id,
    itemId: item.item_id,
    targetIdentity: {
      kind: "workspace-queue-item" as const,
      id: item.item_id,
    },
  }));
}

function projectSessionCard(
  snapshot: WorkspaceSnapshot,
  mission: WorkspaceMissionSummary,
  session: WorkspaceMissionSummary["sessions"][number],
  issue: WorkspaceIssueSliceSummary | undefined,
  detail: WorkspaceIssueSessionDetail | undefined,
): WorkstationCardProjection {
  const cardId = workstationSessionCardId(mission.id, session.session_id);
  const operationStatus = detail?.operation_status || session.operation_status || "";
  const failureSummary = detail?.failure || session.failure || "";
  const status = canonicalStatus(session.status, operationStatus, failureSummary);
  const evidence = issue?.evidence ?? missionSessionEvidence(session);
  const latestCommandOrTest = latestEvidenceLine(evidence);
  const progress = issue?.progress ?? (operationStatus || session.status);
  const failure = failureSummary ? [failureSummary] : [];
  const repairActionAvailable = Boolean(session.repair_action_available);
  const attention = repairActionAvailable || status === "blocked" || status === "failed";
  return {
    id: cardId,
    missionId: mission.id,
    missionTitle: mission.title,
    name: session.assigned_agent,
    sessionId: session.session_id,
    issueId: session.issue_id,
    model: detail?.model || session.model || issue?.model_assignment.model || session.assigned_agent,
    role: detail?.role || session.role || issue?.model_assignment.role || "agent",
    currentTask: issue?.title ?? (session.task_title || session.issue_id),
    status,
    phase: operationStatus || session.status,
    progress,
    lastActivity: session.last_activity_at ?? "",
    approvalBlockers: failure,
    filesTouched: evidence.changed_files.length,
    latestCommandOrTest,
    nextAction: repairActionAvailable
      ? "Launch repair"
      : nextAction(status, progress, detail?.failure),
    acceptedRevision: snapshot.revision,
    attention,
    tone: toneForStatus(status),
    detail: sessionDetail(
      status,
      mission.id,
      cardId,
      session.session_id,
      session.issue_id,
      issue,
      detail,
      evidence,
      snapshot.revision,
      repairActionAvailable,
      session.review_outcome,
    ),
  };
}

function sessionDetail(
  status: WorkstationCardStatus,
  missionId: string,
  cardId: string,
  sessionId: string,
  issueId: string,
  issue: WorkspaceIssueSliceSummary | undefined,
  detail: WorkspaceIssueSessionDetail | undefined,
  evidence: WorkspaceIssueEvidenceSummary,
  acceptedRevision = 0,
  repairActionAvailable = false,
  reviewOutcome = "",
): WorkstationCardDetail {
  const commands = evidence.commands_run;
  const toolActivity: WorkstationToolActivity[] = [
    ...commands.map((command) => ({
      kind: "command-summary" as const,
      label: "Command",
      summary: command,
    })),
  ];
  if (detail?.operation_status) {
    toolActivity.push({
      kind: "operation-summary",
      label: "Provider operation",
      summary: detail.operation_status,
    });
  }
  if (detail?.failure) {
    toolActivity.push({
      kind: "failure-summary",
      label: "Failure",
      summary: detail.failure,
    });
  }

  const changedFiles = evidence.changed_files;
  const filesTouched = changedFiles.map((path) => ({ path, status: "touched" }));
  const readableArtifactLinks = evidence.artifact_links.filter(
    (href) => href.startsWith("app-local://") || href.startsWith("artifact://evidence/"),
  );
  const reviewDiffArtifact = readableArtifactLinks.find((href) =>
    /(?:^|[/\\])(?:review\.diff|review_diff)$/i.test(href),
  );
  const diffs = reviewDiffArtifact
    ? changedFiles.map((path) => ({
        label: `Diff ${path}`,
        path,
        href: reviewDiffArtifact,
        missionId,
        cardId,
        sessionId,
      }))
    : [];
  const evidenceLinks = readableArtifactLinks.map((href) => ({
    label:
      href === reviewDiffArtifact
        ? `Review diff ${sessionId}`
        : `Evidence Package ${sessionId}`,
    href,
    sessionId,
  }));
  const terminalExcerpts = [
    ...commands.map((command) => ({
      label: "Command summary",
      excerpt: command,
      sessionId,
    })),
    ...(evidence.test_results
      ? [
          {
            label: "Test summary",
            excerpt: evidence.test_results,
            sessionId,
          },
        ]
      : []),
  ];

  return {
    originatingSessionId: sessionId,
    issueId,
    toolActivity,
    filesTouched,
    diffs,
    evidenceLinks,
    terminalExcerpts,
    reviewState: {
      evidenceState: evidence.state,
      lifecycle: issue?.lifecycle ?? (reviewOutcome || status),
      risks: evidence.risks || "No risks recorded.",
      reviewReady: status === "review-ready",
    },
    governedActions: governedActions(
      status,
      sessionId,
      issueId,
      acceptedRevision,
      missionId,
      repairActionAvailable,
    ),
  };
}

function workstationSessionCardId(missionId: string, sessionId: string): string {
  return `session:${missionId}:${sessionId}`;
}

function groupCards(cards: readonly WorkstationCardProjection[]): readonly WorkstationCardGroup[] {
  const active = cards.filter((card) => card.status !== "done");
  const done = cards.filter((card) => card.status === "done");
  const splitByMission = new Set(cards.map((card) => card.missionId)).size > 1 || cards.length > 6;
  return [
    ...bucketGroups("active", "Active Work", active, splitByMission),
    ...bucketGroups("done", "Done", done, splitByMission),
  ];
}

function bucketGroups(
  bucket: "active" | "done",
  label: string,
  cards: readonly WorkstationCardProjection[],
  splitByMission: boolean,
): readonly WorkstationCardGroup[] {
  if (!cards.length) return [];
  if (!splitByMission) {
    return [{ id: bucket, bucket, scopeId: null, label, cards: sortCards(cards) }];
  }
  const missionIds = [...new Set(cards.map((card) => card.missionId))];
  return missionIds.map((missionId) => {
    const scopedCards = cards.filter((card) => card.missionId === missionId);
    const missionTitle = scopedCards[0]?.missionTitle ?? missionId;
    return {
      id: `${bucket}:${missionId}`,
      bucket,
      scopeId: missionId,
      label: `${label} / ${missionTitle}`,
      cards: sortCards(scopedCards),
    };
  });
}

function sortCards(
  cards: readonly WorkstationCardProjection[],
): readonly WorkstationCardProjection[] {
  return [...cards].sort((left, right) => {
    const priority = statusPriority(left.status) - statusPriority(right.status);
    if (priority !== 0) return priority;
    if (left.missionId !== right.missionId) return left.missionTitle.localeCompare(right.missionTitle);
    return left.name.localeCompare(right.name);
  });
}

function statusPriority(status: WorkstationCardStatus): number {
  const priority: Record<WorkstationCardStatus, number> = {
    "waiting-approval": 0,
    blocked: 1,
    failed: 2,
    reviewing: 3,
    queued: 4,
    running: 5,
    idle: 6,
    thinking: 7,
    "review-ready": 8,
    done: 9,
  };
  return priority[status];
}

function toneForStatus(status: WorkstationCardStatus): WorkstationCardProjection["tone"] {
  if (status === "failed") return "failed";
  if (status === "waiting-approval" || status === "blocked") return "attention";
  if (
    status === "queued" ||
    status === "running" ||
    status === "reviewing" ||
    status === "review-ready"
  ) return "active";
  return "muted";
}

function canonicalStatus(
  rawStatus = "",
  rawOperationStatus = "",
  failure = "",
): WorkstationCardStatus {
  const status = `${rawStatus} ${rawOperationStatus}`.toLowerCase();
  if (failure || status.includes("failed") || status.includes("rejected")) return "failed";
  if (status.includes("cancelled") || status.includes("canceled")) return "failed";
  if (status.includes("evidence-ready") || status.includes("awaiting-review")) return "review-ready";
  if (status.includes("waiting") || status.includes("approval") || status.includes("pending")) {
    return "waiting-approval";
  }
  if (status.includes("blocked") || status.includes("needs-repair")) return "blocked";
  if (status.includes("reviewing") || status.includes("needs-review")) return "reviewing";
  if (
    status.includes("reviewed") ||
    status.includes("complete") ||
    status.includes("merged") ||
    status.includes("done")
  ) {
    return "done";
  }
  if (status.includes("queued")) return "queued";
  if (
    status.includes("not-started") ||
    (status.includes("idle") && !status.includes("launched"))
  ) {
    return "idle";
  }
  if (
    status.includes("running") ||
    status.includes("streaming") ||
    status.includes("launched") ||
    status.includes("in-progress")
  ) {
    return "running";
  }
  return "thinking";
}

function missionSessionEvidence(
  session: WorkspaceMissionSummary["sessions"][number],
): WorkspaceIssueEvidenceSummary {
  const changedFiles = session.changed_files ?? [];
  const commandsRun = session.commands_run ?? [];
  const artifactLinks = session.artifact_links ?? [];
  const hasEvidence =
    changedFiles.length > 0 ||
    commandsRun.length > 0 ||
    Boolean(session.test_results) ||
    artifactLinks.length > 0;
  const accepted = /reviewed|complete|done/i.test(session.status);
  return {
    state: hasEvidence ? (accepted ? "accepted" : "ready-for-review") : "missing",
    changed_files: changedFiles,
    commands_run: commandsRun,
    test_results: session.test_results || "No evidence package recorded.",
    risks: session.risks || "None recorded.",
    artifact_links: artifactLinks,
  };
}

function latestEvidenceLine(evidence: WorkspaceIssueEvidenceSummary): string {
  const command = evidence.commands_run.at(-1);
  if (command) return command;
  const tests = evidence.test_results;
  return tests && tests !== "No evidence package recorded."
    ? tests
    : "No command or test summary";
}

function nextAction(
  status: WorkstationCardStatus,
  progress: string,
  failure = "",
): string {
  if (status === "failed") return failure || "Inspect failure evidence";
  if (status === "waiting-approval") return "Resolve approval blocker";
  if (status === "blocked") return progress || "Resolve blocker";
  if (status === "review-ready") return "Open Review Workspace";
  if (status === "done") return "Review accepted evidence";
  if (status === "queued") return "Waiting for the Local Agent runner";
  if (status === "idle") return progress || "Await launch or assignment";
  return progress || "Monitor active work";
}

function governedActions(
  status: WorkstationCardStatus,
  sessionId: string,
  issueId: string,
  expectedRevision: number,
  missionId: string,
  repairActionAvailable = false,
): readonly WorkstationGovernedAction[] {
  if (repairActionAvailable) {
    return [
      {
        label: "Launch repair",
        target: "mission-board",
        requiresReason: false,
        actionType: "issue-retry",
        actor: "mission-commander",
        missionId,
        sessionId,
        issueId,
        expectedRevision,
        recoveryPath: "Reload canonical Mission Work and launch the current Review Workspace repair action.",
        targetIdentity: { kind: "agent-session", id: sessionId },
      },
    ];
  }
  if (status === "waiting-approval") {
    return [{ label: "Open Workspace Queue", target: "workspace-queue", requiresReason: false }];
  }
  if (status === "review-ready") {
    return [
      { label: "Open Review Workspace", target: "review-workspace", requiresReason: false },
      reviewAction("Accept evidence", "accept", missionId, sessionId, issueId, expectedRevision, false),
      reviewAction("Request repair", "repair", missionId, sessionId, issueId, expectedRevision, true),
      reviewAction("Escalate human review", "escalate-human", missionId, sessionId, issueId, expectedRevision, false),
    ];
  }
  if (status === "failed" || status === "blocked") {
    return [
      {
        label: "Retry",
        target: "mission-board",
        requiresReason: true,
        actionType: "issue-retry",
        actor: "mission-commander",
        missionId,
        sessionId,
        issueId,
        expectedRevision,
        recoveryPath: "Open the Review Workspace, provide a repair reason, and submit the repair request.",
        targetIdentity: { kind: "agent-session", id: sessionId },
      },
      reviewAction("Request repair", "repair", missionId, sessionId, issueId, expectedRevision, true),
      reviewAction("Escalate human review", "escalate-human", missionId, sessionId, issueId, expectedRevision, false),
    ];
  }
  if (status === "done") {
    return [{ label: "Open Activity", target: "activity", requiresReason: false }];
  }
  return [
    {
      label: "Cancel session",
      target: "mission-board",
      requiresReason: true,
      actionType: "session-cancel",
      actor: "mission-commander",
      missionId,
      sessionId,
      issueId,
      expectedRevision,
      recoveryPath: "Open the session detail and use an available Orchestrator-backed cancel control.",
      targetIdentity: { kind: "agent-session", id: sessionId },
    },
    { label: "Monitor active work", target: "none", requiresReason: false },
  ];
}

function reviewAction(
  label: string,
  reviewDecision: ReviewDecision,
  missionId: string,
  sessionId: string,
  issueId: string,
  expectedRevision: number,
  requiresReason: boolean,
): WorkstationGovernedAction {
  return {
    label,
    target: "review-workspace",
    requiresReason,
    actionType: "review-decision",
    actor: "mission-commander",
    missionId,
    sessionId,
    issueId,
    reviewDecision,
    expectedRevision,
    targetIdentity: { kind: "agent-session", id: sessionId },
    recoveryPath: "Refresh the Review Workspace and retry against the current evidence state.",
  };
}
