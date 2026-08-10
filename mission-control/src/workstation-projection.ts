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
  MissionSessionSummary,
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
    | "storage"
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
    | "issue-archive"
    | "issue-restore"
    | "retirement-pin"
    | "retirement-retry"
    | "retirement-export"
    | "retirement-discard"
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
  readonly repairTaskPacket?: MissionSessionSummary["repair_task_packet"];
  readonly pinState?: boolean;
  readonly requiresDestination?: boolean;
  readonly requiresConfirmation?: boolean;
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
  readonly retirementRecord: MissionSessionSummary["retirement_record"] | null;
  readonly retirementPhase?: string;
  readonly retirementBlockedReason?: string;
  readonly retirementRunnerBoundary?: Readonly<Record<string, unknown>>;
  readonly preservationBudget?: Readonly<Record<string, unknown>>;
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

export type MissionExecutionNodeKind =
  | "mission"
  | "archive"
  | "issue-slice"
  | "ad-hoc-delegation"
  | "agent-session";

export type MissionExecutionNodeState =
  | "working"
  | "queued"
  | "decision-needed"
  | "complete"
  | "blocked"
  | "failed"
  | "idle";

export interface MissionExecutionTreeNode {
  readonly id: string;
  readonly kind: MissionExecutionNodeKind;
  readonly identity: string;
  readonly title: string;
  readonly parent_id: string | null;
  readonly parent_session_id: string | null;
  readonly lineage: "root" | "repair";
  readonly depth: number;
  readonly child_ids: readonly string[];
  readonly state: MissionExecutionNodeState;
  readonly status: string;
  readonly shape: "hexagon" | "square" | "diamond" | "circle" | "repair";
  readonly risk: "none" | "attention" | "blocked" | "failed";
  readonly summary: string;
  readonly inspectable: boolean;
  readonly attention: boolean;
  readonly issue: WorkspaceIssueSliceSummary | null;
  readonly session: MissionSessionSummary | null;
  readonly card: WorkstationCardProjection | null;
  /** Only canonical Issue Slice archive/restore controls may be hosted by a tree node. */
  readonly governed_actions?: readonly WorkstationGovernedAction[];
  readonly blocker_recommendations?: readonly MissionExecutionBlockerRecommendation[];
  readonly archived?: boolean;
}

export interface MissionExecutionBlockerRecommendation {
  readonly blocker_id: string;
  readonly title: string;
  readonly rationale: string;
  readonly proposed_acceptance: string;
  readonly assigned_actor: string;
  readonly dependency_consequence: string;
}

export interface MissionExecutionTreeProjection {
  readonly schema_version: 1;
  readonly revision: number;
  readonly root_id: string | null;
  readonly nodes: readonly MissionExecutionTreeNode[];
  readonly counts: {
    readonly issue_slices: number;
    readonly ad_hoc_delegations: number;
    readonly local_agent_sessions: number;
    readonly repairs: number;
    readonly blockers: number;
    readonly evidence_packages: number;
  };
}

interface ProjectMissionExecutionTreeOptions {
  readonly workspaceQueue?: WorkspaceQueueProjection | null;
}

/**
 * Build the Mission Work hierarchy from canonical snapshot and queue records.
 *
 * This projection deliberately keeps the tree shallow in data ownership: Issue
 * Slices and Ad Hoc Delegations own the Local Agent sessions that serve them;
 * repairs remain nested session records and never become a second work item.
 */
export function projectMissionExecutionTree(
  snapshot: WorkspaceSnapshot,
  options: ProjectMissionExecutionTreeOptions = {},
): MissionExecutionTreeProjection {
  const activeMission = snapshot.active_mission;
  if (!activeMission) {
    return {
      schema_version: 1,
      revision: snapshot.revision,
      root_id: null,
      nodes: [],
      counts: {
        issue_slices: 0,
        ad_hoc_delegations: 0,
        local_agent_sessions: 0,
        repairs: 0,
        blockers: 0,
        evidence_packages: 0,
      },
    };
  }

  const mission =
    snapshot.missions?.find((candidate) => candidate.id === activeMission.id) ?? {
      id: activeMission.id,
      title: activeMission.title,
      issue_count: activeMission.issue_count,
      is_active: true,
      sessions: [],
      attention: [],
    };
  const issueById = new Map(
    snapshot.mission_board.issue_slices?.map((issue) => [issue.issue_id, issue]) ?? [],
  );
  const archivedIssueIds = new Set(
    (mission.archived_issue_ids ?? []).filter(
      (issueId): issueId is string => typeof issueId === "string" && issueById.has(issueId),
    ),
  );
  const cards = projectWorkstationCards(snapshot, {
    workspaceQueue: options.workspaceQueue ?? null,
  }).groups.flatMap((group) => group.cards).filter((card) => card.missionId === mission.id);
  const cardBySessionId = new Map(
    cards.flatMap((card) => (card.sessionId ? [[card.sessionId, card] as const] : [])),
  );
  const cardByAttentionId = new Map(
    cards.flatMap((card) =>
      card.id.startsWith(`attention:${mission.id}:`)
        ? [[card.id.slice(`attention:${mission.id}:`.length), card] as const]
        : [],
    ),
  );
  const sessionsByIssue = new Map<string, MissionSessionSummary[]>();
  for (const session of mission.sessions) {
    const current = sessionsByIssue.get(session.issue_id) ?? [];
    current.push(session);
    sessionsByIssue.set(session.issue_id, current);
  }

  const adHocIds = new Set<string>();
  for (const session of mission.sessions) {
    if (session.work_kind === "ad-hoc-delegation") {
      adHocIds.add(session.issue_id);
    }
  }
  for (const attention of mission.attention) {
    if (attention.kind === "ad-hoc-delegation" || attention.kind === "delegation-approval") {
      adHocIds.add(attention.entity_id);
    }
  }
  for (const item of options.workspaceQueue?.items ?? []) {
    if (item.mission_id === mission.id && item.item_type === "ad-hoc-delegation") {
      adHocIds.add(item.issue_id);
    }
  }

  const nodes: MissionExecutionTreeNode[] = [];
  const childrenByParent = new Map<string, string[]>();
  const addChild = (parentId: string, childId: string): void => {
    const children = childrenByParent.get(parentId) ?? [];
    if (!children.includes(childId)) children.push(childId);
    childrenByParent.set(parentId, children);
  };
  const addNode = (
    node: Omit<MissionExecutionTreeNode, "child_ids">,
  ): MissionExecutionTreeNode => {
    const existing = nodes.find((candidate) => candidate.id === node.id);
    if (existing) return existing;
    const created = { ...node, child_ids: [] };
    nodes.push(created);
    if (node.parent_id) addChild(node.parent_id, node.id);
    return created;
  };

  const rootId = `mission:${mission.id}`;
  addNode({
    id: rootId,
    kind: "mission",
    identity: mission.id,
    title: mission.title,
    parent_id: null,
    parent_session_id: null,
    lineage: "root",
    depth: 0,
    state: "working",
    status: "Active Mission",
    shape: executionNodeShape("working", mission.attention.length > 0 ? "attention" : "none", "root"),
    risk: mission.attention.length > 0 ? "attention" : "none",
    summary: `${activeMission.issue_count} Issue Slices · ${mission.sessions.length} Local Agent sessions`,
    inspectable: false,
    attention: mission.attention.length > 0,
    issue: null,
    session: null,
    card: null,
  });

  const archiveNodeId = `archive:${mission.id}`;
  if (archivedIssueIds.size > 0) {
    addNode({
      id: archiveNodeId,
      kind: "archive",
      identity: "archived-work",
      title: "Archived completed work",
      parent_id: rootId,
      parent_session_id: null,
      lineage: "root",
      depth: 1,
      state: "complete",
      status: "History retained",
      shape: "square",
      risk: "none",
      summary: `${archivedIssueIds.size} completed Issue Slice ${archivedIssueIds.size === 1 ? "subtree" : "subtrees"} retained outside active work`,
      inspectable: false,
      attention: false,
      issue: null,
      session: null,
      card: null,
    });
  }

  const orderedIssueIds = snapshot.mission_board.ordered_issue_ids.filter((issueId) =>
    issueById.has(issueId),
  );
  for (const issueId of orderedIssueIds) {
    const issue = issueById.get(issueId)!;
    const archived = archivedIssueIds.has(issueId);
    const issueSessions = sessionsByIssue.get(issueId) ?? [];
    const issueCardStatuses = issueSessions.map(
      (session) => cardBySessionId.get(session.session_id)?.status ?? canonicalStatus(
        session.status,
        session.operation_status,
        session.failure,
      ),
    );
    const state = issueExecutionState(issue, issueCardStatuses);
    const issueNodeId = `issue:${mission.id}:${issue.issue_id}`;
    addNode({
      id: issueNodeId,
      kind: "issue-slice",
      identity: issue.issue_id,
      title: issue.title,
      parent_id: archived ? archiveNodeId : rootId,
      parent_session_id: null,
      lineage: "root",
      depth: archived ? 2 : 1,
      state,
      status: issue.lifecycle,
      shape: executionNodeShape(state, issueRisk(issue), "root"),
      risk: executionNodeRisk(state, issueRisk(issue)),
      summary: `${issue.issue_id} · ${issueSessions.length} Local Agent ${issueSessions.length === 1 ? "session" : "sessions"} · ${issue.progress}`,
      inspectable: true,
      attention: state === "blocked" || state === "failed" || issue.blockers.some((blocker) => !blocker.satisfied),
      issue,
      session: null,
      card: null,
      archived,
      governed_actions: issueExecutionGovernedActions(
        issue,
        mission.id,
        snapshot.revision,
        archived,
      ),
      blocker_recommendations: blockerRecommendations(issue, issueById),
    });
    appendSessionNodes(
      issueNodeId,
      issueSessions,
      archived ? 3 : 2,
      issue,
      addNode,
      cardBySessionId,
      mission.id,
    );
  }

  const sortedAdHocIds = [...adHocIds].sort((left, right) => left.localeCompare(right));
  for (const adHocId of sortedAdHocIds) {
    const adHocSessions = sessionsByIssue.get(adHocId) ?? [];
    const queueItem = (options.workspaceQueue?.items ?? []).find(
      (candidate) => candidate.mission_id === mission.id &&
        candidate.item_type === "ad-hoc-delegation" && candidate.issue_id === adHocId,
    );
    const attention = mission.attention.find(
      (candidate) =>
        candidate.entity_id === adHocId &&
        (candidate.kind === "ad-hoc-delegation" ||
          candidate.kind === "delegation-approval" ||
          candidate.queue_item_id === queueItem?.item_id),
    );
    const title =
      adHocSessions[0]?.task_title ||
      (typeof queueItem?.proposed_changes.goal === "string" ? queueItem.proposed_changes.goal : "") ||
      attention?.label ||
      adHocId;
    const adHocNodeId = `ad-hoc:${mission.id}:${adHocId}`;
    const adHocState = adHocExecutionState(adHocSessions, cardBySessionId, Boolean(attention));
    addNode({
      id: adHocNodeId,
      kind: "ad-hoc-delegation",
      identity: adHocId,
      title,
      parent_id: rootId,
      parent_session_id: null,
      lineage: "root",
      depth: 1,
      state: adHocState,
      status: attention ? "Decision needed" : adHocStateLabel(adHocState),
      shape: executionNodeShape(
        adHocState,
        attention ? "attention" : adHocState === "failed" ? "failed" : "none",
        "root",
      ),
      risk: executionNodeRisk(
        adHocState,
        attention ? "attention" : adHocState === "failed" ? "failed" : "none",
      ),
      summary: `${adHocId} · ${adHocSessions.length} Local Agent ${adHocSessions.length === 1 ? "session" : "sessions"}${attention ? " · pending approval" : ""}`,
      inspectable: true,
      attention: Boolean(attention),
      issue: null,
      session: null,
      card: attention ? cardByAttentionId.get(attention.attention_id) ?? null : null,
    });
    appendSessionNodes(
      adHocNodeId,
      adHocSessions,
      2,
      null,
      addNode,
      cardBySessionId,
      mission.id,
    );
  }

  const unownedSessions = mission.sessions.filter(
    (session) => !issueById.has(session.issue_id) && !adHocIds.has(session.issue_id),
  );
  appendSessionNodes(rootId, unownedSessions, 1, null, addNode, cardBySessionId, mission.id);

  const evidencePackages = mission.sessions.filter(
    (session) => Boolean(session.evidence_correlation_id || session.artifact_links?.length),
  ).length;
  const repairs = mission.sessions.filter((session) => Boolean(session.parent_session_id)).length;
  const blockers = [...issueById.values()].reduce(
    (count, issue) => count + issue.blockers.filter((blocker) => !blocker.satisfied).length,
    0,
  );
  return {
    schema_version: 1,
    revision: snapshot.revision,
    root_id: rootId,
    nodes: nodes.map((node) => ({
      ...node,
      child_ids: childrenByParent.get(node.id) ?? [],
    })),
    counts: {
      issue_slices: orderedIssueIds.length,
      ad_hoc_delegations: sortedAdHocIds.length,
      local_agent_sessions: mission.sessions.length,
      repairs,
      blockers,
      evidence_packages: evidencePackages,
    },
  };
}

function appendSessionNodes(
  parentId: string,
  sessions: readonly MissionSessionSummary[],
  depth: number,
  issue: WorkspaceIssueSliceSummary | null,
  addNode: (node: Omit<MissionExecutionTreeNode, "child_ids">) => MissionExecutionTreeNode,
  cardBySessionId: ReadonlyMap<string, WorkstationCardProjection>,
  missionId: string,
): void {
  const sessionsById = new Map(sessions.map((session) => [session.session_id, session] as const));
  const cyclicSessionIds = new Set<string>();
  for (const session of sessions) {
    const lineage = new Set<string>();
    let current: MissionSessionSummary | undefined = session;
    while (current?.parent_session_id?.trim()) {
      if (lineage.has(current.session_id)) {
        lineage.forEach((sessionId) => cyclicSessionIds.add(sessionId));
        break;
      }
      lineage.add(current.session_id);
      current = sessionsById.get(current.parent_session_id.trim());
      if (!current) break;
    }
  }
  const visited = new Set<string>();
  const visit = (
    session: MissionSessionSummary,
    fallbackParentId: string,
    fallbackDepth: number,
  ): void => {
    if (visited.has(session.session_id)) return;
    visited.add(session.session_id);
    const card = cardBySessionId.get(session.session_id) ?? null;
    const state = sessionNodeState(session, card ?? null);
    const parentSessionId = session.parent_session_id?.trim() || null;
    const parentSession =
      parentSessionId &&
      !cyclicSessionIds.has(session.session_id) &&
      !cyclicSessionIds.has(parentSessionId)
        ? sessionsById.get(parentSessionId)
        : undefined;
    const nodeParentId = parentSession
      ? `session:${missionId}:${parentSession.session_id}`
      : fallbackParentId;
    const nodeDepth = parentSession ? fallbackDepth + 1 : fallbackDepth;
    const baseRisk =
      state === "failed"
        ? "failed"
        : state === "blocked"
          ? "blocked"
          : card?.attention
            ? "attention"
            : "none";
    addNode({
      id: `session:${missionId}:${session.session_id}`,
      kind: "agent-session",
      identity: session.session_id,
      title: session.task_title || `Session activity for ${session.session_id}`,
      parent_id: nodeParentId,
      parent_session_id: parentSessionId,
      lineage: parentSession ? "repair" : "root",
      depth: nodeDepth,
      state,
      status: card?.status ?? session.status,
      shape: executionNodeShape(state, baseRisk, parentSession ? "repair" : "root"),
      risk: executionNodeRisk(state, baseRisk),
      summary: `${session.session_id} · ${session.assigned_agent} · ${session.role || "Local Agent"} · ${session.model || "model not recorded"}`,
      inspectable: true,
      attention: Boolean(
        card?.attention ||
          session.failure ||
          session.review_next_action ||
          session.supervision_outcome === "decision-needed",
      ),
      issue,
      session,
      card,
    });
    if (!cyclicSessionIds.has(session.session_id)) {
      for (const child of sessions) {
        if (child.parent_session_id?.trim() === session.session_id) {
          visit(child, `session:${missionId}:${session.session_id}`, nodeDepth);
        }
      }
    }
  };

  for (const session of sessions) {
    const parentSessionId = session.parent_session_id?.trim();
    if (
      cyclicSessionIds.has(session.session_id) ||
      !parentSessionId ||
      !sessionsById.has(parentSessionId) ||
      cyclicSessionIds.has(parentSessionId)
    ) {
      visit(session, parentId, depth);
    }
  }
  for (const session of sessions) {
    if (!visited.has(session.session_id)) visit(session, parentId, depth);
  }
}

function issueExecutionState(
  issue: WorkspaceIssueSliceSummary,
  sessionStatuses: readonly WorkstationCardStatus[],
): MissionExecutionNodeState {
  if (issue.blockers.some((blocker) => !blocker.satisfied)) return "blocked";
  if (sessionStatuses.some((status) => status === "failed")) return "failed";
  if (issue.evidence.state === "accepted" || /complete|merged/i.test(issue.lifecycle)) return "complete";
  if (sessionStatuses.some((status) => status === "review-ready" || status === "reviewing")) return "decision-needed";
  if (sessionStatuses.some((status) => status === "running" || status === "thinking")) return "working";
  if (sessionStatuses.some((status) => status === "queued")) return "queued";
  return issue.launch_eligible ? "idle" : "blocked";
}

function adHocExecutionState(
  sessions: readonly MissionSessionSummary[],
  cards: ReadonlyMap<string, WorkstationCardProjection>,
  attention: boolean,
): MissionExecutionNodeState {
  if (attention) return "decision-needed";
  const states = sessions.map((session) => sessionNodeState(session, cards.get(session.session_id) ?? null));
  if (states.some((state) => state === "failed")) return "failed";
  if (states.some((state) => state === "working")) return "working";
  if (states.some((state) => state === "queued")) return "queued";
  if (states.length > 0 && states.every((state) => state === "complete")) return "complete";
  return "idle";
}

function sessionNodeState(
  session: MissionSessionSummary,
  card: WorkstationCardProjection | null,
): MissionExecutionNodeState {
  const status = card?.status ?? canonicalStatus(session.status, session.operation_status, session.failure);
  if (status === "failed") return "failed";
  if (status === "blocked") return "blocked";
  if (status === "review-ready" || status === "reviewing" || session.review_next_action) return "decision-needed";
  if (status === "done") return "complete";
  if (status === "queued") return "queued";
  if (status === "running" || status === "thinking") return "working";
  return "idle";
}

export function executionStateLabel(state: MissionExecutionNodeState): string {
  return state === "decision-needed" ? "Decision needed" : state[0].toUpperCase() + state.slice(1);
}

export function executionRiskLabel(risk: MissionExecutionTreeNode["risk"]): string {
  return risk === "none" ? "No elevated risk" : risk === "attention" ? "Attention" : risk[0].toUpperCase() + risk.slice(1);
}

function executionNodeRisk(
  state: MissionExecutionNodeState,
  risk: MissionExecutionTreeNode["risk"],
): MissionExecutionTreeNode["risk"] {
  if (state === "failed") return "failed";
  if (state === "blocked") return "blocked";
  return risk;
}

function executionNodeShape(
  state: MissionExecutionNodeState,
  risk: MissionExecutionTreeNode["risk"],
  lineage: MissionExecutionTreeNode["lineage"],
): MissionExecutionTreeNode["shape"] {
  if (lineage === "repair") return "repair";
  if (risk === "failed" || risk === "blocked" || state === "failed" || state === "blocked") return "hexagon";
  if (risk === "attention" || state === "decision-needed") return "diamond";
  if (state === "working") return "circle";
  return "square";
}

function adHocStateLabel(state: MissionExecutionNodeState): string {
  return executionStateLabel(state);
}

function issueRisk(issue: WorkspaceIssueSliceSummary): MissionExecutionTreeNode["risk"] {
  if (issue.blockers.some((blocker) => !blocker.satisfied)) return "blocked";
  const risks = issue.evidence.risks.trim().toLowerCase();
  if (risks && !/^(?:none|none recorded|no risks recorded)\.?$/.test(risks)) return "attention";
  return "none";
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
  const runnerSupervision = attention.kind === "runner-supervision";
  const retirementStorage = attention.kind === "retirement-storage";
  const supervisedSession = runnerSupervision
    ? mission.sessions.find((session) => session.session_id === attention.entity_id) ?? null
    : null;
  const supervisionActions =
    supervisedSession?.status === "failed"
      ? governedActions(
          "failed",
          supervisedSession.session_id,
          supervisedSession.issue_id,
          snapshot.revision,
          mission.id,
        ).filter((action) => action.actionType === "issue-retry")
      : [];
  return {
    id: `attention:${mission.id}:${attention.attention_id}`,
    missionId: mission.id,
    missionTitle: mission.title,
    name: attention.label,
    sessionId: runnerSupervision ? attention.entity_id : null,
    issueId: supervisedSession?.issue_id ?? null,
    model: "orchestrator",
    role: "governance",
    currentTask: attention.kind,
    status: retirementStorage
      ? "blocked"
      : runnerSupervision
      ? supervisedSession?.status === "failed"
        ? "failed"
        : "blocked"
      : "waiting-approval",
    phase: retirementStorage ? "Snapshot Storage" : runnerSupervision ? "Supervision" : "Approval",
    progress: retirementStorage
      ? "New Retirement Units are blocked until storage policy can reserve capacity"
      : runnerSupervision
      ? "Automatic recovery stopped; Mission Commander decision required"
      : "Workspace Queue item pending",
    lastActivity: "",
    approvalBlockers: runnerSupervision ? [] : [attention.label],
    filesTouched: 0,
    latestCommandOrTest: "No command or test summary",
    nextAction: retirementStorage
      ? "Inspect /storage, then unpin expired payloads or raise the configured budget before retrying admission"
      : runnerSupervision
      ? supervisionActions.length
        ? "Choose manual Retry with a reason, or leave the session stopped"
        : "Inspect the Local Agent session in Mission Work"
      : "Open Workspace Queue",
    acceptedRevision: snapshot.revision,
    attention: true,
    tone: "attention",
    detail: {
      originatingSessionId: null,
      issueId: supervisedSession?.issue_id ?? null,
      toolActivity: [
        {
          kind: "queue-summary",
          label: runnerSupervision ? "Runner supervision" : "Workspace Queue",
          summary: `${attention.kind}: ${attention.label}`,
        },
      ],
      filesTouched: [],
      diffs: [],
      evidenceLinks: [],
      terminalExcerpts: [],
      reviewState: {
        evidenceState: "not-applicable",
        lifecycle: runnerSupervision ? "Decision required" : "Waiting approval",
        risks: runnerSupervision
          ? "Automatic recovery is stopped until the Mission Commander chooses a manual next step."
          : "No evidence package attached to this queue item.",
        reviewReady: false,
      },
      retirementRecord: null,
      governedActions: retirementStorage
        ? []
        : runnerSupervision
        ? supervisionActions
        : queueItem
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
  const itemId = attention.queue_item_id;
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
      session.repair_task_packet,
      session.retirement_phase,
      session.retirement_actions,
      session.retirement_record,
      session.session_revision,
      session.retirement_blocked_reason,
      session.retirement_runner_boundary,
      session.preservation_budget,
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
  repairTaskPacket: MissionSessionSummary["repair_task_packet"] = null,
  retirementPhase = "active",
  retirementActions: MissionSessionSummary["retirement_actions"] = undefined,
  retirementRecord: MissionSessionSummary["retirement_record"] = undefined,
  retirementRevision = acceptedRevision,
  retirementBlockedReason = "",
  retirementRunnerBoundary: MissionSessionSummary["retirement_runner_boundary"] = undefined,
  preservationBudget: MissionSessionSummary["preservation_budget"] = undefined,
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
    retirementRecord: retirementRecord ?? null,
    retirementPhase,
    retirementBlockedReason,
    retirementRunnerBoundary,
    preservationBudget,
    governedActions: governedActions(
      status,
      sessionId,
      issueId,
      acceptedRevision,
      missionId,
      repairActionAvailable,
      repairTaskPacket,
      retirementPhase,
      retirementActions,
      retirementRecord,
      retirementRevision,
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
  repairTaskPacket: MissionSessionSummary["repair_task_packet"] = null,
  retirementPhase = "active",
  retirementActions: MissionSessionSummary["retirement_actions"] = undefined,
  retirementRecord: MissionSessionSummary["retirement_record"] = undefined,
  retirementRevision = expectedRevision,
): readonly WorkstationGovernedAction[] {
  const retainedSnapshot = retirementRecord?.payload_disposition === "retained";
  const snapshotPolicyAction: WorkstationGovernedAction[] = retainedSnapshot
    ? [
        {
          label: retirementRecord?.pinned ? "Unpin Snapshot Payload" : "Pin Snapshot Payload",
          target: "storage",
          requiresReason: false,
          actionType: "retirement-pin",
          actor: "mission-commander",
          missionId,
          sessionId,
          issueId,
          expectedRevision: retirementRevision,
          pinState: !retirementRecord?.pinned,
          recoveryPath: "Reload canonical Mission Work and retry against the current Retirement Record revision.",
          targetIdentity: { kind: "agent-session", id: sessionId },
        },
      ]
    : [];
  if (
    retirementActions &&
    (retirementPhase === "preservation-blocked" || retirementPhase === "retirement-blocked")
  ) {
    return [
      {
        label: "Retry Retirement",
        target: "mission-board",
        requiresReason: false,
        actionType: "retirement-retry",
        actor: "mission-commander",
        missionId,
        sessionId,
        issueId,
        expectedRevision: retirementRevision,
        targetIdentity: { kind: "agent-session", id: sessionId },
      },
      ...(retirementActions.export
        ? [
            {
              label: "Export Retained Worktree",
              target: "storage" as const,
              requiresReason: false,
              requiresDestination: true,
              actionType: "retirement-export" as const,
              actor: "mission-commander" as const,
              missionId,
              sessionId,
              issueId,
              expectedRevision: retirementRevision,
              targetIdentity: { kind: "agent-session" as const, id: sessionId },
            },
          ]
        : []),
      {
        label: "Discard Retained Worktree",
        target: "storage",
        requiresReason: true,
        requiresConfirmation: true,
        actionType: "retirement-discard",
        actor: "mission-commander",
        missionId,
        sessionId,
        issueId,
        expectedRevision: retirementRevision,
        targetIdentity: { kind: "agent-session", id: sessionId },
      },
      ...snapshotPolicyAction,
    ];
  }
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
        repairTaskPacket,
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
    return [
      { label: "Open Activity", target: "activity", requiresReason: false },
      ...snapshotPolicyAction,
    ];
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

function issueExecutionGovernedActions(
  issue: WorkspaceIssueSliceSummary,
  missionId: string,
  expectedRevision: number,
  archived: boolean,
): readonly WorkstationGovernedAction[] {
  if (archived) {
    return [
      {
        label: "Restore Issue Slice",
        target: "mission-board",
        requiresReason: false,
        actionType: "issue-restore",
        actor: "mission-commander",
        missionId,
        issueId: issue.issue_id,
        expectedRevision,
        targetIdentity: { kind: "issue-slice", id: issue.issue_id },
        recoveryPath: "Reload canonical Mission Work and restore the retained completed subtree.",
      },
    ];
  }
  if (issue.lifecycle !== "Complete" && issue.lifecycle !== "Merged") return [];
  return [
    {
      label: "Archive completed Issue Slice",
      target: "mission-board",
      requiresReason: false,
      actionType: "issue-archive",
      actor: "mission-commander",
      missionId,
      issueId: issue.issue_id,
      expectedRevision,
      targetIdentity: { kind: "issue-slice", id: issue.issue_id },
      recoveryPath: "Reload canonical Mission Work and archive only the currently completed subtree.",
    },
  ];
}

function blockerRecommendations(
  issue: WorkspaceIssueSliceSummary,
  issueById: ReadonlyMap<string, WorkspaceIssueSliceSummary>,
): readonly MissionExecutionBlockerRecommendation[] {
  return issue.blockers
    .filter((blocker) => !blocker.satisfied)
    .map((blocker) => {
      const dependency = issueById.get(blocker.issue_id);
      const proposedAcceptance = dependency?.accepted_boundary.acceptance_criteria.join(" · ") ||
        "The dependency must record an accepted reviewed outcome.";
      const assignedActor = dependency?.model_assignment.agent_id || "No Local Agent assigned";
      return {
        blocker_id: blocker.issue_id,
        title: blocker.title,
        rationale: `${issue.issue_id} remains blocked because ${blocker.issue_id} is ${blocker.lifecycle}.`,
        proposed_acceptance: proposedAcceptance,
        assigned_actor: assignedActor,
        dependency_consequence:
          `${issue.issue_id} remains blocked until ${blocker.issue_id} has an accepted reviewed outcome; creating or approving follow-up work does not unblock ${issue.issue_id}.`,
      };
    });
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

export function workstationActionTargetId(action: WorkstationGovernedAction): string {
  return action.targetIdentity?.id ?? action.sessionId ?? action.issueId ?? action.itemId ?? "";
}

export function workstationActionStateId(action: WorkstationGovernedAction): string {
  return JSON.stringify([
    action.missionId ?? "",
    workstationActionTargetId(action),
    action.decision ?? action.reviewDecision ?? action.actionType ?? "",
  ]);
}

/**
 * The shared pending/outcome identity for one exact Review Decision. Keep the
 * decision in this key: the three decisions for one Local Agent session are
 * independent governed actions and must never disable one another.
 */
export function workstationReviewActionStateId(
  missionId: string,
  sessionId: string,
  decision: ReviewDecision,
): string {
  return workstationActionStateId({
    label: "review-decision",
    target: "review-workspace",
    requiresReason: decision === "repair",
    actionType: "review-decision",
    missionId,
    sessionId,
    reviewDecision: decision,
    targetIdentity: { kind: "agent-session", id: sessionId },
  });
}

export function workstationActionKey(action: WorkstationGovernedAction): string {
  return `${action.actionType ?? "workstation-action"}:${workstationActionStateId(action)}`;
}

export function workstationActionConsequence(action: WorkstationGovernedAction): string {
  const targetId = workstationActionTargetId(action);
  switch (action.actionType) {
    case "workspace-queue-decision":
      if (action.decision === "approve") {
        return `Approving ${targetId || "this queue item"} records the decision and queues one bounded Local Agent session.`;
      }
      if (action.decision === "reject") {
        return `Rejecting ${targetId || "this queue item"} records the Mission Commander's reason; no session is launched.`;
      }
      return `Deferring ${targetId || "this queue item"} retains the pending proposal without launching work.`;
    case "issue-approve":
      return `Approval records ${targetId || "this Issue Slice"} as launch-authorized; it does not launch a session.`;
    case "issue-launch":
      return `Acknowledgement queues one ${targetId || "Issue Slice"} Local Agent session; the runner starts only after canonical state is reloaded.`;
    case "issue-retry":
      if (action.repairTaskPacket) {
        return `Acknowledgement queues exactly one canonical repair session for ${targetId || "this work"} from its inherited review task packet; it does not run inline or duplicate the prior session.`;
      }
      return `Acknowledgement queues one canonical repair or retry session for ${targetId || "this work"}; it does not run the session inline.`;
    case "session-cancel":
      return `Acknowledgement records cancellation for ${targetId || "this session"}; the Orchestrator stops active work, and cancellation is not completion.`;
    case "model-assignment-change":
      return `Acknowledgement changes the assigned eligible worker for ${targetId || "this Issue Slice"}; it does not launch work.`;
    case "issue-archive":
      return `Acknowledgement archives the completed ${targetId || "Issue Slice"} subtree from active Mission Work while retaining its identity, sessions, Evidence Packages, and Activity Journal history for inspection.`;
    case "issue-restore":
      return `Acknowledgement restores the retained ${targetId || "Issue Slice"} subtree to active Mission Work with its identity, sessions, Evidence Packages, and Activity Journal history intact.`;
    case "retirement-pin":
      return `${action.pinState ? "Pinning" : "Unpinning"} changes retention policy for ${targetId || "this Snapshot Payload"}, records the new pin state, and advances the session revision.`;
    case "retirement-retry":
      return `Acknowledgement retries the exact blocked Retirement Unit ${targetId || "session"} without bypassing preservation, quiescence, or containment checks.`;
    case "retirement-export":
      return `Acknowledgement copies the exact blocked retained material for ${targetId || "this session"} to the empty destination after quiescence and identity checks; it does not change canonical work.`;
    case "retirement-discard":
      return `Acknowledgement irreversibly deletes only the exact retained managed worktree for ${targetId || "this session"} after confirmation, reason, quiescence, identity, and containment proof.`;
    case "review-decision":
      if (action.reviewDecision === "accept") {
        return `Accepting evidence marks ${targetId || "this Issue Slice"} complete and PR-ready; it does not merge changes.`;
      }
      if (action.reviewDecision === "repair") {
        return `Requesting repair records the reason and exposes one canonical repair action for ${targetId || "this session"}; it does not launch immediately.`;
      }
      return `Escalating ${targetId || "this review"} records needs-human-review; it does not create a ticket or launch an agent.`;
    default:
      return action.target === "none"
        ? "Monitoring only; no Mission state changes are available from this control."
        : `${action.label} opens ${action.target} without changing canonical Mission state.`;
  }
}
