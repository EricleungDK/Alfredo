import type {
  WorkspaceIssueSessionDetail,
  WorkspaceIssueSliceSummary,
  WorkspaceMissionSummary,
  WorkspaceQueueAttention,
  WorkspaceSnapshot,
} from "./contracts";

export type WorkstationCardStatus =
  | "thinking"
  | "running"
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

interface ProjectWorkstationOptions {
  readonly pendingIntent?: WorkstationPendingIntent | null;
}

export function projectWorkstationCards(
  snapshot: WorkspaceSnapshot,
  options: ProjectWorkstationOptions = {},
): WorkstationProjection {
  const issueSlices = new Map(
    snapshot.mission_board.issue_slices?.map((issue) => [issue.issue_id, issue]),
  );
  const cards =
    snapshot.missions?.flatMap((mission) => [
      ...mission.attention.map((attention) =>
        projectAttentionCard(snapshot, mission, attention),
      ),
      ...mission.sessions.map((session) => {
        const issue = issueSlices.get(session.issue_id);
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

function projectAttentionCard(
  snapshot: WorkspaceSnapshot,
  mission: WorkspaceMissionSummary,
  attention: WorkspaceQueueAttention,
): WorkstationCardProjection {
  return {
    id: `attention:${attention.attention_id}`,
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
    lastActivity: `Revision ${snapshot.revision}`,
    approvalBlockers: [attention.label],
    filesTouched: 0,
    latestCommandOrTest: "No command or test summary",
    nextAction: "Open Workspace Queue",
    acceptedRevision: snapshot.revision,
    attention: true,
    tone: "attention",
  };
}

function projectSessionCard(
  snapshot: WorkspaceSnapshot,
  mission: WorkspaceMissionSummary,
  session: WorkspaceMissionSummary["sessions"][number],
  issue: WorkspaceIssueSliceSummary | undefined,
  detail: WorkspaceIssueSessionDetail | undefined,
): WorkstationCardProjection {
  const status = canonicalStatus(session.status, detail?.operation_status, detail?.failure);
  const latestCommandOrTest = latestEvidenceLine(issue);
  const progress = issue?.progress ?? detail?.operation_status ?? session.status;
  const failure = detail?.failure ? [detail.failure] : [];
  const attention = status === "blocked" || status === "failed";
  return {
    id: `session:${session.session_id}`,
    missionId: mission.id,
    missionTitle: mission.title,
    name: session.assigned_agent,
    sessionId: session.session_id,
    issueId: session.issue_id,
    model: detail?.model || session.model || issue?.model_assignment.model || session.assigned_agent,
    role: detail?.role || session.role || issue?.model_assignment.role || "agent",
    currentTask: issue?.title ?? session.issue_id,
    status,
    phase: detail?.operation_status || session.status,
    progress,
    lastActivity: lastActivity(snapshot, detail, latestCommandOrTest),
    approvalBlockers: failure,
    filesTouched: issue?.evidence.changed_files.length ?? 0,
    latestCommandOrTest,
    nextAction: nextAction(status, progress, detail?.failure),
    acceptedRevision: snapshot.revision,
    attention,
    tone: toneForStatus(status),
  };
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
    running: 4,
    thinking: 5,
    "review-ready": 6,
    done: 7,
  };
  return priority[status];
}

function toneForStatus(status: WorkstationCardStatus): WorkstationCardProjection["tone"] {
  if (status === "failed") return "failed";
  if (status === "waiting-approval" || status === "blocked") return "attention";
  if (status === "running" || status === "reviewing" || status === "review-ready") return "active";
  return "muted";
}

function canonicalStatus(
  rawStatus = "",
  rawOperationStatus = "",
  failure = "",
): WorkstationCardStatus {
  const status = `${rawStatus} ${rawOperationStatus}`.toLowerCase();
  if (failure || status.includes("failed") || status.includes("rejected")) return "failed";
  if (status.includes("waiting") || status.includes("approval") || status.includes("pending")) {
    return "waiting-approval";
  }
  if (status.includes("blocked") || status.includes("needs-repair")) return "blocked";
  if (status.includes("reviewing") || status.includes("needs-review")) return "reviewing";
  if (status.includes("evidence-ready") || status.includes("awaiting-review")) return "review-ready";
  if (
    status.includes("reviewed") ||
    status.includes("complete") ||
    status.includes("merged") ||
    status.includes("done")
  ) {
    return "done";
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

function latestEvidenceLine(issue: WorkspaceIssueSliceSummary | undefined): string {
  const command = issue?.evidence.commands_run.at(-1);
  if (command) return command;
  const tests = issue?.evidence.test_results;
  return tests && tests !== "No evidence package recorded."
    ? tests
    : "No command or test summary";
}

function lastActivity(
  snapshot: WorkspaceSnapshot,
  detail: WorkspaceIssueSessionDetail | undefined,
  latestCommandOrTest: string,
): string {
  if (detail?.failure) return detail.failure;
  if (latestCommandOrTest !== "No command or test summary") return latestCommandOrTest;
  if (detail?.operation_status) return `Revision ${snapshot.revision} / ${detail.operation_status}`;
  return `Revision ${snapshot.revision}`;
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
  return progress || "Monitor active work";
}
