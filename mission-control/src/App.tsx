import { useCallback, useEffect, useState } from "react";
import type {
  AdHocDelegationProposalRequest,
  AgentConsoleMessage,
  ActivityJournalFilters,
  ActivityJournalProjection,
  ConversationScope,
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
} from "./contracts";
import type { WorkspaceClient } from "./workspace-client";
import { applyWorkspaceUpdates } from "./workspace-sync";
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

interface AppProps {
  readonly client: WorkspaceClient;
  readonly syncIntervalMs?: number;
}

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
    setActionStatus("pending");
    const result = await client.changeScope({
      correlation_id: `conversation-scope-${scopeDraft.kind}-${scopeDraft.target_id}-${current.snapshot.revision}`,
      expected_revision: current.snapshot.revision,
      scope_kind: scopeDraft.kind,
      scope_target: scopeDraft.target_id,
      scope_label: scopeDraft.label,
    });
    if (result.kind !== "acknowledged") {
      setActionStatus(result.kind);
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
    setScopeDraft(null);
    setActionStatus("acknowledged");
    await refreshWorkingContext();
  }, [client, refreshWorkingContext, scopeDraft, state]);

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
      setReviewStatus({ state: "pending", message: "Review decision pending" });
      const result = await client.submitReviewDecision({
        correlation_id: `review-${decision}-${sessionId}-${current.snapshot.revision}`,
        expected_revision: current.snapshot.revision,
        session_id: sessionId,
        decision,
        reason,
      });
      if (result.kind !== "acknowledged") {
        setReviewStatus({ state: result.kind, message: result.message });
        return;
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        setReviewStatus({ state: "rejected", message: "Review acknowledged but reload failed" });
        setConnectionStatus("offline");
        return;
      }
      setState(reloaded);
      setConnectionStatus("connected");
      setReviewStatus({
        state: "acknowledged",
        message: result.acknowledgement.effect_summary,
      });
      await refreshReviewWorkspace();
    },
    [client, refreshReviewWorkspace, state],
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
      setQueueStatus({ state: "pending", message: "Workspace Queue decision pending" });
      const result = await client.submitWorkspaceQueueDecision({
        correlation_id: `queue-${decision}-${itemId}-${workspaceQueue.revision}`,
        expected_revision: workspaceQueue.revision,
        item_id: itemId,
        decision,
        reason,
      });
      if (result.kind !== "acknowledged") {
        setQueueStatus({ state: result.kind, message: result.message });
        return;
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        setQueueStatus({ state: "rejected", message: "Queue acknowledged but reload failed" });
        setConnectionStatus("offline");
        return;
      }
      setState(reloaded);
      setConnectionStatus("connected");
      setQueueStatus({
        state: "acknowledged",
        message: result.acknowledgement.effect_summary,
      });
      await refreshWorkspaceQueue();
    },
    [client, refreshWorkspaceQueue, state, workspaceQueue],
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
      setMissionDraftStatus({ state: "pending", message: "Submitting Mission Draft decision." });
      const result = await client.submitMissionDraftDecision({
        correlation_id: `mission-draft-${decision}-${draftId}-${missionDrafts.revision}`,
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
        await refreshMissionDrafts();
        const snapshotResult = await client.loadSnapshot();
        setState(snapshotResult);
        return;
      }
      setMissionDraftStatus({ state: result.kind, message: result.message });
    },
    [client, missionDrafts, refreshMissionDrafts, state],
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
      setMissionDraftStatus({ state: "pending", message: "Creating Mission Draft." });
      const request: MissionDraftCreateRequest = {
        correlation_id: `mission-draft-create-${current.snapshot.revision}`,
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
        await refreshMissionDrafts();
        const snapshotResult = await client.loadSnapshot();
        setState(snapshotResult);
        return;
      }
      setMissionDraftStatus({ state: result.kind, message: result.message });
    },
    [client, refreshMissionDrafts, state],
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
      setQueueStatus({ state: "pending", message: "Ad Hoc Delegation proposal pending" });
      const request: AdHocDelegationProposalRequest = {
        correlation_id: `ad-hoc-delegation-${proposal.originatingMessageId}-${current.snapshot.revision}`,
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
        return;
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        setQueueStatus({ state: "rejected", message: "Proposal acknowledged but reload failed" });
        setConnectionStatus("offline");
        return;
      }
      setState(reloaded);
      setConnectionStatus("connected");
      setQueueStatus({
        state: "acknowledged",
        message: result.acknowledgement.effect_summary,
      });
      await refreshWorkspaceQueue();
    },
    [client, refreshWorkspaceQueue, state],
  );

  if (state === "loading") {
    return (
      <div className="boot-screen" role="status" aria-live="polite">
        <span className="boot-marker" aria-hidden="true" />
        <p>Connecting to Albert</p>
        <small>Waiting for an authoritative workspace snapshot</small>
      </div>
    );
  }

  if (state.kind !== "ready" && state.kind !== "empty") {
    return (
      <div className="boot-screen boot-screen--error" role="alert">
        <p>Command Deck unavailable</p>
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
      onAdHocProposal={submitAdHocDelegationProposal}
      onMissionDraftCreate={submitMissionDraftCreate}
      onMissionDraftReasonChange={(draftId, reason) =>
        setMissionDraftReasons((current) => ({ ...current, [draftId]: reason }))
      }
      onMissionDraftDecision={submitMissionDraftDecision}
    />
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
  onAdHocProposal,
  onMissionDraftCreate,
  onMissionDraftReasonChange,
  onMissionDraftDecision,
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
  onAdHocProposal: (proposal: AdHocDelegationDraft) => void;
  onMissionDraftCreate: (draft: MissionDraftCreateDraft) => void;
  onMissionDraftReasonChange: (draftId: string, reason: string) => void;
  onMissionDraftDecision: (
    draftId: string,
    decision: MissionDraftDecision,
    reason: string,
  ) => void;
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
  const [selectedIssueId, setSelectedIssueId] = useState<string | null>(null);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const selectedIssue =
    (selectedIssueId ? issueSlicesById.get(selectedIssueId) : null) ??
    snapshot.mission_board.issue_slices?.[0] ??
    null;
  return (
    <div className="command-deck">
      <header className="topbar">
        <div className="wordmark">
          <span className="wordmark__signal" aria-hidden="true" />
          <span>ALBERT</span>
          <small>MISSION CONTROL</small>
        </div>
        <div className="session-state">
          <span className="eyebrow">Workspace Session {snapshot.workspace_session.id}</span>
          <strong>{snapshot.workspace_session.workspace_path}</strong>
        </div>
        <span className="revision">STATE / {snapshot.revision.toString().padStart(4, "0")}</span>
      </header>

      <div className="deck-grid">
        <section className="agent-console" aria-label="Agent Console">
          <div className="panel-heading">
            <div>
              <span className="eyebrow">Persistent lane</span>
              <h2>Agent Console</h2>
            </div>
            <span className="connection-pill" role="status" aria-label="Connection status">
              {connectionStatus[0].toUpperCase() + connectionStatus.slice(1)}
            </span>
            {connectionStatus === "offline" ? (
              <button type="button" onClick={onReconnect}>Reconnect</button>
            ) : null}
          </div>

          <div className="console-history">
            {consoleHistory.length === 0 ? (
              <p className="system-line">Canonical workspace state restored.</p>
            ) : (
              consoleHistory.map((message) => (
                <article key={message.message_id} data-outcome={message.outcome}>
                  <p>{message.content}</p>
                  <small>{message.source} / {message.outcome}</small>
                </article>
              ))
            )}
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

          <label className="composer">
            <span className="sr-only">Message Albert</span>
            <textarea
              aria-label="Message Albert"
              placeholder="Steer the active scope…"
              rows={3}
              value={draft}
              onChange={(event) => onDraftChange(event.target.value)}
            />
            <button
              type="button"
              disabled={!draft.trim() || messageStatus === "pending"}
              onClick={onSend}
            >
              Send
            </button>
            {messageStatus ? (
              <span role="status" aria-label="Message status">{messageStatus}</span>
            ) : null}
          </label>
        </section>

        <main className="operations" aria-label="Operations Workspace">
          <nav className="view-rail" aria-label="Operations views">
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
                <span className="eyebrow">Restored operational view</span>
                <h1>{activeViewTitle}</h1>
                <p>This workspace is restored from acknowledged Orchestrator preferences.</p>
              </div>
            ) : <>
            <div className="mission-heading">
              <div>
                <span className="eyebrow">Active Mission / {mission?.id ?? "none"}</span>
                <h1>{mission?.title ?? "No active mission"}</h1>
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
                          <a key={attention.attention_id} href={`#${attention.queue_link.split("#").at(1) ?? ""}`}>
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
                    const ready = issue?.launch_eligible ?? snapshot.mission_board.ready_issue_ids.includes(issueId);
                    const blockers = issue?.blockers ?? [];
                    const lifecycle = issue?.lifecycle ?? (ready ? "Ready" : "Blocked");
                  return (
                    <article className="issue-node" key={issueId} data-selected={selectedIssue?.issue_id === issueId}>
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
            </>}
          </section>
        </main>
      </div>
    </div>
  );
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
        <span role="status" aria-label="Workspace Queue decision status" className="connection-pill">
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
            <span className="eyebrow">Agent Console origin</span>
            <h2>Ad Hoc Delegation</h2>
            <strong>{latestConsoleMessage?.message_id ?? "No Agent Console message"}</strong>
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
        <span role="status" aria-label="Mission Draft decision status" className="connection-pill">
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
        <span role="status" aria-label="Review decision status" className="connection-pill">
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
                    disabled={!reason.trim() || pending}
                    onClick={() => onDecision(item.session_id, "repair", reason)}
                  >
                    Repair
                  </button>
                  <button
                    type="button"
                    aria-label={`Escalate ${item.session_id}`}
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
    <section className="issue-inspector" aria-label="Issue Slice Inspector">
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
