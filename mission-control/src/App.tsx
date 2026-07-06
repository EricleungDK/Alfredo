import { useCallback, useEffect, useRef, useState } from "react";
import type {
  AdHocDelegationProposalRequest,
  AlfredoLaunchContext,
  AgentConsoleMessage,
  ActivityJournalFilters,
  ActivityJournalProjection,
  ConversationScope,
  PathAccessLevel,
  ShellTerminalClassification,
  ShellTerminalCommandRecord,
  ShellTerminalCommandStatus,
  MissionDraftCreateRequest,
  MissionDraftDecision,
  MissionDraftProjection,
  ReviewDecision,
  ReviewWorkspaceProjection,
  WorkingContextProjection,
  WorkspaceQueueDecision,
  WorkspaceQueueItem,
  WorkspaceQueueProjection,
  WorkspaceIssueSliceSummary,
  WorkspaceLoadResult,
  WorkspaceSnapshot,
  WorkstationActionRequest,
} from "./contracts";
import type { WorkspaceClient } from "./workspace-client";
import { applyWorkspaceUpdates } from "./workspace-sync";
import {
  projectWorkstationCards,
  type WorkstationCardGroup,
  type WorkstationCardProjection,
  type WorkstationDiffLink,
  type WorkstationGovernedAction,
} from "./workstation-projection";
import { ShellTerminalPanel } from "./ShellTerminalPanel";
import {
  useShellTerminal,
  type ContextualPathGrantRequest,
  type ShellTerminalController,
  type ShellTerminalTranscriptEntry,
} from "./use-shell-terminal";
import "./styles.css";

interface AdHocDelegationDraft {
  readonly acceptanceCriteria: readonly string[];
  readonly allowedPaths: readonly string[];
  readonly commandPolicy: Readonly<Record<string, string>>;
  readonly proposedAgent: string;
  readonly originatingMessageId: string;
}

interface MissionDraftCreateDraft {
  readonly proposedGoal: string;
  readonly selectedAdHocIds: readonly string[];
  readonly excludedAdHocIds: readonly string[];
  readonly newWorkItems: readonly string[];
  readonly dependencies: readonly string[];
  readonly unresolvedDecisions: readonly string[];
}

interface WorkstationActionTurn {
  readonly id: string;
  readonly content: string;
  readonly source: string;
  readonly outcome: string;
}

interface WorkstationActionState {
  readonly itemId: string;
  readonly state: "pending" | "accepted" | "rejected" | "failed" | "stale" | "disabled";
  readonly message: string;
}

interface WorkstationActionDraftState {
  readonly reason: string;
  readonly agentId: string;
}

interface AppProps {
  readonly client: WorkspaceClient;
  readonly syncIntervalMs?: number;
}

interface WorkstationContinuityState {
  readonly schema_version: 1;
  readonly commandAuditOpen: boolean;
  readonly selectedIssueId: string | null;
  readonly selectedSessionId: string | null;
  readonly expandedWorkstationCardIds: readonly string[];
  readonly pinnedWorkstationCardIds: readonly string[];
  readonly workstationFilter: string;
  readonly workstationSort: "priority" | "name" | "status";
  readonly selectedWorkstationSessionId: string | null;
  readonly selectedWorkstationDiff: WorkstationDiffLink | null;
}

const WORKSTATION_CONTINUITY_SCHEMA_VERSION = 1;

export function App({ client, syncIntervalMs = 1000 }: AppProps) {
  const [state, setState] = useState<WorkspaceLoadResult | "loading">("loading");
  const [connectionStatus, setConnectionStatus] = useState<
    "connected" | "offline" | "reconnecting"
  >("reconnecting");
  const [actionStatus, setActionStatus] = useState<
    "pending" | "acknowledged" | "stale" | "rejected" | null
  >(null);
  const [consoleHistory, setConsoleHistory] = useState<readonly AgentConsoleMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [scopeDraft, setScopeDraft] = useState<ConversationScope | null>(null);
  const [messageStatus, setMessageStatus] = useState<"pending" | "rejected" | null>(null);
  const [workingContext, setWorkingContext] = useState<WorkingContextProjection | null>(null);
  const [contextStatus, setContextStatus] = useState<
    "pending" | "acknowledged" | "stale" | "rejected" | null
  >(null);
  const [reviewWorkspace, setReviewWorkspace] = useState<ReviewWorkspaceProjection | null>(null);
  const [reviewStatus, setReviewStatus] = useState<{
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null>(null);
  const [reviewReasons, setReviewReasons] = useState<Record<string, string>>({});
  const [workspaceQueue, setWorkspaceQueue] = useState<WorkspaceQueueProjection | null>(null);
  const [missionDrafts, setMissionDrafts] = useState<MissionDraftProjection | null>(null);
  const [missionDraftStatus, setMissionDraftStatus] = useState<{
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null>(null);
  const [missionDraftReasons, setMissionDraftReasons] = useState<Record<string, string>>({});
  const [queueStatus, setQueueStatus] = useState<{
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null>(null);
  const [workstationActionTurns, setWorkstationActionTurns] = useState<
    readonly WorkstationActionTurn[]
  >([]);
  const [workstationActionState, setWorkstationActionState] =
    useState<WorkstationActionState | null>(null);
  const [workstationActionDrafts, setWorkstationActionDrafts] = useState<
    Record<string, WorkstationActionDraftState>
  >({});
  const [queueReasons, setQueueReasons] = useState<Record<string, string>>({});
  const [activityJournal, setActivityJournal] = useState<ActivityJournalProjection | null>(null);
  const [activityFilters, setActivityFilters] = useState<ActivityJournalFilters>({
    search: "",
    mission_id: "",
    actor: "",
    action_type: "",
    started_at: "",
    ended_at: "",
  });
  const [activityStatus, setActivityStatus] = useState<"pending" | "rejected" | null>(null);
  const [commandAuditOpen, setCommandAuditOpen] = useState(false);
  const [launchContext, setLaunchContext] = useState<AlfredoLaunchContext | null>(null);
  const appendWorkstationActionTurn = useCallback((turn: WorkstationActionTurn) => {
    setWorkstationActionTurns((turns) => [...turns, turn]);
  }, []);
  const beginVisibleWorkstationAction = useCallback(
    (correlationId: string, label: string, targetId: string) => {
      setWorkstationActionState({
        itemId: targetId,
        state: "pending",
        message: `Waiting for Orchestrator acknowledgement: ${label}.`,
      });
      setWorkstationActionTurns((turns) => [
        ...turns,
        {
          id: `${correlationId}:intent`,
          content: `Workstation action: Mission Commander requested ${label}.`,
          source: "mission-commander",
          outcome: "pending",
        },
        {
          id: `${correlationId}:reaction:pending`,
          content: "Orchestrator validating workstation action.",
          source: "orchestrator",
          outcome: "pending",
        },
      ]);
    },
    [],
  );
  const finishVisibleWorkstationAction = useCallback(
    (
      correlationId: string,
      targetId: string,
      result: "acknowledged" | "stale" | "rejected" | "failed",
      message: string,
    ) => {
      const state =
        result === "acknowledged"
          ? "accepted"
          : result === "failed"
            ? "failed"
            : result === "stale"
              ? "stale"
              : "rejected";
      const recovery =
        result === "stale"
          ? `${message} Refresh the canonical workspace state and retry the action.`
          : message;
      setWorkstationActionState({
        itemId: targetId,
        state,
        message: recovery,
      });
      setWorkstationActionTurns((turns) => [
        ...turns,
        {
          id: `${correlationId}:reaction:${result}`,
          content:
            result === "acknowledged"
              ? `Orchestrator accepted workstation action: ${message}`
              : `Orchestrator ${result === "stale" ? "reported stale state" : "rejected workstation action"}: ${recovery}`,
          source: "orchestrator",
          outcome: result,
        },
      ]);
    },
    [],
  );
  const workspacePath =
    state !== "loading" && (state.kind === "ready" || state.kind === "empty")
      ? state.snapshot.workspace_session.workspace_path
      : "";
  const shellTerminal = useShellTerminal(client, workspacePath, {
    onWorkstationActionTurn: appendWorkstationActionTurn,
  });

  useEffect(() => {
    if (workspacePath && client.loadShellTerminal) void shellTerminal.load();
  }, [client.loadShellTerminal, shellTerminal.load, workspacePath]);

  useEffect(() => {
    if (commandAuditOpen) void shellTerminal.load();
  }, [commandAuditOpen, shellTerminal.load]);

  const refreshWorkingContext = useCallback(async () => {
    if (!client.loadWorkingContext) return false;
    const result = await client.loadWorkingContext();
    if (result.kind !== "working-context") return false;
    setWorkingContext(result.projection);
    return true;
  }, [client]);

  const refreshReviewWorkspace = useCallback(async () => {
    if (!client.loadReviewWorkspace) return false;
    const result = await client.loadReviewWorkspace();
    if (result.kind !== "review-workspace") return false;
    setReviewWorkspace(result.projection);
    return true;
  }, [client]);

  const refreshWorkspaceQueue = useCallback(async () => {
    if (!client.loadWorkspaceQueue) return false;
    const result = await client.loadWorkspaceQueue();
    if (result.kind !== "workspace-queue") return false;
    setWorkspaceQueue(result.projection);
    return true;
  }, [client]);

  const refreshMissionDrafts = useCallback(async () => {
    if (!client.loadMissionDrafts) return false;
    const result = await client.loadMissionDrafts();
    if (result.kind !== "mission-drafts") return false;
    setMissionDrafts(result.projection);
    return true;
  }, [client]);

  const refreshActivityJournal = useCallback(
    async (filters: ActivityJournalFilters) => {
      if (!client.loadActivityJournal) return false;
      setActivityStatus("pending");
      const result = await client.loadActivityJournal(filters);
      if (result.kind !== "activity-journal") {
        setActivityStatus("rejected");
        return false;
      }
      setActivityJournal(result.projection);
      setActivityStatus(null);
      return true;
    },
    [client],
  );

  const connect = useCallback(() => {
    setState("loading");
    void client.loadSnapshot().then((result) => {
      setState(result);
      setConnectionStatus(result.kind === "ready" || result.kind === "empty" ? "connected" : "offline");
    });
  }, [client]);

  useEffect(connect, [connect]);

  useEffect(() => {
    if (!client.loadLaunchContext) return;
    void client.loadLaunchContext().then((result) => {
      if (result.kind === "launch-context") setLaunchContext(result.context);
    });
  }, [client]);

  useEffect(() => {
    if (!client.loadConsoleHistory) return;
    void client.loadConsoleHistory().then((result) => {
      if (result.kind === "history") setConsoleHistory(result.history.messages);
    });
  }, [client]);

  useEffect(() => {
    void refreshWorkingContext();
  }, [refreshWorkingContext]);

  useEffect(() => {
    if (
      state !== "loading" &&
      (state.kind === "ready" || state.kind === "empty") &&
      state.snapshot.operations_view === "review-workspace"
    ) {
      void refreshReviewWorkspace();
    }
  }, [refreshReviewWorkspace, state]);

  useEffect(() => {
    if (
      state !== "loading" &&
      (state.kind === "ready" || state.kind === "empty") &&
      state.snapshot.operations_view === "workspace-queue"
    ) {
      void refreshWorkspaceQueue();
      void refreshMissionDrafts();
    }
  }, [refreshMissionDrafts, refreshWorkspaceQueue, state]);

  useEffect(() => {
    if (
      state !== "loading" &&
      (state.kind === "ready" || state.kind === "empty") &&
      state.snapshot.missions?.some((mission) => mission.attention.length > 0)
    ) {
      void refreshWorkspaceQueue();
    }
  }, [refreshWorkspaceQueue, state]);

  useEffect(() => {
    if (
      state !== "loading" &&
      (state.kind === "ready" || state.kind === "empty") &&
      state.snapshot.operations_view === "activity"
    ) {
      void refreshActivityJournal(activityFilters);
    }
  }, [refreshActivityJournal, state]);

  useEffect(() => {
    if (
      connectionStatus !== "connected" ||
      state === "loading" ||
      (state.kind !== "ready" && state.kind !== "empty") ||
      !client.loadUpdates
    ) {
      return;
    }
    const current = state;
    const timer = window.setTimeout(() => {
      void client.loadUpdates!(current.snapshot.revision).then((updates) => {
        if (updates.kind !== "updates") {
          setConnectionStatus("offline");
          return;
        }
        const applied = applyWorkspaceUpdates(current.snapshot, updates.batch);
        if (applied.kind !== "applied") {
          setConnectionStatus("offline");
          return;
        }
        if (applied.snapshot !== current.snapshot) {
          setState({ ...current, snapshot: applied.snapshot });
        }
      });
    }, syncIntervalMs);
    return () => window.clearTimeout(timer);
  }, [client, connectionStatus, state, syncIntervalMs]);

  const reconnect = useCallback(() => {
    setConnectionStatus("reconnecting");
    void client.loadSnapshot().then((result) => {
      if (result.kind === "ready" || result.kind === "empty") {
        setState(result);
        setConnectionStatus("connected");
      } else {
        setConnectionStatus("offline");
      }
    });
  }, [client]);

  const submitView = useCallback(
    async (operationsView: string) => {
      if (
        state === "loading" ||
        (state.kind !== "ready" && state.kind !== "empty") ||
        !client.submitAction
      ) {
        return;
      }
      const current = state;
      setActionStatus("pending");
      const result = await client.submitAction({
        correlation_id: `operations-view-${operationsView}-${current.snapshot.revision}`,
        expected_revision: current.snapshot.revision,
        operations_view: operationsView,
      });
      if (result.kind !== "acknowledged") {
        setActionStatus(result.kind);
        return;
      }
      if (!client.loadUpdates) {
        setActionStatus("rejected");
        return;
      }
      const updates = await client.loadUpdates(current.snapshot.revision);
      if (updates.kind !== "updates") {
        setActionStatus("rejected");
        return;
      }
      const applied = applyWorkspaceUpdates(current.snapshot, updates.batch);
      if (applied.kind !== "applied") {
        setActionStatus("rejected");
        return;
      }
      setState({ ...current, snapshot: applied.snapshot });
      setActionStatus("acknowledged");
      if (operationsView === "review-workspace") {
        await refreshReviewWorkspace();
      }
      if (operationsView === "workspace-queue") {
        await refreshWorkspaceQueue();
      }
      if (operationsView === "activity") {
        await refreshActivityJournal(activityFilters);
      }
    },
    [activityFilters, client, refreshActivityJournal, refreshReviewWorkspace, refreshWorkspaceQueue, state],
  );

  const submitMissionSwitch = useCallback(
    async (missionId: string) => {
      if (
        state === "loading" ||
        (state.kind !== "ready" && state.kind !== "empty") ||
        !client.switchMission
      ) {
        return;
      }
      const current = state;
      if (current.snapshot.active_mission?.id === missionId) return;
      setActionStatus("pending");
      const result = await client.switchMission({
        correlation_id: `active-mission-${missionId}-${current.snapshot.revision}`,
        expected_revision: current.snapshot.revision,
        active_mission_id: missionId,
      });
      if (result.kind !== "acknowledged") {
        setActionStatus(result.kind);
        return;
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        setActionStatus("rejected");
        setConnectionStatus("offline");
        return;
      }
      setState(reloaded);
      setConnectionStatus("connected");
      setActionStatus("acknowledged");
      await refreshWorkingContext();
    },
    [client, refreshWorkingContext, state],
  );

  const submitScope = useCallback(async () => {
    if (
      !scopeDraft ||
      state === "loading" ||
      (state.kind !== "ready" && state.kind !== "empty") ||
      !client.changeScope ||
      !client.loadUpdates
    ) {
      return;
    }
    const current = state;
    const correlationId = `conversation-scope-${scopeDraft.kind}-${scopeDraft.target_id}-${current.snapshot.revision}`;
    beginVisibleWorkstationAction(
      correlationId,
      `Change Conversation Scope to ${scopeDraft.label}`,
      `scope:${scopeDraft.kind}:${scopeDraft.target_id}`,
    );
    setActionStatus("pending");
    const result = await client.changeScope({
      correlation_id: correlationId,
      expected_revision: current.snapshot.revision,
      scope_kind: scopeDraft.kind,
      scope_target: scopeDraft.target_id,
      scope_label: scopeDraft.label,
    });
    if (result.kind !== "acknowledged") {
      setActionStatus(result.kind);
      finishVisibleWorkstationAction(
        correlationId,
        `scope:${scopeDraft.kind}:${scopeDraft.target_id}`,
        result.kind,
        result.message,
      );
      return;
    }
    const updates = await client.loadUpdates(current.snapshot.revision);
    if (updates.kind !== "updates") {
      setActionStatus("rejected");
      finishVisibleWorkstationAction(
        correlationId,
        `scope:${scopeDraft.kind}:${scopeDraft.target_id}`,
        "failed",
        "Conversation Scope was acknowledged but updates could not be loaded.",
      );
      return;
    }
    const applied = applyWorkspaceUpdates(current.snapshot, updates.batch);
    if (applied.kind !== "applied") {
      setActionStatus("rejected");
      finishVisibleWorkstationAction(
        correlationId,
        `scope:${scopeDraft.kind}:${scopeDraft.target_id}`,
        "failed",
        "Conversation Scope was acknowledged but canonical updates could not be applied.",
      );
      return;
    }
    setState({ ...current, snapshot: applied.snapshot });
    setScopeDraft(null);
    setActionStatus("acknowledged");
    finishVisibleWorkstationAction(
      correlationId,
      `scope:${scopeDraft.kind}:${scopeDraft.target_id}`,
      "acknowledged",
      `Conversation Scope now targets ${scopeDraft.label}.`,
    );
    await refreshWorkingContext();
  }, [beginVisibleWorkstationAction, client, finishVisibleWorkstationAction, refreshWorkingContext, scopeDraft, state]);

  const submitMessage = useCallback(async () => {
    if (
      !draft.trim() ||
      state === "loading" ||
      (state.kind !== "ready" && state.kind !== "empty") ||
      !client.appendConsoleMessage
    ) {
      return;
    }
    const scope = state.snapshot.conversation_scope;
    setMessageStatus("pending");
    const result = await client.appendConsoleMessage({
      role: "user",
      content: draft,
      outcome: "proposed",
      source: "mission-commander",
      expected_revision: state.snapshot.revision,
      scope_kind: scope.kind,
      scope_target: scope.target_id,
      scope_label: scope.label,
    });
    if (result.kind !== "message") {
      setMessageStatus("rejected");
      return;
    }
    setConsoleHistory((messages) => [...messages, result.message]);
    setDraft("");
    setMessageStatus(null);
    await refreshWorkingContext();
  }, [client, draft, refreshWorkingContext, state]);

  const curateWorkingContext = useCallback(
    async (sourceId: string, disposition: "included" | "pinned" | "excluded") => {
      if (!workingContext || !client.curateWorkingContext) return;
      setContextStatus("pending");
      const result = await client.curateWorkingContext({
        source_id: sourceId,
        disposition,
        expected_context_revision: workingContext.revision,
      });
      if (result.kind !== "acknowledged") {
        setContextStatus(result.kind);
        return;
      }
      const reloaded = await refreshWorkingContext();
      setContextStatus(reloaded ? "acknowledged" : "rejected");
    },
    [client, refreshWorkingContext, workingContext],
  );

  const submitReviewDecision = useCallback(
    async (sessionId: string, decision: ReviewDecision, reason: string) => {
      if (
        state === "loading" ||
        (state.kind !== "ready" && state.kind !== "empty") ||
        !client.submitReviewDecision
      ) {
        return;
      }
      const current = state;
      const correlationId = `review-${decision}-${sessionId}-${current.snapshot.revision}`;
      beginVisibleWorkstationAction(
        correlationId,
        `${reviewDecisionLabel(decision)} for ${sessionId}`,
        sessionId,
      );
      setReviewStatus({ state: "pending", message: "Review decision pending" });
      const result = await client.submitReviewDecision({
        correlation_id: correlationId,
        expected_revision: current.snapshot.revision,
        session_id: sessionId,
        decision,
        reason,
      });
      if (result.kind !== "acknowledged") {
        setReviewStatus({ state: result.kind, message: result.message });
        finishVisibleWorkstationAction(correlationId, sessionId, result.kind, result.message);
        return;
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        setReviewStatus({ state: "rejected", message: "Review acknowledged but reload failed" });
        finishVisibleWorkstationAction(
          correlationId,
          sessionId,
          "failed",
          "Review acknowledged but canonical snapshot reload failed.",
        );
        setConnectionStatus("offline");
        return;
      }
      setState(reloaded);
      setConnectionStatus("connected");
      setReviewStatus({
        state: "acknowledged",
        message: result.acknowledgement.effect_summary,
      });
      finishVisibleWorkstationAction(
        correlationId,
        sessionId,
        "acknowledged",
        result.acknowledgement.effect_summary,
      );
      await refreshReviewWorkspace();
    },
    [beginVisibleWorkstationAction, client, finishVisibleWorkstationAction, refreshReviewWorkspace, state],
  );

  const submitWorkspaceQueueDecision = useCallback(
    async (itemId: string, decision: WorkspaceQueueDecision, reason: string) => {
      if (
        !workspaceQueue ||
        state === "loading" ||
        (state.kind !== "ready" && state.kind !== "empty") ||
        !client.submitWorkspaceQueueDecision
      ) {
        return;
      }
      const item = workspaceQueue.items.find((candidate) => candidate.item_id === itemId);
      const actionLabel = `${decision[0].toUpperCase() + decision.slice(1)} ${item?.requested_action ?? itemId}`;
      const correlationId = `queue-${decision}-${itemId}-${workspaceQueue.revision}`;
      setQueueStatus({ state: "pending", message: "Workspace Queue decision pending" });
      beginVisibleWorkstationAction(correlationId, actionLabel, itemId);
      const result = await client.submitWorkspaceQueueDecision({
        correlation_id: correlationId,
        action_type: "workspace-queue-decision",
        actor: "mission-commander",
        expected_revision: workspaceQueue.revision,
        target: {
          kind: "workspace-queue-item",
          id: itemId,
        },
        item_id: itemId,
        decision,
        reason,
      });
      if (result.kind !== "acknowledged") {
        setQueueStatus({ state: result.kind, message: result.message });
        finishVisibleWorkstationAction(correlationId, itemId, result.kind, result.message);
        return;
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        setQueueStatus({ state: "rejected", message: "Queue acknowledged but reload failed" });
        finishVisibleWorkstationAction(
          correlationId,
          itemId,
          "failed",
          "Orchestrator acknowledged the action, but Alfredo could not reload canonical state.",
        );
        setConnectionStatus("offline");
        return;
      }
      setState(reloaded);
      setConnectionStatus("connected");
      setQueueStatus({
        state: "acknowledged",
        message: result.acknowledgement.effect_summary,
      });
      finishVisibleWorkstationAction(
        correlationId,
        itemId,
        "acknowledged",
        result.acknowledgement.effect_summary,
      );
      await refreshWorkspaceQueue();
    },
    [beginVisibleWorkstationAction, client, finishVisibleWorkstationAction, refreshWorkspaceQueue, state, workspaceQueue],
  );

  const submitWorkstationAction = useCallback(
    async (action: WorkstationGovernedAction, draft: WorkstationActionDraftState) => {
      if (
        state === "loading" ||
        (state.kind !== "ready" && state.kind !== "empty") ||
        !client.submitWorkstationAction ||
        !action.actionType ||
        !isExecutableWorkstationAction(action) ||
        typeof action.expectedRevision !== "number" ||
        !action.targetIdentity
      ) {
        return;
      }
      const target = workstationActionRequestTarget(action);
      const targetId = workstationActionTargetId(action);
      if (!target || !targetId) return;
      const correlationId = `workstation-${action.actionType}-${targetId}-${action.expectedRevision}`;
      const label = `${action.label} ${targetId}`;
      const request: WorkstationActionRequest = {
        correlation_id: correlationId,
        action_type: action.actionType,
        actor: "mission-commander",
        expected_revision: action.expectedRevision,
        target,
        issue_id: action.issueId,
        session_id: action.sessionId,
        agent_id: draft.agentId.trim() || undefined,
        reason: draft.reason.trim() || undefined,
        allowed_paths: [],
        command_policy: {},
      };
      beginVisibleWorkstationAction(correlationId, label, targetId);
      const result = await client.submitWorkstationAction(request);
      if (result.kind !== "acknowledged") {
        finishVisibleWorkstationAction(correlationId, targetId, result.kind, result.message);
        return;
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        finishVisibleWorkstationAction(
          correlationId,
          targetId,
          "failed",
          "Orchestrator acknowledged the action, but Alfredo could not reload canonical state.",
        );
        setConnectionStatus("offline");
        return;
      }
      setState(reloaded);
      setConnectionStatus("connected");
      finishVisibleWorkstationAction(
        correlationId,
        targetId,
        "acknowledged",
        result.acknowledgement.effect_summary,
      );
    },
    [beginVisibleWorkstationAction, client, finishVisibleWorkstationAction, state],
  );

  const submitMissionDraftDecision = useCallback(
    async (draftId: string, decision: MissionDraftDecision, reason: string) => {
      if (
        state === "loading" ||
        (state.kind !== "ready" && state.kind !== "empty") ||
        !client.submitMissionDraftDecision ||
        !missionDrafts
      ) {
        return;
      }
      const correlationId = `mission-draft-${decision}-${draftId}-${missionDrafts.revision}`;
      beginVisibleWorkstationAction(
        correlationId,
        `${missionDraftDecisionLabel(decision)} Mission Draft ${draftId}`,
        draftId,
      );
      setMissionDraftStatus({ state: "pending", message: "Submitting Mission Draft decision." });
      const result = await client.submitMissionDraftDecision({
        correlation_id: correlationId,
        expected_revision: missionDrafts.revision,
        draft_id: draftId,
        decision,
        reason,
      });
      if (result.kind === "acknowledged") {
        setMissionDraftStatus({
          state: "acknowledged",
          message: result.acknowledgement.effect_summary,
        });
        finishVisibleWorkstationAction(
          correlationId,
          draftId,
          "acknowledged",
          result.acknowledgement.effect_summary,
        );
        await refreshMissionDrafts();
        const snapshotResult = await client.loadSnapshot();
        setState(snapshotResult);
        return;
      }
      setMissionDraftStatus({ state: result.kind, message: result.message });
      finishVisibleWorkstationAction(correlationId, draftId, result.kind, result.message);
    },
    [beginVisibleWorkstationAction, client, finishVisibleWorkstationAction, missionDrafts, refreshMissionDrafts, state],
  );

  const submitMissionDraftCreate = useCallback(
    async (draft: MissionDraftCreateDraft) => {
      if (
        state === "loading" ||
        (state.kind !== "ready" && state.kind !== "empty") ||
        !client.submitMissionDraftCreate
      ) {
        return;
      }
      const current = state;
      const correlationId = `mission-draft-create-${current.snapshot.revision}`;
      beginVisibleWorkstationAction(
        correlationId,
        `Create Mission Draft ${draft.proposedGoal}`,
        "mission-draft:create",
      );
      setMissionDraftStatus({ state: "pending", message: "Creating Mission Draft." });
      const request: MissionDraftCreateRequest = {
        correlation_id: correlationId,
        expected_revision: current.snapshot.revision,
        proposed_goal: draft.proposedGoal,
        selected_ad_hoc_ids: draft.selectedAdHocIds,
        excluded_ad_hoc_ids: draft.excludedAdHocIds,
        new_work_items: draft.newWorkItems,
        dependencies: draft.dependencies,
        unresolved_decisions: draft.unresolvedDecisions,
        mission_id: current.snapshot.active_mission?.id,
      };
      const result = await client.submitMissionDraftCreate(request);
      if (result.kind === "acknowledged") {
        setMissionDraftStatus({
          state: "acknowledged",
          message: result.acknowledgement.effect_summary,
        });
        finishVisibleWorkstationAction(
          correlationId,
          "mission-draft:create",
          "acknowledged",
          result.acknowledgement.effect_summary,
        );
        await refreshMissionDrafts();
        const snapshotResult = await client.loadSnapshot();
        setState(snapshotResult);
        return;
      }
      setMissionDraftStatus({ state: result.kind, message: result.message });
      finishVisibleWorkstationAction(correlationId, "mission-draft:create", result.kind, result.message);
    },
    [beginVisibleWorkstationAction, client, finishVisibleWorkstationAction, refreshMissionDrafts, state],
  );

  const submitAdHocDelegationProposal = useCallback(
    async (proposal: AdHocDelegationDraft) => {
      if (
        state === "loading" ||
        (state.kind !== "ready" && state.kind !== "empty") ||
        !client.submitAdHocDelegationProposal
      ) {
        return;
      }
      const current = state;
      const scope = current.snapshot.conversation_scope;
      const correlationId = `ad-hoc-delegation-${proposal.originatingMessageId}-${current.snapshot.revision}`;
      beginVisibleWorkstationAction(
        correlationId,
        `Propose Ad Hoc Delegation from ${proposal.originatingMessageId}`,
        proposal.originatingMessageId,
      );
      setQueueStatus({ state: "pending", message: "Ad Hoc Delegation proposal pending" });
      const request: AdHocDelegationProposalRequest = {
        correlation_id: correlationId,
        expected_revision: current.snapshot.revision,
        source: "agent-console",
        scope_kind: scope.kind,
        scope_target: scope.target_id,
        scope_label: scope.label,
        acceptance_criteria: proposal.acceptanceCriteria,
        allowed_paths: proposal.allowedPaths,
        command_policy: proposal.commandPolicy,
        proposed_agent: proposal.proposedAgent,
        originating_message_id: proposal.originatingMessageId,
      };
      const result = await client.submitAdHocDelegationProposal(request);
      if (result.kind !== "acknowledged") {
        setQueueStatus({ state: result.kind, message: result.message });
        finishVisibleWorkstationAction(correlationId, proposal.originatingMessageId, result.kind, result.message);
        return;
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        setQueueStatus({ state: "rejected", message: "Proposal acknowledged but reload failed" });
        finishVisibleWorkstationAction(
          correlationId,
          proposal.originatingMessageId,
          "failed",
          "Ad Hoc Delegation proposal acknowledged but canonical snapshot reload failed.",
        );
        setConnectionStatus("offline");
        return;
      }
      setState(reloaded);
      setConnectionStatus("connected");
      setQueueStatus({
        state: "acknowledged",
        message: result.acknowledgement.effect_summary,
      });
      finishVisibleWorkstationAction(
        correlationId,
        proposal.originatingMessageId,
        "acknowledged",
        result.acknowledgement.effect_summary,
      );
      await refreshWorkspaceQueue();
    },
    [beginVisibleWorkstationAction, client, finishVisibleWorkstationAction, refreshWorkspaceQueue, state],
  );

  if (state === "loading") {
    return (
      <div className="boot-screen" role="status" aria-live="polite">
        <span className="boot-marker" aria-hidden="true" />
        <p>Connecting to Alfredo</p>
        <small>Waiting for an authoritative workspace snapshot</small>
      </div>
    );
  }

  if (state.kind !== "ready" && state.kind !== "empty") {
    return (
      <div className="boot-screen boot-screen--error" role="alert">
        <p>Alfredo workstation unavailable</p>
        <small>{state.message}</small>
        {state.recoverable ? <button onClick={connect}>Retry connection</button> : null}
      </div>
    );
  }

  return (
    <CommandDeck
      snapshot={state.snapshot}
      empty={state.kind === "empty"}
      actionStatus={actionStatus}
      onSelectView={submitView}
      onSwitchMission={submitMissionSwitch}
      connectionStatus={connectionStatus}
      onReconnect={reconnect}
      consoleHistory={consoleHistory}
      draft={draft}
      onDraftChange={setDraft}
      scopeDraft={scopeDraft}
      onScopeDraftChange={setScopeDraft}
      onApplyScope={submitScope}
      onSend={submitMessage}
      messageStatus={messageStatus}
      workingContext={workingContext}
      contextStatus={contextStatus}
      onCurateContext={curateWorkingContext}
      reviewWorkspace={reviewWorkspace}
      reviewStatus={reviewStatus}
      reviewReasons={reviewReasons}
      onReviewReasonChange={(sessionId, reason) =>
        setReviewReasons((current) => ({ ...current, [sessionId]: reason }))
      }
      onReviewDecision={submitReviewDecision}
      workspaceQueue={workspaceQueue}
      missionDrafts={missionDrafts}
      missionDraftStatus={missionDraftStatus}
      missionDraftReasons={missionDraftReasons}
      activityJournal={activityJournal}
      activityFilters={activityFilters}
      activityStatus={activityStatus}
      onActivityFilterChange={setActivityFilters}
      onActivityRefresh={() => void refreshActivityJournal(activityFilters)}
      queueStatus={queueStatus}
      latestConsoleMessage={consoleHistory.at(-1) ?? null}
      queueReasons={queueReasons}
      onQueueReasonChange={(itemId, reason) =>
        setQueueReasons((current) => ({ ...current, [itemId]: reason }))
      }
      onQueueDecision={submitWorkspaceQueueDecision}
      workstationActionTurns={workstationActionTurns}
      workstationActionState={workstationActionState}
      workstationActionDrafts={workstationActionDrafts}
      onWorkstationActionDraftChange={(key, draft) =>
        setWorkstationActionDrafts((current) => ({ ...current, [key]: draft }))
      }
      onWorkstationAction={submitWorkstationAction}
      onAdHocProposal={submitAdHocDelegationProposal}
      onMissionDraftCreate={submitMissionDraftCreate}
      onMissionDraftReasonChange={(draftId, reason) =>
        setMissionDraftReasons((current) => ({ ...current, [draftId]: reason }))
      }
      onMissionDraftDecision={submitMissionDraftDecision}
      commandAuditOpen={commandAuditOpen}
      onCommandAuditOpenChange={setCommandAuditOpen}
      shellTerminal={shellTerminal}
      launchContext={launchContext}
    />
  );
}

interface WorkstationTranscriptTurn {
  readonly id: string;
  readonly content: string;
  readonly source: string;
  readonly outcome: string;
}

interface CommandConsoleTurn {
  readonly id: string;
  readonly commandId: string;
  readonly record: ShellTerminalCommandRecord | null;
  readonly command: string;
  readonly purpose: string;
  readonly workingDirectory: string;
  readonly requestedPaths: readonly string[];
  readonly accessLevel: PathAccessLevel;
  readonly requester: string;
  readonly classification: ShellTerminalClassification;
  readonly status: ShellTerminalCommandStatus;
  readonly approvalState: string;
  readonly exitCode: number | null;
  readonly summary: string;
  readonly stdout: string;
  readonly stderr: string;
}

function buildWorkstationTranscriptTurns(
  snapshot: WorkspaceSnapshot,
): readonly WorkstationTranscriptTurn[] {
  const attentionTurns =
    snapshot.missions?.flatMap((mission) =>
      mission.attention.map((attention) => ({
        id: `attention:${attention.attention_id}`,
        content: `Workstation action pending: ${attention.label}.`,
        source: "orchestrator",
        outcome: "waiting-approval",
      })),
    ) ?? [];
  const sessionTurns =
    snapshot.missions?.flatMap((mission) =>
      mission.sessions.map((session) => ({
        id: `session:${session.session_id}`,
        content: `Workstation outcome: ${session.issue_id} is ${session.status} on ${session.assigned_agent}.`,
        source: session.assigned_agent,
        outcome: session.status,
      })),
    ) ?? [];
  return [...attentionTurns, ...sessionTurns];
}

function buildCommandConsoleTurns(
  terminal: ShellTerminalController,
): readonly CommandConsoleTurn[] {
  const recordsByCommandId = new Map(
    terminal.projection?.commands.map((command) => [command.command_id, command]) ?? [],
  );
  const resultsByCommandId = new Map(
    terminal.transcript.map((entry) => [entry.command_id, entry]),
  );
  const commandIds = new Set([...recordsByCommandId.keys(), ...resultsByCommandId.keys()]);
  return [...commandIds].map((commandId) =>
    commandConsoleTurn(
      commandId,
      recordsByCommandId.get(commandId),
      resultsByCommandId.get(commandId),
      terminal.workingDirectory,
      terminal.accessLevel,
    ),
  );
}

function commandConsoleTurn(
  commandId: string,
  record: ShellTerminalCommandRecord | undefined,
  result: ShellTerminalTranscriptEntry | undefined,
  fallbackWorkingDirectory: string,
  fallbackAccessLevel: PathAccessLevel,
): CommandConsoleTurn {
  const classification = result?.classification ?? record?.classification ?? "auto-allowed";
  const status = result?.status ?? record?.status ?? "pending-approval";
  const stdout = result?.stdout ?? "";
  const stderr = result?.stderr ?? "";
  return {
    id: `command:${commandId}`,
    commandId,
    record: record ?? null,
    command: record?.command ?? result?.command ?? "Unknown command",
    purpose: commandPurpose(record),
    workingDirectory: (record?.working_directory ?? fallbackWorkingDirectory) || "Current workspace",
    requestedPaths: record?.requested_paths ?? [],
    accessLevel: record?.access_level ?? fallbackAccessLevel,
    requester: record?.requester ?? "mission-commander",
    classification,
    status,
    approvalState: commandApprovalState(record, classification, status),
    exitCode: result?.exit_code ?? record?.exit_code ?? null,
    summary: commandOutputSummary(status, result?.exit_code ?? record?.exit_code ?? null, stdout, stderr),
    stdout,
    stderr,
  };
}

function commandPurpose(record: ShellTerminalCommandRecord | undefined): string {
  if (record?.reason.trim()) return record.reason;
  if (record?.requester) return `Requested by ${record.requester}.`;
  return "Purpose not provided.";
}

function commandApprovalState(
  record: ShellTerminalCommandRecord | undefined,
  classification: ShellTerminalClassification,
  status: ShellTerminalCommandStatus,
): string {
  if (status === "pending-approval") {
    if (classification === "human-required") return "Waiting for Mission Commander approval";
    if (classification === "frontier-approvable") return "Waiting for Frontier Model approval";
    return "Policy check pending";
  }
  if (status === "denied") return record?.decider ? `Denied by ${record.decider}` : "Denied";
  if (classification === "auto-allowed") return "Auto-allowed by command policy";
  return record?.approver ? `Approved by ${record.approver}` : "Approved by command policy";
}

function commandOutputSummary(
  status: ShellTerminalCommandStatus,
  exitCode: number | null,
  stdout: string,
  stderr: string,
): string {
  const stdoutLines = countOutputLines(stdout);
  const stderrLines = countOutputLines(stderr);
  if (stdoutLines || stderrLines) {
    const parts = [
      stdoutLines ? `${stdoutLines} stdout ${stdoutLines === 1 ? "line" : "lines"}` : "",
      stderrLines ? `${stderrLines} stderr ${stderrLines === 1 ? "line" : "lines"}` : "",
    ].filter(Boolean);
    return `Captured ${parts.join(" and ")}; inspect full output for terminal bytes.`;
  }
  if (status === "pending-approval") return "Command is waiting for approval before execution.";
  if (status === "completed") return `Completed${exitCode === null ? "" : ` with exit ${exitCode}`} and no output.`;
  if (status === "failed") return `Failed${exitCode === null ? "" : ` with exit ${exitCode}`} and no captured output.`;
  return "Command did not produce captured output.";
}

function countOutputLines(output: string): number {
  return output.split(/\r?\n/).filter((line) => line.trim()).length;
}

function reviewDecisionLabel(decision: ReviewDecision): string {
  if (decision === "accept") return "Accept evidence";
  if (decision === "repair") return "Request repair";
  return "Escalate human review";
}

function missionDraftDecisionLabel(decision: MissionDraftDecision): string {
  return decision === "confirm" ? "Confirm" : "Abandon";
}

function snapshotExecutionState(snapshot: WorkspaceSnapshot): string {
  const attention = snapshot.missions?.flatMap((mission) => mission.attention) ?? [];
  if (attention.length > 0) return "Waiting approval";
  const sessions = snapshot.missions?.flatMap((mission) => mission.sessions) ?? [];
  const active = sessions.find((session) => !isDoneStatus(session.status));
  return active ? `Session ${active.status}` : "Idle";
}

function isDoneStatus(status: string): boolean {
  const normalized = status.toLowerCase();
  return (
    normalized.includes("complete") ||
    normalized.includes("done") ||
    normalized.includes("failed") ||
    normalized.includes("merged")
  );
}

function CommandDeck({
  snapshot,
  empty,
  actionStatus,
  onSelectView,
  onSwitchMission,
  connectionStatus,
  onReconnect,
  consoleHistory,
  draft,
  onDraftChange,
  scopeDraft,
  onScopeDraftChange,
  onApplyScope,
  onSend,
  messageStatus,
  workingContext,
  contextStatus,
  onCurateContext,
  reviewWorkspace,
  reviewStatus,
  reviewReasons,
  onReviewReasonChange,
  onReviewDecision,
  workspaceQueue,
  missionDrafts,
  missionDraftStatus,
  missionDraftReasons,
  activityJournal,
  activityFilters,
  activityStatus,
  onActivityFilterChange,
  onActivityRefresh,
  queueStatus,
  latestConsoleMessage,
  queueReasons,
  onQueueReasonChange,
  onQueueDecision,
  workstationActionTurns,
  workstationActionState,
  workstationActionDrafts,
  onWorkstationActionDraftChange,
  onWorkstationAction,
  onAdHocProposal,
  onMissionDraftCreate,
  onMissionDraftReasonChange,
  onMissionDraftDecision,
  commandAuditOpen,
  onCommandAuditOpenChange,
  shellTerminal,
  launchContext,
}: {
  snapshot: WorkspaceSnapshot;
  empty: boolean;
  actionStatus: "pending" | "acknowledged" | "stale" | "rejected" | null;
  onSelectView: (view: string) => void;
  onSwitchMission: (missionId: string) => void;
  connectionStatus: "connected" | "offline" | "reconnecting";
  onReconnect: () => void;
  consoleHistory: readonly AgentConsoleMessage[];
  draft: string;
  onDraftChange: (draft: string) => void;
  scopeDraft: ConversationScope | null;
  onScopeDraftChange: (scope: ConversationScope) => void;
  onApplyScope: () => void;
  onSend: () => void;
  messageStatus: "pending" | "rejected" | null;
  workingContext: WorkingContextProjection | null;
  contextStatus: "pending" | "acknowledged" | "stale" | "rejected" | null;
  onCurateContext: (
    sourceId: string,
    disposition: "included" | "pinned" | "excluded",
  ) => void;
  reviewWorkspace: ReviewWorkspaceProjection | null;
  reviewStatus: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  reviewReasons: Record<string, string>;
  onReviewReasonChange: (sessionId: string, reason: string) => void;
  onReviewDecision: (sessionId: string, decision: ReviewDecision, reason: string) => void;
  workspaceQueue: WorkspaceQueueProjection | null;
  missionDrafts: MissionDraftProjection | null;
  missionDraftStatus: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  missionDraftReasons: Record<string, string>;
  activityJournal: ActivityJournalProjection | null;
  activityFilters: ActivityJournalFilters;
  activityStatus: "pending" | "rejected" | null;
  onActivityFilterChange: (filters: ActivityJournalFilters) => void;
  onActivityRefresh: () => void;
  queueStatus: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  latestConsoleMessage: AgentConsoleMessage | null;
  queueReasons: Record<string, string>;
  onQueueReasonChange: (itemId: string, reason: string) => void;
  onQueueDecision: (itemId: string, decision: WorkspaceQueueDecision, reason: string) => void;
  workstationActionTurns: readonly WorkstationActionTurn[];
  workstationActionState: WorkstationActionState | null;
  workstationActionDrafts: Record<string, WorkstationActionDraftState>;
  onWorkstationActionDraftChange: (key: string, draft: WorkstationActionDraftState) => void;
  onWorkstationAction: (
    action: WorkstationGovernedAction,
    draft: WorkstationActionDraftState,
  ) => void;
  onAdHocProposal: (proposal: AdHocDelegationDraft) => void;
  onMissionDraftCreate: (draft: MissionDraftCreateDraft) => void;
  onMissionDraftReasonChange: (draftId: string, reason: string) => void;
  onMissionDraftDecision: (
    draftId: string,
    decision: MissionDraftDecision,
    reason: string,
  ) => void;
  commandAuditOpen: boolean;
  onCommandAuditOpenChange: (open: boolean) => void;
  shellTerminal: ShellTerminalController;
  launchContext: AlfredoLaunchContext | null;
}) {
  const mission = snapshot.active_mission;
  const missions = snapshot.missions?.length
    ? snapshot.missions
    : mission
      ? [
          {
            id: mission.id,
            title: mission.title,
            issue_count: mission.issue_count,
            is_active: true,
            sessions: [],
            attention: [],
          },
        ]
      : [];
  const viewTitle: Record<string, string> = {
    "mission-board": "Mission Board",
    "review-workspace": "Review Workspace",
    "workspace-queue": "Workspace Queue",
    activity: "Activity",
  };
  const activeViewTitle = viewTitle[snapshot.operations_view] ?? "Mission Board";
  const workingDirectoryLabel = snapshot.workspace_session.workspace_path.split(/[\\/]/).filter(Boolean).at(-1) ?? "Working directory";
  const scopeOptions: ConversationScope[] = [
    {
      kind: "working-directory",
      target_id: snapshot.workspace_session.workspace_path,
      label: workingDirectoryLabel,
    },
    ...(mission
      ? [{ kind: "mission" as const, target_id: mission.id, label: mission.title }]
      : []),
    ...snapshot.mission_board.ordered_issue_ids.map((issueId) => ({
      kind: "issue-slice" as const,
      target_id: issueId,
      label:
        snapshot.conversation_scope.kind === "issue-slice" &&
        snapshot.conversation_scope.target_id === issueId
          ? snapshot.conversation_scope.label
          : issueId,
    })),
  ];
  const selectedScope = scopeDraft ?? snapshot.conversation_scope;
  const scopeValue = `${selectedScope.kind}:${selectedScope.target_id}`;
  const contextCounts = workingContext?.sources.reduce<Record<string, number>>(
    (counts, source) => ({ ...counts, [source.kind]: (counts[source.kind] ?? 0) + 1 }),
    {},
  );
  const issueSlicesById = new Map(
    snapshot.mission_board.issue_slices?.map((issue) => [issue.issue_id, issue]),
  );
  const workstationContinuityKey = workstationContinuityStorageKey(snapshot);
  const [selectedIssueId, setSelectedIssueId] = useState<string | null>(null);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [expandedWorkstationCardIds, setExpandedWorkstationCardIds] = useState<readonly string[]>([]);
  const [pinnedWorkstationCardIds, setPinnedWorkstationCardIds] = useState<readonly string[]>([]);
  const [workstationFilter, setWorkstationFilter] = useState("");
  const [workstationSort, setWorkstationSort] = useState<"priority" | "name" | "status">("priority");
  const [selectedWorkstationSessionId, setSelectedWorkstationSessionId] = useState<string | null>(null);
  const [selectedWorkstationDiff, setSelectedWorkstationDiff] = useState<WorkstationDiffLink | null>(null);
  useEffect(() => {
    if (selectedIssueId) document.getElementById("issue-slice-inspector")?.focus();
  }, [selectedIssueId]);
  const selectedIssue =
    (selectedIssueId ? issueSlicesById.get(selectedIssueId) : null) ??
    snapshot.mission_board.issue_slices?.[0] ??
    null;
  const workstationProjection = projectWorkstationCards(snapshot, {
    workspaceQueue,
    pendingIntent:
      actionStatus === "pending"
        ? {
            id: `workspace-action-${snapshot.revision}`,
            label: "Awaiting Orchestrator acknowledgement",
            expectedRevision: snapshot.revision,
          }
        : null,
  });
  const workstationCards = workstationProjection.groups.flatMap((group) => group.cards);
  const visibleWorkstationGroups = filterAndSortWorkstationGroups(
    workstationProjection.groups,
    workstationFilter,
    workstationSort,
    pinnedWorkstationCardIds,
  );
  const hydratedWorkstationContinuityKey = useRef<string | null>(null);
  const skipWorkstationContinuityWrite = useRef(false);
  useEffect(() => {
    if (hydratedWorkstationContinuityKey.current === workstationContinuityKey) return;
    const restored = readWorkstationContinuity(workstationContinuityKey);
    const cardIds = new Set(workstationCards.map((card) => card.id));
    const sessionIds = new Set(
      workstationCards.flatMap((card) => (card.sessionId ? [card.sessionId] : [])),
    );
    const issueIds = new Set(snapshot.mission_board.ordered_issue_ids);
    if (restored) {
      skipWorkstationContinuityWrite.current = true;
      onCommandAuditOpenChange(restored.commandAuditOpen);
      setSelectedIssueId(
        restored.selectedIssueId && issueIds.has(restored.selectedIssueId)
          ? restored.selectedIssueId
          : null,
      );
      setSelectedSessionId(
        restored.selectedSessionId && sessionIds.has(restored.selectedSessionId)
          ? restored.selectedSessionId
          : null,
      );
      setExpandedWorkstationCardIds(
        restored.expandedWorkstationCardIds.filter((cardId) => cardIds.has(cardId)),
      );
      setPinnedWorkstationCardIds(
        restored.pinnedWorkstationCardIds.filter((cardId) => cardIds.has(cardId)),
      );
      setWorkstationFilter(restored.workstationFilter);
      setWorkstationSort(restored.workstationSort);
      setSelectedWorkstationSessionId(
        restored.selectedWorkstationSessionId &&
          sessionIds.has(restored.selectedWorkstationSessionId)
          ? restored.selectedWorkstationSessionId
          : null,
      );
      setSelectedWorkstationDiff(
        restored.selectedWorkstationDiff &&
          sessionIds.has(restored.selectedWorkstationDiff.sessionId)
          ? restored.selectedWorkstationDiff
          : null,
      );
    }
    hydratedWorkstationContinuityKey.current = workstationContinuityKey;
  }, [onCommandAuditOpenChange, snapshot.mission_board.ordered_issue_ids, workstationCards, workstationContinuityKey]);
  useEffect(() => {
    if (hydratedWorkstationContinuityKey.current !== workstationContinuityKey) return;
    if (skipWorkstationContinuityWrite.current) {
      skipWorkstationContinuityWrite.current = false;
      return;
    }
    writeWorkstationContinuity(workstationContinuityKey, {
      schema_version: WORKSTATION_CONTINUITY_SCHEMA_VERSION,
      commandAuditOpen,
      selectedIssueId,
      selectedSessionId,
      expandedWorkstationCardIds,
      pinnedWorkstationCardIds,
      workstationFilter,
      workstationSort,
      selectedWorkstationSessionId,
      selectedWorkstationDiff,
    });
  }, [
    expandedWorkstationCardIds,
    commandAuditOpen,
    pinnedWorkstationCardIds,
    selectedIssueId,
    selectedSessionId,
    selectedWorkstationDiff,
    selectedWorkstationSessionId,
    workstationContinuityKey,
    workstationFilter,
    workstationSort,
  ]);
  const workstationTranscriptTurns = [
    ...buildWorkstationTranscriptTurns(snapshot),
    ...workstationActionTurns,
  ];
  const commandConsoleTurns = buildCommandConsoleTurns(shellTerminal);
  const contextualGrantRequest = shellTerminal.contextualGrantRequest;
  const activeExecutionState =
    actionStatus === "pending"
      ? "Action pending"
      : reviewStatus?.state === "pending"
        ? "Review pending"
        : queueStatus?.state === "pending"
          ? "Queue pending"
          : missionDraftStatus?.state === "pending"
            ? "Mission Draft pending"
            : shellTerminal.actionStatus?.state === "pending"
              ? "Command pending"
              : snapshotExecutionState(snapshot);
  const toggleExpandedWorkstationCard = (cardId: string) => {
    setExpandedWorkstationCardIds((current) =>
      current.includes(cardId)
        ? current.filter((id) => id !== cardId)
        : [...current, cardId],
    );
  };
  const togglePinnedWorkstationCard = (cardId: string) => {
    setPinnedWorkstationCardIds((current) =>
      current.includes(cardId)
        ? current.filter((id) => id !== cardId)
        : [...current, cardId],
    );
  };
  return (
    <div className="command-deck">
      <header className="topbar">
        <div className="wordmark">
          <span className="wordmark__signal" aria-hidden="true" />
          <span>ALFREDO</span>
          <small>WORKSTATION</small>
        </div>
        <div className="session-state">
          <span className="eyebrow">Workspace Session {snapshot.workspace_session.id}</span>
          <strong>{snapshot.workspace_session.workspace_path}</strong>
        </div>
        <span className="revision">STATE / {snapshot.revision.toString().padStart(4, "0")}</span>
      </header>

      <div className="deck-grid">
        <main className="prompt-workspace" aria-label="Prompt Workstation">
          <section className="prompt-pane" aria-label="Prompt Transcript">
            <div className="panel-heading">
              <div>
                <span className="eyebrow">Agent Console / {mission?.id ?? "none"}</span>
                <h1>{mission?.title ?? "No active mission"}</h1>
              </div>
              {connectionStatus === "offline" ? (
                <button type="button" onClick={onReconnect}>Reconnect</button>
              ) : null}
            </div>

            <div className="console-history">
              {consoleHistory.length === 0 &&
              workstationTranscriptTurns.length === 0 &&
              commandConsoleTurns.length === 0 ? (
                <p className="system-line">Canonical workspace state restored.</p>
              ) : null}
              {consoleHistory.map((message) => (
                <article key={message.message_id} data-outcome={message.outcome}>
                  <p>{message.content}</p>
                  <small>{message.source} / {message.outcome}</small>
                </article>
              ))}
              {workstationTranscriptTurns.map((turn) => (
                <article key={turn.id} data-outcome={turn.outcome}>
                  <p>{turn.content}</p>
                  <small>{turn.source} / {turn.outcome}</small>
                </article>
              ))}
              {commandConsoleTurns.map((turn) => (
                <CommandConsoleCard
                  key={turn.id}
                  turn={turn}
                  actionPending={shellTerminal.actionStatus?.state === "pending"}
                  denialReason={shellTerminal.denialReasons[turn.commandId] ?? ""}
                  onDenialReasonChange={(reason) =>
                    shellTerminal.setDenialReason(turn.commandId, reason)
                  }
                  onDecide={(decision) => {
                    if (turn.record) void shellTerminal.decide(turn.record, decision);
                  }}
                />
              ))}
              {contextualGrantRequest ? (
                <PathGrantConsolePrompt
                  request={contextualGrantRequest}
                  actionPending={shellTerminal.actionStatus?.state === "pending"}
                  onGrant={() =>
                    void shellTerminal.createGrantForRequest(contextualGrantRequest.requestId)
                  }
                  onDeny={() =>
                    shellTerminal.denyGrantRequest(contextualGrantRequest.requestId)
                  }
                />
              ) : null}
            </div>

            <div className="scope-card">
              <span className="eyebrow">Conversation Scope / {snapshot.conversation_scope.kind}</span>
              <strong>{snapshot.conversation_scope.label}</strong>
              <code>{snapshot.conversation_scope.target_id}</code>
              <select
                aria-label="Conversation Scope"
                value={scopeValue}
                onChange={(event) => {
                  const next = scopeOptions.find(
                    (scope) => `${scope.kind}:${scope.target_id}` === event.target.value,
                  );
                  if (next) onScopeDraftChange(next);
                }}
              >
                {scopeOptions.map((scope) => (
                  <option key={`${scope.kind}:${scope.target_id}`} value={`${scope.kind}:${scope.target_id}`}>
                    {scope.kind === "working-directory" ? "Working directory" : scope.label}
                  </option>
                ))}
              </select>
              <button
                type="button"
                onClick={onApplyScope}
                disabled={
                  !scopeDraft ||
                  (scopeDraft.kind === snapshot.conversation_scope.kind &&
                    scopeDraft.target_id === snapshot.conversation_scope.target_id)
                }
              >
                Apply scope
              </button>
            </div>

            {workingContext ? (
              <section className="context-inspector" aria-label="Context Inspector">
                <div className="context-inspector__heading">
                  <div>
                    <span className="eyebrow">Bounded model input</span>
                    <h3>Context Inspector</h3>
                  </div>
                  <code>{workingContext.content_character_count} / 4000 chars</code>
                </div>
                <div className="context-inspector__counts">
                  {Object.entries(contextCounts ?? {}).map(([kind, count]) => (
                    <small key={kind}>{count} {kind} {count === 1 ? "source" : "sources"}</small>
                  ))}
                </div>
                <div className="context-inspector__sources">
                  {workingContext.sources.map((source) => (
                    <article key={source.source_id} data-disposition={source.disposition}>
                      <small>{source.kind}</small>
                      <strong>{source.label}</strong>
                      <p>{source.content}</p>
                      <code>{source.disposition}</code>
                      {source.governed ? (
                        <span>Governed / required</span>
                      ) : (
                        <div className="context-inspector__actions">
                          <button
                            type="button"
                            aria-label={`Pin ${source.label}`}
                            disabled={contextStatus === "pending" || source.disposition === "pinned"}
                            onClick={() => onCurateContext(source.source_id, "pinned")}
                          >Pin</button>
                          <button
                            type="button"
                            aria-label={`Exclude ${source.label}`}
                            disabled={contextStatus === "pending" || source.disposition === "excluded"}
                            onClick={() => onCurateContext(source.source_id, "excluded")}
                          >Exclude</button>
                          {source.disposition !== "included" ? (
                            <button
                              type="button"
                              aria-label={`Include ${source.label}`}
                              disabled={contextStatus === "pending"}
                              onClick={() => onCurateContext(source.source_id, "included")}
                            >Include</button>
                          ) : null}
                        </div>
                      )}
                    </article>
                  ))}
                </div>
                {contextStatus ? (
                  <span role="status" aria-label="Context curation status">
                    {contextStatus[0].toUpperCase() + contextStatus.slice(1)}
                  </span>
                ) : null}
              </section>
            ) : null}

            <div className="prompt-composer-dock" role="region" aria-label="Prompt Composer">
              <div className="prompt-status-line" aria-label="Prompt status line">
                <span role="status" aria-label="Connection status">
                  Connection {connectionStatus[0].toUpperCase() + connectionStatus.slice(1)}
                </span>
                <span>Controller {launchContext?.selected_agent || "default"}</span>
                <span>Model {launchContext?.selected_model || "default"}</span>
                <span>Conversation Scope {snapshot.conversation_scope.label}</span>
                <span>Workspace {snapshot.workspace_session.workspace_path}</span>
                <span>Runtime {launchContext?.runtime_root || "backend default"}</span>
                <span role="status" aria-label="Execution status" aria-live="polite">
                  Execution {activeExecutionState}
                </span>
                {launchContext?.recent_workspaces.length ? (
                  <span>{launchContext.recent_workspaces.length} recent workspaces</span>
                ) : null}
              </div>
              <label className="composer prompt-composer">
                <span className="sr-only">Message Alfredo</span>
                <textarea
                  aria-label="Message Alfredo"
                  placeholder="Steer the active scope..."
                  rows={3}
                  value={draft}
                  onChange={(event) => onDraftChange(event.target.value)}
                />
                <button
                  type="button"
                  aria-label="Send prompt"
                  disabled={!draft.trim() || messageStatus === "pending"}
                  onClick={onSend}
                >
                  Send
                </button>
                {messageStatus ? (
                  <span role="status" aria-label="Message status">{messageStatus}</span>
                ) : null}
              </label>
            </div>
          </section>
        </main>

        <aside className="agent-workstations" aria-label="Mission Work">
          <div className="agent-workstations__heading">
            <div>
              <span className="eyebrow">Persistent supervision</span>
              <h2>Mission Work</h2>
            </div>
            <span className="connection-pill">
              {workstationCards.length} streams
            </span>
          </div>
          <section className="workstation-cards" aria-label="Workstation Cards">
            <div className="mission-work-section-heading">
              <div>
                <span className="eyebrow">Live supervision</span>
                <h3>Active Workstations</h3>
              </div>
              <button
                type="button"
                aria-expanded={commandAuditOpen}
                onClick={() => onCommandAuditOpenChange(!commandAuditOpen)}
              >
                {commandAuditOpen ? "Close command audit" : "Open command audit"}
              </button>
            </div>
            <div className="workstation-card-controls">
              <label>
                <span>Filter</span>
                <input
                  type="search"
                  aria-label="Filter workstation cards"
                  value={workstationFilter}
                  onChange={(event) => setWorkstationFilter(event.target.value)}
                />
              </label>
              <label>
                <span>Sort</span>
                <select
                  aria-label="Sort workstation cards"
                  value={workstationSort}
                  onChange={(event) =>
                    setWorkstationSort(event.target.value as "priority" | "name" | "status")
                  }
                >
                  <option value="priority">Priority</option>
                  <option value="name">Name</option>
                  <option value="status">Status</option>
                </select>
              </label>
            </div>
            {selectedWorkstationDiff ? (
              <div
                className="workstation-local-selection"
                role="status"
                aria-label="Selected workstation diff"
              >
                Diff opened locally: {selectedWorkstationDiff.path}
                <small>{selectedWorkstationDiff.sessionId}</small>
              </div>
            ) : null}
            {workstationProjection.pendingIntent ? (
              <div className="workstation-pending" role="status" aria-label="Pending workstation intent">
                <span>{workstationProjection.pendingIntent.label}</span>
                <small>
                  Expected revision {workstationProjection.pendingIntent.expectedRevision}
                </small>
              </div>
            ) : null}
            {visibleWorkstationGroups.map((group) => (
              <div className="workstation-card-group" key={group.id}>
                <div className="workstation-card-group__heading">
                  <span>{group.label}</span>
                  <small>{group.cards.length}</small>
                </div>
                {group.cards.map((card) => (
                  <WorkstationCard
                    key={card.id}
                    card={card}
                    expanded={expandedWorkstationCardIds.includes(card.id)}
                    pinned={pinnedWorkstationCardIds.includes(card.id)}
                    selectedSessionId={selectedWorkstationSessionId}
                    onToggleExpanded={() => toggleExpandedWorkstationCard(card.id)}
                    onTogglePinned={() => togglePinnedWorkstationCard(card.id)}
                    onSelectSession={setSelectedWorkstationSessionId}
                    onOpenDiff={setSelectedWorkstationDiff}
                    queueReasons={queueReasons}
                    onQueueReasonChange={onQueueReasonChange}
                    onQueueDecision={onQueueDecision}
                    workstationActionDrafts={workstationActionDrafts}
                    onWorkstationActionDraftChange={onWorkstationActionDraftChange}
                    onWorkstationAction={onWorkstationAction}
                    actionState={workstationActionState}
                  />
                ))}
              </div>
            ))}
            {visibleWorkstationGroups.length === 0 ? (
              <div className="workstation-local-selection">
                No workstation cards match the current filter.
              </div>
            ) : null}
          </section>
          <div className="workstation-panel">
            <section className="operations" aria-label="Workstation Detail Views">
                <nav className="view-rail" aria-label="Workstation detail views">
                  <button
                    aria-current={snapshot.operations_view === "mission-board" ? "page" : undefined}
                    className={snapshot.operations_view === "mission-board" ? "is-active" : undefined}
                    type="button"
                    onClick={() => onSelectView("mission-board")}
                    disabled={actionStatus === "pending"}
                  >
                    Mission Board
                  </button>
                  <button
                    aria-current={snapshot.operations_view === "review-workspace" ? "page" : undefined}
                    className={snapshot.operations_view === "review-workspace" ? "is-active" : undefined}
                    type="button"
                    onClick={() => onSelectView("review-workspace")}
                    disabled={actionStatus === "pending"}
                  >Review</button>
                  <button
                    aria-current={snapshot.operations_view === "workspace-queue" ? "page" : undefined}
                    className={snapshot.operations_view === "workspace-queue" ? "is-active" : undefined}
                    type="button"
                    onClick={() => onSelectView("workspace-queue")}
                    disabled={actionStatus === "pending"}
                  >Queue</button>
                  <button
                    aria-current={snapshot.operations_view === "activity" ? "page" : undefined}
                    className={snapshot.operations_view === "activity" ? "is-active" : undefined}
                    type="button"
                    onClick={() => onSelectView("activity")}
                    disabled={actionStatus === "pending"}
                  >Activity</button>
                </nav>

                {actionStatus ? (
                  <span role="status" aria-label="Action status" className="connection-pill">
                    {actionStatus[0].toUpperCase() + actionStatus.slice(1)}
                  </span>
                ) : null}

                <section className="mission-surface">
                  {snapshot.operations_view === "review-workspace" ? (
                    <ReviewWorkspace
                      projection={reviewWorkspace}
                      status={reviewStatus}
                      reasons={reviewReasons}
                      onReasonChange={onReviewReasonChange}
                      onDecision={onReviewDecision}
                    />
                  ) : snapshot.operations_view === "workspace-queue" ? (
                    <WorkspaceQueue
                      projection={workspaceQueue}
                      missionDrafts={missionDrafts}
                      missionDraftStatus={missionDraftStatus}
                      missionDraftReasons={missionDraftReasons}
                      status={queueStatus}
                      latestConsoleMessage={latestConsoleMessage}
                      reasons={queueReasons}
                      onReasonChange={onQueueReasonChange}
                      onDecision={onQueueDecision}
                      onAdHocProposal={onAdHocProposal}
                      onMissionDraftCreate={onMissionDraftCreate}
                      onMissionDraftReasonChange={onMissionDraftReasonChange}
                      onMissionDraftDecision={onMissionDraftDecision}
                    />
                  ) : snapshot.operations_view === "activity" ? (
                    <ActivityJournal
                      projection={activityJournal}
                      filters={activityFilters}
                      status={activityStatus}
                      onFilterChange={onActivityFilterChange}
                      onRefresh={onActivityRefresh}
                    />
                  ) : snapshot.operations_view !== "mission-board" ? (
                    <div className="empty-state">
                      <span className="eyebrow">Restored detail view</span>
                      <h1>{activeViewTitle}</h1>
                      <p>This workspace is restored from acknowledged Orchestrator preferences.</p>
                    </div>
                  ) : (
                    <>
                      <div className="mission-heading">
                        <div>
                          <span className="eyebrow">Active Mission / {mission?.id ?? "none"}</span>
                          <h2>Mission Board</h2>
                          <small>{mission?.title ?? "No active mission"}</small>
                        </div>
                        <div className="mission-count">
                          <strong>{mission?.issue_count ?? 0}</strong>
                          <span>Issue Slices</span>
                        </div>
                      </div>

                      {missions.length > 0 ? (
                        <div className="mission-switcher" aria-label="Mission Selector">
                          <label>
                            <span className="eyebrow">Active Mission</span>
                            <select
                              aria-label="Active Mission"
                              value={mission?.id ?? ""}
                              disabled={actionStatus === "pending" || !mission}
                              onChange={(event) => onSwitchMission(event.target.value)}
                            >
                              {missions.map((item) => (
                                <option key={item.id} value={item.id}>
                                  {item.title}
                                </option>
                              ))}
                            </select>
                          </label>
                          <div className="mission-catalog" aria-label="Mission Catalog">
                            {missions.map((item) => {
                              const activeSessions = item.sessions.filter(
                                (session) => session.status !== "complete" && session.status !== "failed",
                              ).length;
                              return (
                                <article key={item.id} data-active={item.is_active}>
                                  <div>
                                    <strong>{item.title}</strong>
                                    <small>{item.is_active ? "Active" : "Background"}</small>
                                  </div>
                                  <span>
                                    {activeSessions} active {activeSessions === 1 ? "session" : "sessions"}
                                  </span>
                                  {item.attention.map((attention) => (
                                    <a
                                      key={attention.attention_id}
                                      href={`#${attention.queue_link.split("#").at(1) ?? ""}`}
                                    >
                                      {attention.label}
                                    </a>
                                  ))}
                                </article>
                              );
                            })}
                          </div>
                        </div>
                      ) : null}

                      {empty ? (
                        <div className="empty-state">
                          <span className="empty-state__glyph" aria-hidden="true">＋</span>
                          <h2>Workspace is ready</h2>
                          <p>No Issue Slices exist yet. Add a tracker issue to begin mission operations.</p>
                        </div>
                      ) : (
                        <div className="mission-board-layout">
                          <div className="mission-progress" aria-label="Mission Progress">
                            <strong>
                              {snapshot.mission_board.ready_issue_ids.length} / {snapshot.mission_board.issue_count}
                            </strong>
                            <span>launch eligible</span>
                          </div>
                          <div className="issue-graph" role="region" aria-label="Issue Graph">
                            {snapshot.mission_board.ordered_issue_ids.map((issueId, index) => {
                              const issue = issueSlicesById.get(issueId);
                              const ready =
                                issue?.launch_eligible ?? snapshot.mission_board.ready_issue_ids.includes(issueId);
                              const blockers = issue?.blockers ?? [];
                              const lifecycle = issue?.lifecycle ?? (ready ? "Ready" : "Blocked");
                              return (
                                <article
                                  className="issue-node"
                                  key={issueId}
                                  data-selected={selectedIssue?.issue_id === issueId}
                                >
                                  <span className="issue-node__index">{String(index + 1).padStart(2, "0")}</span>
                                  <div>
                                    <strong>{issueId}</strong>
                                    <small>{issue?.title ?? (ready ? "Launch eligible" : "Waiting on blocker")}</small>
                                    {blockers.length > 0 ? (
                                      <small>
                                        Blocked by {blockers.map((blocker) => blocker.issue_id).join(", ")}
                                      </small>
                                    ) : null}
                                  </div>
                                  <span className={ready ? "status status--ready" : "status"}>
                                    {lifecycle}
                                  </span>
                                  {issue ? (
                                    <button
                                      type="button"
                                      className="issue-node__inspect"
                                      aria-label={`Inspect ${issue.issue_id}`}
                                      onClick={() => {
                                        setSelectedIssueId(issue.issue_id);
                                        setSelectedSessionId(null);
                                      }}
                                    >
                                      Inspect
                                    </button>
                                  ) : null}
                                </article>
                              );
                            })}
                          </div>
                          {selectedIssue ? (
                            <IssueSliceInspector
                              issue={selectedIssue}
                              selectedSessionId={selectedSessionId}
                              onSelectSession={setSelectedSessionId}
                            />
                          ) : null}
                        </div>
                      )}
                    </>
                  )}
                </section>
            </section>
            {commandAuditOpen ? (
              <section className="command-audit" aria-label="Command Audit">
                <div className="command-audit__heading">
                  <div>
                    <span className="eyebrow">Audit drill-down</span>
                    <h3>Command Audit</h3>
                  </div>
                </div>
                <ShellTerminalPanel terminal={shellTerminal} />
              </section>
            ) : null}
          </div>
        </aside>
      </div>
    </div>
  );
}

function CommandConsoleCard({
  turn,
  actionPending,
  denialReason,
  onDenialReasonChange,
  onDecide,
}: {
  readonly turn: CommandConsoleTurn;
  readonly actionPending: boolean;
  readonly denialReason: string;
  readonly onDenialReasonChange: (reason: string) => void;
  readonly onDecide: (decision: "approve" | "deny") => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const fullOutput = [turn.stdout, turn.stderr].filter(Boolean).join("\n");
  const showApprovalPrompt =
    turn.status === "pending-approval" &&
    (turn.classification === "human-required" || turn.classification === "frontier-approvable");
  return (
    <article className="command-console-card" data-outcome={turn.status}>
      <header>
        <div>
          <span className="eyebrow">Shell Terminal command</span>
          <code>{turn.command}</code>
        </div>
        <strong>{turn.status}</strong>
      </header>
      <dl>
        <div>
          <dt>Purpose</dt>
          <dd>{turn.purpose}</dd>
        </div>
        <div>
          <dt>Working directory</dt>
          <dd>{turn.workingDirectory}</dd>
        </div>
        <div>
          <dt>Policy</dt>
          <dd>{turn.classification} / {turn.approvalState}</dd>
        </div>
        <div>
          <dt>Access</dt>
          <dd>
            {turn.accessLevel}
            {turn.requestedPaths.length ? ` / ${turn.requestedPaths.join(", ")}` : ""}
          </dd>
        </div>
        <div>
          <dt>Outcome</dt>
          <dd>{turn.exitCode === null ? "Exit not available" : `Exit ${turn.exitCode}`}</dd>
        </div>
      </dl>
      <p>{turn.summary}</p>
      {showApprovalPrompt ? (
        <div
          className="command-approval-prompt"
          role="group"
          aria-label={`Approval prompt for ${turn.commandId}`}
        >
          <div>
            <strong>Command approval required</strong>
            <span>Access / {turn.accessLevel}</span>
          </div>
          <code>{turn.command}</code>
          {turn.classification === "human-required" ? (
            <div className="command-approval-prompt__actions">
              <button
                type="button"
                aria-label={`Approve ${turn.commandId} inline`}
                disabled={actionPending}
                onClick={() => onDecide("approve")}
              >
                Approve
              </button>
              <label>
                <span>Denial reason</span>
                <input
                  aria-label={`Inline denial reason ${turn.commandId}`}
                  value={denialReason}
                  onChange={(event) => onDenialReasonChange(event.target.value)}
                />
              </label>
              <button
                type="button"
                aria-label={`Deny ${turn.commandId} inline`}
                className="action--danger"
                disabled={!denialReason.trim() || actionPending}
                onClick={() => onDecide("deny")}
              >
                Deny
              </button>
            </div>
          ) : (
            <span>Frontier Model approval required before execution.</span>
          )}
        </div>
      ) : null}
      <button
        type="button"
        aria-expanded={expanded}
        aria-label={`Inspect full output for ${turn.commandId}`}
        disabled={!fullOutput}
        onClick={() => setExpanded((current) => !current)}
      >
        {expanded ? "Hide full output" : "Inspect full output"}
      </button>
      {expanded ? (
        <pre aria-label={`Full command output for ${turn.commandId}`}>{fullOutput}</pre>
      ) : null}
      <small>{turn.requester} / {turn.commandId}</small>
    </article>
  );
}

function PathGrantConsolePrompt({
  request,
  actionPending,
  onGrant,
  onDeny,
}: {
  readonly request: ContextualPathGrantRequest;
  readonly actionPending: boolean;
  readonly onGrant: () => void;
  readonly onDeny: () => void;
}) {
  const resolved = request.status !== "pending";
  return (
    <article
      className="path-grant-prompt"
      data-outcome={request.status}
      role="group"
      aria-label={`Additional Path Grant request for ${request.path}`}
    >
      <header>
        <div>
          <span className="eyebrow">Additional Path Grant request</span>
          <strong>{request.status === "pending" ? "Authority required" : request.status}</strong>
        </div>
      </header>
      <p>{request.reason}</p>
      <div className="path-grant-prompt__details">
        <span>Path / {request.path}</span>
        <span>Access / {request.accessLevel}</span>
        <span>Duration / {request.durationSeconds} seconds</span>
        <span>Affected action / {request.affectedAction}</span>
      </div>
      {!resolved ? (
        <div className="path-grant-prompt__actions">
          <button
            type="button"
            aria-label={`Grant ${request.accessLevel} access for ${request.path}`}
            disabled={actionPending}
            onClick={onGrant}
          >
            Grant access
          </button>
          <button
            type="button"
            className="action--danger"
            aria-label={`Deny grant request for ${request.path}`}
            disabled={actionPending}
            onClick={onDeny}
          >
            Deny
          </button>
        </div>
      ) : (
        <small>
          {request.status === "granted"
            ? "Mission Commander granted this bounded authority."
            : "Mission Commander denied this grant request."}
        </small>
      )}
    </article>
  );
}

function workstationContinuityStorageKey(snapshot: WorkspaceSnapshot): string {
  const workspaceIdentity = encodeURIComponent(
    `${snapshot.workspace_session.id}:${snapshot.workspace_session.workspace_path}`,
  );
  return `alfredo:workstation-continuity:v1:${workspaceIdentity}`;
}

function readWorkstationContinuity(key: string): WorkstationContinuityState | null {
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return null;
    const value = JSON.parse(raw) as Partial<WorkstationContinuityState>;
    if (value.schema_version !== WORKSTATION_CONTINUITY_SCHEMA_VERSION) return null;
    const legacyContinuity = value as Partial<WorkstationContinuityState> & {
      readonly leftLaneMode?: "agent" | "terminal";
    };
    return {
      schema_version: WORKSTATION_CONTINUITY_SCHEMA_VERSION,
      commandAuditOpen:
        typeof value.commandAuditOpen === "boolean"
          ? value.commandAuditOpen
          : legacyContinuity.leftLaneMode === "terminal",
      selectedIssueId: stringOrNull(value.selectedIssueId),
      selectedSessionId: stringOrNull(value.selectedSessionId),
      expandedWorkstationCardIds: stringArray(value.expandedWorkstationCardIds),
      pinnedWorkstationCardIds: stringArray(value.pinnedWorkstationCardIds),
      workstationFilter: typeof value.workstationFilter === "string" ? value.workstationFilter : "",
      workstationSort: workstationSortValue(value.workstationSort),
      selectedWorkstationSessionId: stringOrNull(value.selectedWorkstationSessionId),
      selectedWorkstationDiff: workstationDiffOrNull(value.selectedWorkstationDiff),
    };
  } catch {
    return null;
  }
}

function writeWorkstationContinuity(
  key: string,
  value: WorkstationContinuityState,
): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Best-effort local UI continuity must not block authoritative workspace rendering.
  }
}

function stringOrNull(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function stringArray(value: unknown): readonly string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string" && item.length > 0)
    : [];
}

function workstationSortValue(value: unknown): "priority" | "name" | "status" {
  return value === "name" || value === "status" ? value : "priority";
}

function workstationDiffOrNull(value: unknown): WorkstationDiffLink | null {
  if (!value || typeof value !== "object") return null;
  const diff = value as Partial<WorkstationDiffLink>;
  return typeof diff.label === "string" &&
    typeof diff.path === "string" &&
    typeof diff.href === "string" &&
    typeof diff.sessionId === "string"
    ? {
        label: diff.label,
        path: diff.path,
        href: diff.href,
        sessionId: diff.sessionId,
      }
    : null;
}

function filterAndSortWorkstationGroups(
  groups: readonly WorkstationCardGroup[],
  filter: string,
  sort: "priority" | "name" | "status",
  pinnedIds: readonly string[],
): readonly WorkstationCardGroup[] {
  const normalizedFilter = filter.trim().toLowerCase();
  return groups
    .map((group) => {
      const cards = group.cards.filter((card) => {
        if (!normalizedFilter) return true;
        return workstationCardSearchText(card).includes(normalizedFilter);
      });
      return {
        ...group,
        cards: [...cards].sort((left, right) => {
          const pinPriority =
            Number(pinnedIds.includes(right.id)) - Number(pinnedIds.includes(left.id));
          if (pinPriority !== 0) return pinPriority;
          if (sort === "name") return left.name.localeCompare(right.name);
          if (sort === "status") return left.status.localeCompare(right.status);
          return 0;
        }),
      };
    })
    .filter((group) => group.cards.length > 0);
}

function workstationCardSearchText(card: WorkstationCardProjection): string {
  return [
    card.name,
    card.sessionId ?? "",
    card.issueId ?? "",
    card.model,
    card.role,
    card.currentTask,
    card.status,
    card.phase,
    card.progress,
    card.latestCommandOrTest,
    ...card.detail.filesTouched.map((file) => file.path),
    ...card.detail.evidenceLinks.map((link) => link.href),
    ...card.detail.terminalExcerpts.map((excerpt) => excerpt.excerpt),
  ]
    .join(" ")
    .toLowerCase();
}

function WorkstationCard({
  card,
  expanded,
  pinned,
  selectedSessionId,
  onToggleExpanded,
  onTogglePinned,
  onSelectSession,
  onOpenDiff,
  queueReasons,
  onQueueReasonChange,
  onQueueDecision,
  workstationActionDrafts,
  onWorkstationActionDraftChange,
  onWorkstationAction,
  actionState,
}: {
  card: WorkstationCardProjection;
  expanded: boolean;
  pinned: boolean;
  selectedSessionId: string | null;
  onToggleExpanded: () => void;
  onTogglePinned: () => void;
  onSelectSession: (sessionId: string) => void;
  onOpenDiff: (diff: WorkstationDiffLink) => void;
  queueReasons: Record<string, string>;
  onQueueReasonChange: (itemId: string, reason: string) => void;
  onQueueDecision: (itemId: string, decision: WorkspaceQueueDecision, reason: string) => void;
  workstationActionDrafts: Record<string, WorkstationActionDraftState>;
  onWorkstationActionDraftChange: (key: string, draft: WorkstationActionDraftState) => void;
  onWorkstationAction: (
    action: WorkstationGovernedAction,
    draft: WorkstationActionDraftState,
  ) => void;
  actionState: WorkstationActionState | null;
}) {
  const queueActions = card.detail.governedActions.filter(
    (action) =>
      action.actionType === "workspace-queue-decision" && action.itemId && action.decision,
  );
  const workstationActions = card.detail.governedActions.filter(isExecutableWorkstationAction);
  const queueItemId = queueActions[0]?.itemId ?? null;
  const queueReason = queueItemId ? queueReasons[queueItemId] ?? "" : "";
  const workstationActionTargetIds = workstationActions.map(workstationActionTargetId).filter(Boolean);
  const matchingActionState =
    actionState &&
    ((queueItemId && actionState.itemId === queueItemId) ||
      workstationActionTargetIds.includes(actionState.itemId))
      ? actionState
      : queueActions.length === 0 && workstationActions.length === 0 && card.status === "waiting-approval"
        ? {
            itemId: card.id,
            state: "disabled" as const,
            message: "Approval actions are unavailable until the Orchestrator exposes a pending queue item.",
          }
        : null;
  const queueActionPending = matchingActionState?.state === "pending";
  const cardDomId = workstationDomId(card.id);
  const cardTitleId = `${cardDomId}-title`;
  const cardSummaryId = `${cardDomId}-summary`;
  const cardStatusDescriptionId = `${cardDomId}-status-description`;
  const cardDetailId = `${cardDomId}-detail`;
  return (
    <article
      className="workstation-card"
      data-attention={card.attention}
      data-tone={card.tone}
      data-pinned={pinned}
      tabIndex={0}
      aria-labelledby={cardTitleId}
      aria-describedby={cardSummaryId}
    >
      <p id={cardSummaryId} className="sr-only">
        {workstationCardSummary(card)}
      </p>
      <header>
        <div>
          <span className="eyebrow">{card.missionTitle}</span>
          <h3 id={cardTitleId}>{card.name}<span className="sr-only"> workstation card</span></h3>
          <small>{card.sessionId ?? card.issueId ?? card.missionId}</small>
        </div>
        <span
          className={card.attention ? "status status--ready" : "status"}
          aria-describedby={cardStatusDescriptionId}
        >
          {card.status}
        </span>
        <span id={cardStatusDescriptionId} className="sr-only">
          {workstationStatusDescription(card.status)}
        </span>
      </header>
      <p>{card.currentTask}</p>
      <dl>
        <div>
          <dt>Model</dt>
          <dd>{card.model}</dd>
        </div>
        <div>
          <dt>Role</dt>
          <dd>{card.role}</dd>
        </div>
        <div>
          <dt>Issue Slice</dt>
          <dd>{card.issueId ?? "None"}</dd>
        </div>
        <div>
          <dt>Phase</dt>
          <dd>{card.phase}</dd>
        </div>
        <div>
          <dt>Last activity</dt>
          <dd>{card.lastActivity}</dd>
        </div>
        <div>
          <dt>Files</dt>
          <dd>{card.filesTouched}</dd>
        </div>
        <div>
          <dt>Accepted</dt>
          <dd>r{card.acceptedRevision}</dd>
        </div>
      </dl>
      <small>{card.progress}</small>
      <small>{card.latestCommandOrTest}</small>
      {card.approvalBlockers.length ? (
        <ul className="workstation-card__blockers" aria-label={`${card.name} approval blockers`}>
          {card.approvalBlockers.map((blocker) => (
            <li key={blocker}>Approval blocker: {blocker}</li>
          ))}
        </ul>
      ) : null}
      <strong>{card.nextAction}</strong>
      <div className="workstation-card__actions">
        <button
          type="button"
          aria-label={`${expanded ? "Collapse" : "Expand"} ${card.name}`}
          aria-expanded={expanded}
          aria-controls={cardDetailId}
          onClick={onToggleExpanded}
        >
          {expanded ? "Collapse" : "Expand"}
        </button>
        <button
          type="button"
          aria-label={`${pinned ? "Unpin" : "Pin"} ${card.name}`}
          aria-pressed={pinned}
          onClick={onTogglePinned}
        >
          {pinned ? "Unpin" : "Pin"}
        </button>
      </div>
      {matchingActionState ? (
        <span
          role={
            matchingActionState.state === "rejected" || matchingActionState.state === "failed"
              ? "alert"
              : "status"
          }
          aria-label={`${card.name} workstation action state`}
          className="connection-pill"
        >
          {matchingActionState.state}: {matchingActionState.message}
        </span>
      ) : null}
      {queueActions.length > 0 && queueItemId ? (
        <div className="workstation-card__decision-actions">
          <label className="composer">
            <span>Decision reason</span>
            <textarea
              aria-label={`Workstation action reason ${queueItemId}`}
              rows={2}
              value={queueReason}
              onChange={(event) => onQueueReasonChange(queueItemId, event.target.value)}
            />
          </label>
          <div className="context-inspector__actions">
            {queueActions.map((action) => {
              const disabled =
                queueActionPending ||
                Boolean(action.requiresReason && !queueReason.trim()) ||
                !action.itemId ||
                !action.decision;
              const disabledDescription =
                disabled && action.itemId
                  ? workstationQueueActionDisabledDescription(action, queueReason, queueActionPending)
                  : null;
              const helpId = disabledDescription
                ? `${workstationDomId(`${action.itemId}:${action.decision ?? action.label}`)}-help`
                : undefined;
              return (
                <span className="workstation-card__action-wrap" key={`${action.itemId}:${action.decision}`}>
                  {disabledDescription ? (
                    <small id={helpId} className="workstation-card__action-help">
                      {disabledDescription}
                    </small>
                  ) : null}
                  <button
                    type="button"
                    aria-label={`${action.label} ${action.itemId}`}
                    aria-describedby={helpId}
                    disabled={disabled}
                    className={action.decision === "reject" ? "action--danger" : undefined}
                    onClick={() => {
                      if (!action.itemId || !action.decision) return;
                      onQueueDecision(action.itemId, action.decision, queueReason);
                    }}
                  >
                    {action.label}
                  </button>
                </span>
              );
            })}
          </div>
        </div>
      ) : null}
      {workstationActions.length > 0 ? (
        <div className="workstation-card__decision-actions">
          {workstationActions.map((action) => {
            const targetId = workstationActionTargetId(action);
            const key = workstationActionKey(action);
            const draft = workstationActionDrafts[key] ?? { reason: "", agentId: "" };
            const pending = matchingActionState?.state === "pending";
            const needsAgent = action.actionType === "model-assignment-change";
            const disabledDescription = workstationActionDisabledDescription(
              action,
              targetId,
              draft,
              pending,
            );
            const helpId = disabledDescription ? `${workstationDomId(key)}-help` : undefined;
            const disabled =
              pending ||
              Boolean(action.disabledReason) ||
              !targetId ||
              (action.requiresReason && !draft.reason.trim()) ||
              (needsAgent && !draft.agentId.trim());
            return (
              <div className="workstation-card__direct-action" key={key}>
                {needsAgent ? (
                  <label>
                    <span>Agent</span>
                    <input
                      aria-label={`Workstation action agent ${targetId}`}
                      value={draft.agentId}
                      onChange={(event) =>
                        onWorkstationActionDraftChange(key, {
                          ...draft,
                          agentId: event.target.value,
                        })
                      }
                    />
                  </label>
                ) : null}
                {action.requiresReason ? (
                  <label className="composer">
                    <span>Reason</span>
                    <textarea
                      aria-label={`Workstation action reason ${targetId}`}
                      rows={2}
                      value={draft.reason}
                      onChange={(event) =>
                        onWorkstationActionDraftChange(key, {
                          ...draft,
                          reason: event.target.value,
                        })
                      }
                    />
                  </label>
                ) : null}
                <button
                  type="button"
                  aria-label={`${action.label} ${targetId}`}
                  aria-describedby={helpId}
                  disabled={disabled}
                  className={action.actionType === "session-cancel" ? "action--danger" : undefined}
                  onClick={() => onWorkstationAction(action, draft)}
                >
                  {action.label}
                </button>
                {disabledDescription ? (
                  <small id={helpId} className="workstation-card__action-help">
                    {disabledDescription}
                  </small>
                ) : null}
              </div>
            );
          })}
        </div>
      ) : null}

      {expanded ? (
        <section
          id={cardDetailId}
          className="workstation-card-detail"
          aria-label={`${card.name} operational detail`}
        >
          <div className="workstation-card-detail__section">
            <h4>Tool Activity</h4>
            {card.detail.toolActivity.length === 0 ? (
              <p>No summarized tool activity recorded.</p>
            ) : (
              <ul>
                {card.detail.toolActivity.map((activity) => (
                  <li key={`${activity.label}:${activity.summary}`}>
                    <strong>{activity.label}</strong>
                    <span>{activity.summary}</span>
                  </li>
                ))}
              </ul>
            )}
          </div>

          <div className="workstation-card-detail__section">
            <h4>Files and Diffs</h4>
            {card.detail.filesTouched.length === 0 ? (
              <p>No touched files recorded.</p>
            ) : (
              <ul>
                {card.detail.filesTouched.map((file) => (
                  <li key={file.path}>
                    <span>{file.path}</span>
                    <small>{file.status}</small>
                  </li>
                ))}
              </ul>
            )}
            {card.detail.diffs.length > 0 ? (
              <div className="workstation-card-detail__actions">
                {card.detail.diffs.map((diff) => (
                  <button
                    key={diff.href}
                    type="button"
                    aria-label={`Open diff ${diff.path}`}
                    onClick={() => onOpenDiff(diff)}
                  >
                    {diff.label}
                  </button>
                ))}
              </div>
            ) : null}
          </div>

          <div className="workstation-card-detail__section">
            <h4>Evidence Packages</h4>
            {card.detail.evidenceLinks.length === 0 ? (
              <p>No Evidence Package link recorded.</p>
            ) : (
              <ul>
                {card.detail.evidenceLinks.map((link) => (
                  <li key={link.href}>
                    <a href={link.href}>{link.label}</a>
                    <small>{link.sessionId}</small>
                  </li>
                ))}
              </ul>
            )}
          </div>

          <div className="workstation-card-detail__section">
            <h4>Terminal Excerpts</h4>
            {card.detail.terminalExcerpts.length === 0 ? (
              <p>No terminal excerpts summarized.</p>
            ) : (
              <ul>
                {card.detail.terminalExcerpts.map((excerpt) => (
                  <li key={`${excerpt.label}:${excerpt.excerpt}`}>
                    <strong>{excerpt.label}</strong>
                    <span>{excerpt.excerpt}</span>
                    <small>{excerpt.sessionId}</small>
                  </li>
                ))}
              </ul>
            )}
          </div>

          <div className="workstation-card-detail__section">
            <h4>Review State</h4>
            <dl>
              <div>
                <dt>Evidence</dt>
                <dd>{card.detail.reviewState.evidenceState}</dd>
              </div>
              <div>
                <dt>Lifecycle</dt>
                <dd>{card.detail.reviewState.lifecycle}</dd>
              </div>
              <div>
                <dt>Review ready</dt>
                <dd>{card.detail.reviewState.reviewReady ? "yes" : "no"}</dd>
              </div>
              <div>
                <dt>Risks</dt>
                <dd>{card.detail.reviewState.risks}</dd>
              </div>
            </dl>
          </div>

          <div className="workstation-card-detail__section">
            <h4>Governed Actions</h4>
            <ul>
              {card.detail.governedActions.map((action) => (
                <li key={action.label}>
                  <strong>{action.label}</strong>
                  <span>{governedActionSurface(action.target)}</span>
                  {action.requiresReason ? <small>Reason required</small> : null}
                  {action.disabledReason ? <small>{action.disabledReason}</small> : null}
                  {action.recoveryPath ? <small>{action.recoveryPath}</small> : null}
                </li>
              ))}
            </ul>
          </div>

          {card.detail.originatingSessionId ? (
            <div className="workstation-card-detail__section">
              <h4>Originating Session</h4>
              <button
                type="button"
                aria-label={`Select session ${card.detail.originatingSessionId}`}
                onClick={() => onSelectSession(card.detail.originatingSessionId!)}
              >
                {card.detail.originatingSessionId}
              </button>
              {selectedSessionId === card.detail.originatingSessionId ? (
                <span className="workstation-local-selection">
                  Selected session {card.detail.originatingSessionId}
                </span>
              ) : null}
            </div>
          ) : null}
        </section>
      ) : null}
    </article>
  );
}

function workstationDomId(value: string): string {
  return `workstation-${value.replace(/[^a-zA-Z0-9_-]+/g, "-")}`;
}

function workstationCardSummary(card: WorkstationCardProjection): string {
  const blockers = card.approvalBlockers.length
    ? ` Approval blockers: ${card.approvalBlockers.map(workstationSentence).join(" ")}`
    : "";
  return `${workstationSentence(card.status)} ${workstationSentence(card.currentTask)} Next action: ${workstationSentence(card.nextAction)}${blockers} Last activity: ${workstationSentence(card.lastActivity)}`;
}

const WORKSTATION_STATUS_DESCRIPTIONS: Record<WorkstationCardProjection["status"], string> = {
  "waiting-approval": "Waiting for approval. Resolve the visible approval blocker before work can continue.",
  blocked: "Blocked work needs a recovery decision before progress can continue.",
  failed: "Failed work needs review, repair, retry, or human escalation.",
  reviewing: "Review is in progress. Monitor the review workspace for the next decision.",
  "review-ready": "Evidence is ready for review. Open Review Workspace for acceptance or repair.",
  done: "Work is complete. Review accepted evidence or activity history if needed.",
  running: "Running work is active. Monitor progress and preserve the prompt workflow.",
  idle: "Idle work is assigned or queued but not actively executing.",
  thinking: "The agent is preparing or thinking. Monitor progress without taking action yet.",
};

function workstationStatusDescription(status: WorkstationCardProjection["status"]): string {
  return WORKSTATION_STATUS_DESCRIPTIONS[status];
}

function workstationQueueActionDisabledDescription(
  action: WorkstationGovernedAction,
  reason: string,
  pending: boolean,
): string | null {
  const pendingDescription = workstationPendingActionDescription(action.label, pending);
  if (pendingDescription) return pendingDescription;
  if (action.requiresReason && !reason.trim() && action.itemId) {
    return workstationRequiredInputDescription(action.label, action.itemId, "a decision reason");
  }
  if (!action.itemId || !action.decision) return `${action.label} is unavailable from the current queue state.`;
  return null;
}

function workstationActionDisabledDescription(
  action: WorkstationGovernedAction,
  targetId: string,
  draft: WorkstationActionDraftState,
  pending: boolean,
): string | null {
  const pendingDescription = workstationPendingActionDescription(action.label, pending);
  if (pendingDescription) return pendingDescription;
  if (action.disabledReason) {
    return action.recoveryPath
      ? `${action.disabledReason} ${action.recoveryPath}`
      : action.disabledReason;
  }
  if (!targetId) return `${action.label} is unavailable from the current workstation state.`;
  if (action.requiresReason && !draft.reason.trim()) {
    return workstationRequiredInputDescription(action.label, targetId, "a reason");
  }
  if (action.actionType === "model-assignment-change" && !draft.agentId.trim()) {
    return workstationRequiredInputDescription(action.label, targetId, "an agent id");
  }
  return null;
}

function workstationPendingActionDescription(label: string, pending: boolean): string | null {
  return pending ? `${label} is disabled while the Orchestrator validates the current action.` : null;
}

function workstationRequiredInputDescription(label: string, targetId: string, inputPhrase: string): string {
  return `Enter ${inputPhrase} to enable ${label} for ${targetId}.`;
}

function workstationSentence(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  return /[.!?]$/.test(trimmed) ? trimmed : `${trimmed}.`;
}

function isExecutableWorkstationAction(
  action: WorkstationGovernedAction,
): action is WorkstationGovernedAction & {
  actionType:
    | "issue-launch"
    | "issue-retry"
    | "session-cancel"
    | "model-assignment-change";
} {
  return (
    action.actionType === "issue-launch" ||
    action.actionType === "issue-retry" ||
    action.actionType === "session-cancel" ||
    action.actionType === "model-assignment-change"
  );
}

function workstationActionTargetId(action: WorkstationGovernedAction): string {
  return action.targetIdentity?.id ?? action.sessionId ?? action.issueId ?? action.label;
}

function workstationActionKey(action: WorkstationGovernedAction): string {
  return `${action.actionType ?? "workstation-action"}:${workstationActionTargetId(action)}`;
}

function workstationActionRequestTarget(
  action: WorkstationGovernedAction,
): WorkstationActionRequest["target"] | null {
  if (action.targetIdentity?.kind === "issue-slice") {
    return { kind: "issue-slice", id: action.targetIdentity.id };
  }
  if (action.targetIdentity?.kind === "agent-session") {
    return { kind: "agent-session", id: action.targetIdentity.id };
  }
  return null;
}

function governedActionSurface(
  actionTarget: WorkstationCardProjection["detail"]["governedActions"][number]["target"],
): string {
  if (actionTarget === "workspace-queue") return "Use Workspace Queue governed controls";
  if (actionTarget === "review-workspace") return "Use Review Workspace governed controls";
  if (actionTarget === "activity") return "Use Activity Journal review history";
  return "Local monitoring only";
}

function ActivityJournal({
  projection,
  filters,
  status,
  onFilterChange,
  onRefresh,
}: {
  projection: ActivityJournalProjection | null;
  filters: ActivityJournalFilters;
  status: "pending" | "rejected" | null;
  onFilterChange: (filters: ActivityJournalFilters) => void;
  onRefresh: () => void;
}) {
  const updateFilter = (key: keyof ActivityJournalFilters, value: string) => {
    onFilterChange({ ...filters, [key]: value });
  };
  const entries = projection?.entries ?? [];
  return (
    <section className="activity-journal" aria-label="Activity Journal">
      <div className="activity-journal__heading">
        <div>
          <span className="eyebrow">Append-only record</span>
          <h1>Activity</h1>
        </div>
        <div className="mission-count">
          <strong>{entries.length}</strong>
          <span>{entries.length === 1 ? "Entry" : "Entries"}</span>
        </div>
      </div>

      <div className="activity-filters">
        <label>
          <span>Search Activity</span>
          <input
            type="search"
            aria-label="Search Activity"
            value={filters.search ?? ""}
            onChange={(event) => updateFilter("search", event.target.value)}
          />
        </label>
        <label>
          <span>Activity Mission</span>
          <input
            aria-label="Activity Mission"
            value={filters.mission_id ?? ""}
            onChange={(event) => updateFilter("mission_id", event.target.value)}
          />
        </label>
        <label>
          <span>Activity actor</span>
          <select
            aria-label="Activity actor"
            value={filters.actor ?? ""}
            onChange={(event) => updateFilter("actor", event.target.value)}
          >
            <option value="">All actors</option>
            <option value="mission-commander">Mission Commander</option>
            <option value="orchestrator">Orchestrator</option>
            <option value="frontier-model">Frontier Model</option>
            <option value="local-agent">Local Agent</option>
          </select>
        </label>
        <label>
          <span>Activity action type</span>
          <input
            aria-label="Activity action type"
            value={filters.action_type ?? ""}
            onChange={(event) => updateFilter("action_type", event.target.value)}
          />
        </label>
        <label>
          <span>Started at</span>
          <input
            aria-label="Activity started at"
            placeholder="2026-06-26T10:00:00Z"
            value={filters.started_at ?? ""}
            onChange={(event) => updateFilter("started_at", event.target.value)}
          />
        </label>
        <label>
          <span>Ended at</span>
          <input
            aria-label="Activity ended at"
            placeholder="2026-06-26T11:00:00Z"
            value={filters.ended_at ?? ""}
            onChange={(event) => updateFilter("ended_at", event.target.value)}
          />
        </label>
        <button
          type="button"
          onClick={onRefresh}
          disabled={status === "pending"}
        >
          Apply Activity filters
        </button>
      </div>

      {status ? (
        <span role="status" aria-label="Activity Journal status" className="connection-pill">
          {status[0].toUpperCase() + status.slice(1)}
        </span>
      ) : null}

      {entries.length === 0 ? (
        <div className="empty-state">
          <h2>No Activity Journal entries</h2>
        </div>
      ) : (
        <ol className="activity-list">
          {entries.map((entry) => (
            <li key={entry.entry_id}>
              <article className="activity-entry">
                <header>
                  <time dateTime={entry.recorded_at}>{entry.recorded_at}</time>
                  <code>{entry.actor} / {entry.action_type}</code>
                </header>
                <h2>{entry.summary}</h2>
                <small>{entry.entry_id} / {entry.correlation_id}</small>
                <div className="activity-entry__links" aria-label={`${entry.entry_id} affected entities`}>
                  {entry.affected_entities.map((entity) => (
                    <div key={`${entry.entry_id}:${entity.entity_type}:${entity.entity_id}`}>
                      <span>{entity.entity_type} / {entity.entity_id}</span>
                      {entity.href ? (
                        <a href={entity.href}>{entity.label}</a>
                      ) : (
                        <strong>{entity.label}</strong>
                      )}
                    </div>
                  ))}
                </div>
                {entry.evidence_links.length > 0 ? (
                  <div className="activity-entry__evidence">
                    {entry.evidence_links.map((link) => (
                      <a key={link} href={link}>{link}</a>
                    ))}
                  </div>
                ) : null}
              </article>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function WorkspaceQueue({
  projection,
  missionDrafts,
  missionDraftStatus,
  missionDraftReasons,
  status,
  latestConsoleMessage,
  reasons,
  onReasonChange,
  onDecision,
  onAdHocProposal,
  onMissionDraftCreate,
  onMissionDraftReasonChange,
  onMissionDraftDecision,
}: {
  projection: WorkspaceQueueProjection | null;
  missionDrafts: MissionDraftProjection | null;
  missionDraftStatus: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  missionDraftReasons: Record<string, string>;
  status: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  latestConsoleMessage: AgentConsoleMessage | null;
  reasons: Record<string, string>;
  onReasonChange: (itemId: string, reason: string) => void;
  onDecision: (itemId: string, decision: WorkspaceQueueDecision, reason: string) => void;
  onAdHocProposal: (proposal: AdHocDelegationDraft) => void;
  onMissionDraftCreate: (draft: MissionDraftCreateDraft) => void;
  onMissionDraftReasonChange: (draftId: string, reason: string) => void;
  onMissionDraftDecision: (
    draftId: string,
    decision: MissionDraftDecision,
    reason: string,
  ) => void;
}) {
  const [acceptanceCriteria, setAcceptanceCriteria] = useState("");
  const [allowedPaths, setAllowedPaths] = useState("");
  const [commandPolicy, setCommandPolicy] = useState("");
  const [proposedAgent, setProposedAgent] = useState("qwen-coder-local-1");
  const [draftGoal, setDraftGoal] = useState("");
  const [selectedAdHocIds, setSelectedAdHocIds] = useState<readonly string[]>([]);
  const [excludedAdHocIds, setExcludedAdHocIds] = useState<readonly string[]>([]);
  const [newWorkItems, setNewWorkItems] = useState("");
  const [dependencies, setDependencies] = useState("");
  const [unresolvedDecisions, setUnresolvedDecisions] = useState("");
  const decisionStatusRef = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (status) decisionStatusRef.current?.focus();
  }, [status]);
  if (!projection) {
    return (
      <div className="empty-state">
        <span className="eyebrow">Governance inbox</span>
        <h1>Workspace Queue</h1>
        <p>Loading queue items.</p>
      </div>
    );
  }

  const parsedCriteria = splitDraftList(acceptanceCriteria);
  const parsedPaths = splitDraftList(allowedPaths);
  const parsedPolicy = parseCommandPolicyDraft(commandPolicy);
  const pendingAdHocItems = projection.items.filter(isPendingAdHocDelegation);
  const parsedNewWorkItems = splitDraftList(newWorkItems);
  const parsedDependencies = splitDraftList(dependencies);
  const parsedUnresolvedDecisions = splitDraftList(unresolvedDecisions);
  const canPropose =
    Boolean(latestConsoleMessage) &&
    parsedCriteria.length > 0 &&
    parsedPaths.length > 0 &&
    proposedAgent.trim().length > 0 &&
    status?.state !== "pending";
  const canCreateMissionDraft =
    draftGoal.trim().length > 0 &&
    (selectedAdHocIds.length > 0 || parsedNewWorkItems.length > 0) &&
    missionDraftStatus?.state !== "pending";

  return (
    <section className="workspace-queue" aria-label="Workspace Queue">
      <div className="mission-heading">
        <div>
          <span className="eyebrow">Governance inbox</span>
          <h1>Workspace Queue</h1>
        </div>
        <div className="mission-count">
          <strong>{projection.items.length}</strong>
          <span>queue items</span>
        </div>
      </div>
      {status ? (
        <span
          ref={decisionStatusRef}
          role="status"
          aria-label="Workspace Queue decision status"
          className="connection-pill"
          tabIndex={-1}
        >
          {status.state[0].toUpperCase() + status.state.slice(1)}: {status.message}
        </span>
      ) : null}
      <MissionDrafts
        projection={missionDrafts}
        status={missionDraftStatus}
        reasons={missionDraftReasons}
        onReasonChange={onMissionDraftReasonChange}
        onDecision={onMissionDraftDecision}
      />
      <section className="queue-proposal" aria-label="Mission Draft creation">
        <div className="issue-inspector__heading">
          <div>
            <span className="eyebrow">Selected ad hoc work</span>
            <h2>Create Mission Draft</h2>
            <strong>{pendingAdHocItems.length} available delegations</strong>
          </div>
        </div>
        {pendingAdHocItems.length === 0 ? (
          <p>No pending Ad Hoc Delegations available.</p>
        ) : (
          <div className="draft-selection-list">
            {pendingAdHocItems.map((item) => (
              <article key={item.item_id} className="draft-selection-item">
                <div>
                  <strong>{item.issue_id}</strong>
                  <span>{item.requested_action}</span>
                </div>
                <label>
                  <input
                    type="checkbox"
                    aria-label={`Include ${item.issue_id}`}
                    checked={selectedAdHocIds.includes(item.issue_id)}
                    onChange={(event) => {
                      if (event.target.checked) {
                        setSelectedAdHocIds((current) => [...current, item.issue_id]);
                        setExcludedAdHocIds((current) =>
                          current.filter((workId) => workId !== item.issue_id),
                        );
                        return;
                      }
                      setSelectedAdHocIds((current) =>
                        current.filter((workId) => workId !== item.issue_id),
                      );
                    }}
                  />
                  <span>Include</span>
                </label>
                <label>
                  <input
                    type="checkbox"
                    aria-label={`Exclude ${item.issue_id}`}
                    checked={excludedAdHocIds.includes(item.issue_id)}
                    onChange={(event) => {
                      if (event.target.checked) {
                        setExcludedAdHocIds((current) => [...current, item.issue_id]);
                        setSelectedAdHocIds((current) =>
                          current.filter((workId) => workId !== item.issue_id),
                        );
                        return;
                      }
                      setExcludedAdHocIds((current) =>
                        current.filter((workId) => workId !== item.issue_id),
                      );
                    }}
                  />
                  <span>Exclude</span>
                </label>
              </article>
            ))}
          </div>
        )}
        <label>
          <span>Proposed goal</span>
          <input
            aria-label="Mission Draft proposed goal"
            value={draftGoal}
            onChange={(event) => setDraftGoal(event.target.value)}
          />
        </label>
        <label className="composer">
          <span>New work</span>
          <textarea
            aria-label="Mission Draft new work"
            rows={2}
            value={newWorkItems}
            onChange={(event) => setNewWorkItems(event.target.value)}
          />
        </label>
        <label>
          <span>Dependencies</span>
          <input
            aria-label="Mission Draft dependencies"
            value={dependencies}
            onChange={(event) => setDependencies(event.target.value)}
          />
        </label>
        <label>
          <span>Unresolved decisions</span>
          <input
            aria-label="Mission Draft unresolved decisions"
            value={unresolvedDecisions}
            onChange={(event) => setUnresolvedDecisions(event.target.value)}
          />
        </label>
        <button
          type="button"
          disabled={!canCreateMissionDraft}
          onClick={() =>
            onMissionDraftCreate({
              proposedGoal: draftGoal.trim(),
              selectedAdHocIds,
              excludedAdHocIds,
              newWorkItems: parsedNewWorkItems,
              dependencies: parsedDependencies,
              unresolvedDecisions: parsedUnresolvedDecisions,
            })
          }
        >
          Create Mission Draft
        </button>
      </section>
      <section className="queue-proposal" aria-label="Ad Hoc Delegation proposal">
        <div className="issue-inspector__heading">
          <div>
            <span className="eyebrow">Prompt origin</span>
            <h2>Ad Hoc Delegation</h2>
            <strong>{latestConsoleMessage?.message_id ?? "No prompt message"}</strong>
          </div>
        </div>
        <label className="composer">
          <span>Acceptance criteria</span>
          <textarea
            aria-label="Ad Hoc Delegation acceptance criteria"
            rows={2}
            value={acceptanceCriteria}
            onChange={(event) => setAcceptanceCriteria(event.target.value)}
          />
        </label>
        <label>
          <span>Allowed paths</span>
          <input
            aria-label="Ad Hoc Delegation allowed paths"
            value={allowedPaths}
            onChange={(event) => setAllowedPaths(event.target.value)}
          />
        </label>
        <label>
          <span>Command policy</span>
          <input
            aria-label="Ad Hoc Delegation command policy"
            value={commandPolicy}
            onChange={(event) => setCommandPolicy(event.target.value)}
          />
        </label>
        <label>
          <span>Proposed Local Agent</span>
          <input
            aria-label="Ad Hoc Delegation proposed agent"
            value={proposedAgent}
            onChange={(event) => setProposedAgent(event.target.value)}
          />
        </label>
        <button
          type="button"
          disabled={!canPropose}
          onClick={() => {
            if (!latestConsoleMessage || !canPropose) return;
            onAdHocProposal({
              acceptanceCriteria: parsedCriteria,
              allowedPaths: parsedPaths,
              commandPolicy: parsedPolicy,
              proposedAgent: proposedAgent.trim(),
              originatingMessageId: latestConsoleMessage.message_id,
            });
          }}
        >
          Propose Ad Hoc Delegation
        </button>
      </section>
      {projection.items.length === 0 ? (
        <div className="empty-state">
          <h2>No governance items pending</h2>
        </div>
      ) : (
        <div className="queue-groups">
          {projection.groups.map((group) => (
            <section key={group.group_id} className="queue-group">
              <div className="issue-inspector__heading">
                <div>
                  <span className="eyebrow">{group.item_count} items</span>
                  <h2>{group.item_type} / {group.mission_id}</h2>
                </div>
              </div>
              <div className="review-list">
                {group.items.map((item) => {
                  const reason = reasons[item.item_id] ?? "";
                  const pending = status?.state === "pending";
                  const isPendingItem = item.status === "pending";
                  return (
                    <article className="review-item queue-item" key={item.item_id} id={item.item_id}>
                      <div className="issue-inspector__heading">
                        <div>
                          <span className="eyebrow">{item.issue_id} / {item.status}</span>
                          <h3>{item.requested_action}</h3>
                          <strong>{item.item_id}</strong>
                        </div>
                        <span className={isPendingItem ? "status status--ready" : "status"}>
                          {item.status}
                        </span>
                      </div>

                      <dl className="issue-inspector__facts">
                        <div>
                          <dt>Source</dt>
                          <dd>{item.source}</dd>
                        </div>
                        <div>
                          <dt>Affected boundary</dt>
                          <dd>{item.affected_boundary}</dd>
                        </div>
                        <div>
                          <dt>Consequence</dt>
                          <dd>{item.consequence}</dd>
                        </div>
                      </dl>

                      <section>
                        <h4>Requested Changes</h4>
                        <ul>
                          {proposedChangeLines(item.proposed_changes).map((line) => (
                            <li key={line}>{line}</li>
                          ))}
                        </ul>
                      </section>

                      <label className="composer">
                        <span>Decision reason</span>
                        <textarea
                          aria-label={`Workspace Queue reason ${item.item_id}`}
                          rows={2}
                          value={reason}
                          onChange={(event) => onReasonChange(item.item_id, event.target.value)}
                        />
                      </label>

                      <div className="context-inspector__actions">
                        <button
                          type="button"
                          aria-label={`Approve ${item.item_id}`}
                          disabled={!isPendingItem || pending}
                          onClick={() => onDecision(item.item_id, "approve", reason)}
                        >
                          Approve
                        </button>
                        <button
                          type="button"
                          aria-label={`Reject ${item.item_id}`}
                          className="action--danger"
                          disabled={!isPendingItem || pending || !reason.trim()}
                          onClick={() => onDecision(item.item_id, "reject", reason)}
                        >
                          Reject
                        </button>
                        <button
                          type="button"
                          aria-label={`Defer ${item.item_id}`}
                          disabled={!isPendingItem || pending || !reason.trim()}
                          onClick={() => onDecision(item.item_id, "defer", reason)}
                        >
                          Defer
                        </button>
                      </div>
                    </article>
                  );
                })}
              </div>
            </section>
          ))}
        </div>
      )}
    </section>
  );
}

function MissionDrafts({
  projection,
  status,
  reasons,
  onReasonChange,
  onDecision,
}: {
  projection: MissionDraftProjection | null;
  status: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  reasons: Record<string, string>;
  onReasonChange: (draftId: string, reason: string) => void;
  onDecision: (draftId: string, decision: MissionDraftDecision, reason: string) => void;
}) {
  const decisionStatusRef = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (status) decisionStatusRef.current?.focus();
  }, [status]);
  if (!projection) {
    return (
      <section className="queue-group" aria-label="Mission Drafts">
        <div className="issue-inspector__heading">
          <div>
            <span className="eyebrow">Proposed mission state</span>
            <h2>Mission Drafts</h2>
          </div>
        </div>
        <p>Loading Mission Drafts.</p>
      </section>
    );
  }

  return (
    <section className="queue-group" aria-label="Mission Drafts">
      <div className="issue-inspector__heading">
        <div>
          <span className="eyebrow">Proposed mission state / revision {projection.revision}</span>
          <h2>Mission Drafts</h2>
        </div>
      </div>
      {status ? (
        <span
          ref={decisionStatusRef}
          role="status"
          aria-label="Mission Draft decision status"
          className="connection-pill"
          tabIndex={-1}
        >
          {status.state[0].toUpperCase() + status.state.slice(1)}: {status.message}
        </span>
      ) : null}
      {projection.drafts.length === 0 ? (
        <p>No Mission Drafts proposed.</p>
      ) : (
        <div className="review-list">
          {projection.drafts.map((draft) => (
            <article className="review-item queue-item" key={draft.draft_id}>
              <div className="review-item__heading">
                <div>
                  <span className="eyebrow">{draft.status} / {draft.mission_id}</span>
                  <h3>{draft.proposed_goal}</h3>
                  <code>{draft.draft_id}</code>
                </div>
              </div>
              <div className="queue-item__payload">
                <strong>Included work</strong>
                {draft.included_ad_hoc_work.length === 0 ? (
                  <p>No Ad Hoc Delegations selected.</p>
                ) : (
                  draft.included_ad_hoc_work.map((work) => (
                    <div key={work.work_id}>
                      <code>{work.work_id}</code>
                      <small>{work.source} / {work.status}</small>
                      {work.acceptance_criteria.map((criterion) => (
                        <p key={criterion}>{criterion}</p>
                      ))}
                    </div>
                  ))
                )}
                <strong>Exclusions</strong>
                {draft.excluded_ad_hoc_work_ids.length === 0 ? (
                  <p>No explicit exclusions.</p>
                ) : (
                  draft.excluded_ad_hoc_work_ids.map((workId) => (
                    <p key={workId}>Excluded: {workId}</p>
                  ))
                )}
                <strong>New work</strong>
                {draft.new_work_items.map((item) => <p key={item}>{item}</p>)}
                <strong>Dependencies</strong>
                {draft.dependencies.length === 0 ? (
                  <p>No dependencies listed.</p>
                ) : (
                  draft.dependencies.map((item) => <p key={item}>{item}</p>)
                )}
                <strong>Unresolved decisions</strong>
                {draft.unresolved_decisions.length === 0 ? (
                  <p>No unresolved decisions listed.</p>
                ) : (
                  draft.unresolved_decisions.map((item) => <p key={item}>{item}</p>)
                )}
              </div>
              {draft.status === "draft" ? (
                <div className="queue-item__actions">
                  <label>
                    <span>Mission Draft decision reason</span>
                    <textarea
                      aria-label="Mission Draft decision reason"
                      rows={2}
                      value={reasons[draft.draft_id] ?? ""}
                      onChange={(event) => onReasonChange(draft.draft_id, event.target.value)}
                    />
                  </label>
                  <button
                    type="button"
                    disabled={status?.state === "pending" || !(reasons[draft.draft_id] ?? "").trim()}
                    aria-label={`Confirm ${draft.draft_id}`}
                    onClick={() =>
                      onDecision(draft.draft_id, "confirm", (reasons[draft.draft_id] ?? "").trim())
                    }
                  >
                    Confirm
                  </button>
                  <button
                    type="button"
                    disabled={status?.state === "pending" || !(reasons[draft.draft_id] ?? "").trim()}
                    aria-label={`Abandon ${draft.draft_id}`}
                    className="action--danger"
                    onClick={() =>
                      onDecision(draft.draft_id, "abandon", (reasons[draft.draft_id] ?? "").trim())
                    }
                  >
                    Abandon
                  </button>
                </div>
              ) : null}
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

function splitDraftList(value: string): string[] {
  return value
    .split(/\n|,/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function isPendingAdHocDelegation(item: WorkspaceQueueItem): boolean {
  return item.item_type === "ad-hoc-delegation" && item.status === "pending";
}

function parseCommandPolicyDraft(value: string): Record<string, string> {
  return splitDraftList(value).reduce<Record<string, string>>((policy, entry) => {
    const [command, level] = entry.split(/=(.*)/s).filter((part) => part !== undefined);
    if (command?.trim() && level?.trim()) {
      policy[command.trim()] = level.trim();
    }
    return policy;
  }, {});
}

function proposedChangeLines(changes: Readonly<Record<string, unknown>>): string[] {
  return Object.entries(changes).flatMap(([field, value]) => {
    if (Array.isArray(value)) {
      return value.map((item) => String(item));
    }
    if (value && typeof value === "object") {
      return [`${field}: ${JSON.stringify(value)}`];
    }
    return [`${field}: ${String(value)}`];
  });
}

function ReviewWorkspace({
  projection,
  status,
  reasons,
  onReasonChange,
  onDecision,
}: {
  projection: ReviewWorkspaceProjection | null;
  status: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  reasons: Record<string, string>;
  onReasonChange: (sessionId: string, reason: string) => void;
  onDecision: (sessionId: string, decision: ReviewDecision, reason: string) => void;
}) {
  const decisionStatusRef = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (status) decisionStatusRef.current?.focus();
  }, [status]);
  if (!projection) {
    return (
      <div className="empty-state">
        <span className="eyebrow">Evidence decision surface</span>
        <h1>Review Workspace</h1>
        <p>Loading review evidence.</p>
      </div>
    );
  }

  return (
    <section className="review-workspace" aria-label="Review Workspace">
      <div className="mission-heading">
        <div>
          <span className="eyebrow">Evidence decision surface</span>
          <h1>Review Workspace</h1>
        </div>
        <div className="mission-count">
          <strong>{projection?.items.length ?? 0}</strong>
          <span>awaiting review</span>
        </div>
      </div>
      {status ? (
        <span
          ref={decisionStatusRef}
          role="status"
          aria-label="Review decision status"
          className="connection-pill"
          tabIndex={-1}
        >
          {status.state[0].toUpperCase() + status.state.slice(1)}: {status.message}
        </span>
      ) : null}
      {projection.items.length === 0 ? (
        <div className="empty-state">
          <h2>No evidence awaiting review</h2>
        </div>
      ) : (
        <div className="review-list">
          {projection.items.map((item) => {
            const reason = reasons[item.session_id] ?? "";
            const pending = status?.state === "pending";
            return (
              <article className="review-item" key={item.session_id}>
                <div className="issue-inspector__heading">
                  <div>
                    <span className="eyebrow">{item.issue_id} / {item.status}</span>
                    <h2>{item.issue_title}</h2>
                    <strong>{item.session_id}</strong>
                  </div>
                  <span className={item.evidence_complete ? "status status--ready" : "status"}>
                    {item.evidence_complete ? "Evidence complete" : "Evidence incomplete"}
                  </span>
                </div>

                {item.missing_evidence.length > 0 ? (
                  <p>{item.missing_evidence.join(", ")}</p>
                ) : null}

                <dl className="issue-inspector__facts">
                  <div>
                    <dt>Changed files</dt>
                    <dd>{item.evidence.changed_files.join(", ") || "None recorded"}</dd>
                  </div>
                  <div>
                    <dt>Diff summary</dt>
                    <dd>{item.evidence.diff_summary || "Missing"}</dd>
                  </div>
                  <div>
                    <dt>Test results</dt>
                    <dd>{item.evidence.test_results || "Missing"}</dd>
                  </div>
                  <div>
                    <dt>Risks</dt>
                    <dd>{item.evidence.risks || "Missing"}</dd>
                  </div>
                  <div>
                    <dt>Proposed context updates</dt>
                    <dd>{item.evidence.proposed_context_updates || "Missing"}</dd>
                  </div>
                </dl>

                <section>
                  <h3>Commands</h3>
                  {item.evidence.commands_run.length === 0 ? (
                    <p>Missing</p>
                  ) : (
                    <ul>
                      {item.evidence.commands_run.map((command) => (
                        <li key={command}>{command}</li>
                      ))}
                    </ul>
                  )}
                </section>

                <section>
                  <h3>Visibility Limitations</h3>
                  {item.visibility_limitations.length === 0 ? (
                    <p>None recorded</p>
                  ) : (
                    <ul>
                      {item.visibility_limitations.map((limitation) => (
                        <li key={`${item.session_id}:${limitation.path}`}>
                          <strong>{limitation.path}</strong>
                          <span>{limitation.classification}</span>
                          <small>{limitation.consequence}</small>
                        </li>
                      ))}
                    </ul>
                  )}
                </section>

                <label className="composer">
                  <span>Decision reason</span>
                  <textarea
                    aria-label={`Review reason ${item.session_id}`}
                    rows={2}
                    value={reason}
                    onChange={(event) => onReasonChange(item.session_id, event.target.value)}
                  />
                </label>

                <div className="context-inspector__actions">
                  <button
                    type="button"
                    aria-label={`Accept ${item.session_id}`}
                    disabled={!item.can_accept || pending}
                    onClick={() => onDecision(item.session_id, "accept", reason)}
                  >
                    Accept
                  </button>
                  <button
                    type="button"
                    aria-label={`Request repair ${item.session_id}`}
                    className="action--warning"
                    disabled={!reason.trim() || pending}
                    onClick={() => onDecision(item.session_id, "repair", reason)}
                  >
                    Repair
                  </button>
                  <button
                    type="button"
                    aria-label={`Escalate ${item.session_id}`}
                    className="action--danger"
                    disabled={pending}
                    onClick={() => onDecision(item.session_id, "escalate-human", reason)}
                  >
                    Escalate
                  </button>
                </div>
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}

function IssueSliceInspector({
  issue,
  selectedSessionId,
  onSelectSession,
}: {
  issue: WorkspaceIssueSliceSummary;
  selectedSessionId: string | null;
  onSelectSession: (sessionId: string) => void;
}) {
  const selectedSession =
    issue.sessions.find((session) => session.session_id === selectedSessionId) ?? null;
  return (
    <section
      id="issue-slice-inspector"
      className="issue-inspector"
      aria-label="Issue Slice Inspector"
      tabIndex={-1}
    >
      <div className="issue-inspector__heading">
        <div>
          <span className="eyebrow">Issue Slice</span>
          <h2>{issue.issue_id}</h2>
        </div>
        <span className={issue.launch_eligible ? "status status--ready" : "status"}>
          {issue.lifecycle}
        </span>
      </div>
      <p>{issue.title}</p>
      <dl className="issue-inspector__facts">
        <div>
          <dt>Progress</dt>
          <dd>{issue.progress}</dd>
        </div>
        <div>
          <dt>Provenance</dt>
          <dd>{`${issue.provenance.role} / ${issue.provenance.provider} / ${issue.provenance.model}`}</dd>
        </div>
        <div>
          <dt>Assignment</dt>
          <dd>{issue.model_assignment.agent_id}</dd>
          <dd>{issue.model_assignment.availability}</dd>
          {issue.model_assignment.availability_reason ? (
            <dd>{issue.model_assignment.availability_reason}</dd>
          ) : null}
        </div>
        <div>
          <dt>Provider Operation</dt>
          <dd>{issue.model_assignment.operation_status}</dd>
          {issue.model_assignment.failure ? <dd>{issue.model_assignment.failure}</dd> : null}
        </div>
      </dl>

      <section>
        <h3>Accepted Boundary</h3>
        <p>{issue.accepted_boundary.what_to_build}</p>
        <ul>
          {issue.accepted_boundary.acceptance_criteria.map((criterion) => (
            <li key={criterion}>{criterion}</li>
          ))}
        </ul>
      </section>

      <section>
        <h3>Blockers</h3>
        {issue.blockers.length === 0 ? (
          <p>No blockers</p>
        ) : (
          <ul>
            {issue.blockers.map((blocker) => (
              <li key={blocker.issue_id}>
                <span>{`Blocked by ${blocker.issue_id}`}</span>
                <small>{`${blocker.lifecycle} / ${blocker.satisfied ? "satisfied" : "open"}`}</small>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section>
        <h3>Evidence Package</h3>
        <p>{issue.evidence.test_results}</p>
        <small>{issue.evidence.state}</small>
        {issue.evidence.risks ? <small>{issue.evidence.risks}</small> : null}
      </section>

      <section>
        <h3>Working Context</h3>
        <ul>
          {issue.working_context_sources.map((source) => (
            <li key={source.source_id}>{source.label}</li>
          ))}
        </ul>
      </section>

      <section>
        <h3>Sessions</h3>
        {issue.sessions.length === 0 ? (
          <p>No attached sessions</p>
        ) : (
          <div className="issue-inspector__sessions">
            {issue.sessions.map((session) => (
              <button
                key={session.session_id}
                type="button"
                aria-label={`Session ${session.session_id}`}
                onClick={() => onSelectSession(session.session_id)}
              >
                {session.session_id}
              </button>
            ))}
          </div>
        )}
        {selectedSession ? (
          <article className="issue-session-detail">
            <strong>{selectedSession.session_id}</strong>
            <span>{selectedSession.role} / {selectedSession.provider} / {selectedSession.model}</span>
            <span>{selectedSession.status}</span>
            <span>{selectedSession.operation_status}</span>
            <span>{selectedSession.stale ? "stale" : "fresh"}</span>
            <span>{selectedSession.disconnected ? "disconnected" : "connected"}</span>
            {selectedSession.failure ? <span>{selectedSession.failure}</span> : null}
          </article>
        ) : null}
      </section>
    </section>
  );
}
