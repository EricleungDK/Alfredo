import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactElement,
} from "react";
import type {
  MissionExecutionNodeState,
  MissionExecutionTreeNode,
  MissionExecutionTreeProjection,
  WorkstationGovernedAction,
  WorkstationDiffLink,
  WorkstationEvidenceLink,
} from "./workstation-projection";
import {
  executionRiskLabel,
  executionStateLabel,
  workstationActionConsequence,
  workstationActionKey,
  workstationActionStateId,
  workstationActionTargetId,
} from "./workstation-projection";
import type { ReviewDecision, WorkspaceQueueDecision } from "./contracts";

export type MissionExecutionOutputState = "unavailable" | "subscribing" | "subscribed";

interface MissionExecutionActionDraft {
  readonly reason: string;
  readonly agentId: string;
}

interface MissionExecutionActionState {
  readonly itemId: string;
  readonly state: string;
  readonly message: string;
}

export interface MissionExecutionTreeProps {
  readonly projection: MissionExecutionTreeProjection;
  readonly selectedNodeId: string | null;
  readonly onSelectNode: (nodeId: string) => void;
  readonly onCloseInspector: () => void;
  readonly outputLines: readonly string[];
  readonly outputState: MissionExecutionOutputState;
  readonly onOpenDiff?: (diff: WorkstationDiffLink, returnFocus?: HTMLElement | null) => void;
  readonly onOpenEvidence?: (
    missionId: string,
    sessionId: string,
    artifactRef: string,
    label: string,
    returnFocus?: HTMLElement | null,
  ) => void;
  readonly workstationActionDrafts?: Readonly<Record<string, MissionExecutionActionDraft>>;
  readonly onWorkstationActionDraftChange?: (
    key: string,
    draft: MissionExecutionActionDraft,
  ) => void;
  readonly onWorkstationAction?: (
    action: WorkstationGovernedAction,
    draft: MissionExecutionActionDraft,
  ) => void;
  readonly actionState?: MissionExecutionActionState | null;
  readonly reviewReasons?: Readonly<Record<string, string>>;
  readonly onReviewReasonChange?: (sessionId: string, reason: string) => void;
  readonly onReviewDecision?: (
    sessionId: string,
    decision: ReviewDecision,
    reason: string,
    missionId?: string,
  ) => void;
  readonly queueReasons?: Readonly<Record<string, string>>;
  readonly onQueueReasonChange?: (itemId: string, reason: string) => void;
  readonly onQueueDecision?: (
    itemId: string,
    decision: WorkspaceQueueDecision,
    reason: string,
  ) => void;
  readonly onOpenView?: (view: string) => void;
  readonly agentOptions?: readonly { readonly id: string; readonly model?: string }[];
}

export function MissionExecutionTree({
  projection,
  selectedNodeId,
  onSelectNode,
  onCloseInspector,
  outputLines,
  outputState,
  onOpenDiff,
  onOpenEvidence,
  workstationActionDrafts,
  onWorkstationActionDraftChange,
  onWorkstationAction,
  actionState,
  reviewReasons,
  onReviewReasonChange,
  onReviewDecision,
  queueReasons,
  onQueueReasonChange,
  onQueueDecision,
  onOpenView,
  agentOptions,
}: MissionExecutionTreeProps): ReactElement {
  const parentIds = useMemo(
    () => projection.nodes.filter((node) => node.child_ids.length > 0).map((node) => node.id),
    [projection.nodes],
  );
  const [expandedNodeIds, setExpandedNodeIds] = useState<ReadonlySet<string>>(
    () => new Set(parentIds),
  );
  const [focusedNodeId, setFocusedNodeId] = useState<string | null>(projection.root_id);
  const treeRef = useRef<HTMLDivElement>(null);
  const originatingFocusRef = useRef<HTMLElement | null>(null);
  const previousRootIdRef = useRef(projection.root_id);
  const previousParentIdsRef = useRef<ReadonlySet<string>>(new Set(parentIds));

  useEffect(() => {
    const rootChanged = previousRootIdRef.current !== projection.root_id;
    previousRootIdRef.current = projection.root_id;
    const previousParentIds = previousParentIdsRef.current;
    previousParentIdsRef.current = new Set(parentIds);
    setExpandedNodeIds((current) => {
      const known = new Set(projection.nodes.map((node) => node.id));
      const next = new Set([...current].filter((nodeId) => known.has(nodeId)));
      if (rootChanged || current.size === 0) parentIds.forEach((nodeId) => next.add(nodeId));
      else parentIds.forEach((nodeId) => {
        if (!previousParentIds.has(nodeId)) next.add(nodeId);
      });
      return next;
    });
  }, [parentIds, projection.revision]);

  const visibleNodes = useMemo(() => {
    if (!projection.root_id) return [];
    const byId = new Map(projection.nodes.map((node) => [node.id, node]));
    const result: MissionExecutionTreeNode[] = [];
    const visit = (nodeId: string): void => {
      const node = byId.get(nodeId);
      if (!node) return;
      result.push(node);
      if (!expandedNodeIds.has(node.id)) return;
      for (const childId of node.child_ids) visit(childId);
    };
    visit(projection.root_id);
    return result;
  }, [expandedNodeIds, projection.nodes, projection.root_id]);
  const nodesById = useMemo(
    () => new Map(projection.nodes.map((node) => [node.id, node] as const)),
    [projection.nodes],
  );

  useEffect(() => {
    if (focusedNodeId && visibleNodes.some((node) => node.id === focusedNodeId)) return;
    setFocusedNodeId(visibleNodes[0]?.id ?? null);
  }, [focusedNodeId, visibleNodes]);

  const focusNode = (nodeId: string | undefined): void => {
    if (!nodeId) return;
    setFocusedNodeId(nodeId);
    const target = [...(treeRef.current?.querySelectorAll<HTMLElement>('[data-execution-node-id]') ?? [])]
      .find((candidate) => candidate.dataset.executionNodeId === nodeId);
    target?.focus();
  };

  const toggleNode = (node: MissionExecutionTreeNode): void => {
    if (node.child_ids.length === 0) return;
    setExpandedNodeIds((current) => {
      const next = new Set(current);
      if (next.has(node.id)) next.delete(node.id);
      else next.add(node.id);
      return next;
    });
  };

  const handleNodeKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    node: MissionExecutionTreeNode,
  ): void => {
    const index = visibleNodes.findIndex((candidate) => candidate.id === node.id);
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (node.inspectable) {
        originatingFocusRef.current = event.currentTarget;
        onSelectNode(node.id);
      }
      else toggleNode(node);
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      focusNode(visibleNodes[index + 1]?.id ?? visibleNodes[0]?.id);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      focusNode(visibleNodes[index - 1]?.id ?? visibleNodes.at(-1)?.id);
      return;
    }
    if (event.key === "Home") {
      event.preventDefault();
      focusNode(visibleNodes[0]?.id);
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      focusNode(visibleNodes.at(-1)?.id);
      return;
    }
    if (event.key === "ArrowRight") {
      event.preventDefault();
      if (node.child_ids.length > 0 && !expandedNodeIds.has(node.id)) {
        toggleNode(node);
      } else {
        focusNode(visibleNodes[index + 1]?.id);
      }
      return;
    }
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      if (node.child_ids.length > 0 && expandedNodeIds.has(node.id)) {
        toggleNode(node);
      } else {
        focusNode(node.parent_id ?? undefined);
      }
    }
  };

  const selectedNode = projection.nodes.find((node) => node.id === selectedNodeId) ?? null;

  return (
    <section className="mission-execution-tree" aria-label="Mission Execution Tree">
      <header className="mission-execution-tree__heading">
        <div>
          <span className="eyebrow">Focus desk / canonical work</span>
          <h3>Mission Execution Tree</h3>
          <p>Work stays primary; every Local Agent session remains nested beneath the work it serves.</p>
        </div>
        <div className="mission-execution-tree__counts" aria-label="Mission Execution Tree counts">
          <span>{projection.counts.issue_slices} Issue Slices</span>
          <span>{projection.counts.ad_hoc_delegations} Ad Hoc Delegations</span>
          <span>{projection.counts.local_agent_sessions} Local Agent sessions</span>
          <span>{projection.counts.blockers} blockers</span>
          <span>{projection.counts.evidence_packages} Evidence Packages</span>
        </div>
      </header>

      {projection.root_id ? (
        <div
          ref={treeRef}
          className="mission-execution-tree__outline"
          role="tree"
          aria-label="Mission Execution Tree hierarchy"
          aria-multiselectable="false"
        >
          {visibleNodes.map((node) => (
            <button
              key={node.id}
              type="button"
              role="treeitem"
              className={`mission-execution-node mission-execution-node--${node.kind}`}
              data-state={node.state}
              data-risk={node.risk}
              data-lineage={node.lineage}
              data-execution-node-id={node.id}
              data-selected={selectedNodeId === node.id}
              aria-level={node.depth + 1}
              aria-setsize={
                node.parent_id ? nodesById.get(node.parent_id)?.child_ids.length ?? 1 : 1
              }
              aria-posinset={
                node.parent_id
                  ? (nodesById.get(node.parent_id)?.child_ids.indexOf(node.id) ?? 0) + 1
                  : 1
              }
              aria-selected={selectedNodeId === node.id}
              aria-label={`${node.kind === "agent-session" ? "Local Agent session" : node.kind} ${node.identity}; ${node.title}; ${nodeStateLabel(node.state)}; ${riskLabel(node.risk)}`}
              aria-expanded={node.child_ids.length > 0 ? expandedNodeIds.has(node.id) : undefined}
              tabIndex={
                focusedNodeId === node.id || (!focusedNodeId && node.id === visibleNodes[0]?.id)
                  ? 0
                  : -1
              }
              style={{
                marginInlineStart: `${Math.min(node.depth, 3) * 0.75}rem`,
                width: `calc(100% - ${Math.min(node.depth, 3) * 0.75}rem)`,
              }}
              onClick={() => {
                setFocusedNodeId(node.id);
                if (node.inspectable) {
                  originatingFocusRef.current = document.activeElement instanceof HTMLElement
                    ? document.activeElement
                    : null;
                  onSelectNode(node.id);
                }
                else toggleNode(node);
              }}
              onFocus={() => setFocusedNodeId(node.id)}
              onKeyDown={(event) => handleNodeKeyDown(event, node)}
            >
              <span
                className="mission-execution-node__shape"
                data-shape={node.shape}
                aria-hidden="true"
              />
              <span className="mission-execution-node__copy">
                <span className="mission-execution-node__identity">
                  {node.kind === "agent-session" ? "Local Agent session / " : `${node.kind} / `}
                  {node.identity}
                </span>
                <strong>{node.title}</strong>
                <small>{node.lineage === "repair" ? `Repair lineage · ${node.summary}` : node.summary}</small>
              </span>
              <span className="mission-execution-node__status">
                <span>{node.status}</span>
                <small>{nodeStateLabel(node.state)}</small>
              </span>
              {node.risk !== "none" ? (
                <span className="mission-execution-node__risk">{riskLabel(node.risk)}</span>
              ) : null}
              {node.child_ids.length > 0 ? (
                <span className="mission-execution-node__disclosure" aria-hidden="true">
                  {expandedNodeIds.has(node.id) ? "−" : "+"}
                </span>
              ) : null}
            </button>
          ))}
        </div>
      ) : (
        <div className="empty-state">
          <span className="eyebrow">Mission Work</span>
          <p>No Active Mission is available for the Mission Execution Tree.</p>
        </div>
      )}

      {selectedNode ? (
        <MissionExecutionInspector
          node={selectedNode}
          outputLines={outputLines}
          outputState={outputState}
          onClose={() => {
            const returnFocus = originatingFocusRef.current;
            onCloseInspector();
            requestAnimationFrame(() => {
              if (returnFocus?.isConnected) returnFocus.focus();
            });
          }}
          onOpenDiff={onOpenDiff}
          onOpenEvidence={onOpenEvidence}
          workstationActionDrafts={workstationActionDrafts}
          onWorkstationActionDraftChange={onWorkstationActionDraftChange}
          onWorkstationAction={onWorkstationAction}
          actionState={actionState}
          reviewReasons={reviewReasons}
          onReviewReasonChange={onReviewReasonChange}
          onReviewDecision={onReviewDecision}
          queueReasons={queueReasons}
          onQueueReasonChange={onQueueReasonChange}
          onQueueDecision={onQueueDecision}
          onOpenView={onOpenView}
          agentOptions={agentOptions}
        />
      ) : null}
    </section>
  );
}

function MissionExecutionInspector({
  node,
  outputLines,
  outputState,
  onClose,
  onOpenDiff,
  onOpenEvidence,
  workstationActionDrafts,
  onWorkstationActionDraftChange,
  onWorkstationAction,
  actionState,
  reviewReasons,
  onReviewReasonChange,
  onReviewDecision,
  queueReasons,
  onQueueReasonChange,
  onQueueDecision,
  onOpenView,
  agentOptions,
}: {
  readonly node: MissionExecutionTreeNode;
  readonly outputLines: readonly string[];
  readonly outputState: MissionExecutionOutputState;
  readonly onClose: () => void;
  readonly onOpenDiff?: (diff: WorkstationDiffLink, returnFocus?: HTMLElement | null) => void;
  readonly onOpenEvidence?: (
    missionId: string,
    sessionId: string,
    artifactRef: string,
    label: string,
    returnFocus?: HTMLElement | null,
  ) => void;
  readonly workstationActionDrafts?: Readonly<Record<string, MissionExecutionActionDraft>>;
  readonly onWorkstationActionDraftChange?: (
    key: string,
    draft: MissionExecutionActionDraft,
  ) => void;
  readonly onWorkstationAction?: (
    action: WorkstationGovernedAction,
    draft: MissionExecutionActionDraft,
  ) => void;
  readonly actionState?: MissionExecutionActionState | null;
  readonly reviewReasons?: Readonly<Record<string, string>>;
  readonly onReviewReasonChange?: (sessionId: string, reason: string) => void;
  readonly onReviewDecision?: (
    sessionId: string,
    decision: ReviewDecision,
    reason: string,
    missionId?: string,
  ) => void;
  readonly queueReasons?: Readonly<Record<string, string>>;
  readonly onQueueReasonChange?: (itemId: string, reason: string) => void;
  readonly onQueueDecision?: (
    itemId: string,
    decision: WorkspaceQueueDecision,
    reason: string,
  ) => void;
  readonly onOpenView?: (view: string) => void;
  readonly agentOptions?: readonly { readonly id: string; readonly model?: string }[];
}): ReactElement {
  const headingId = `mission-execution-inspector-${node.id.replace(/[^a-zA-Z0-9_-]+/g, "-")}`;
  const inspectorRef = useRef<HTMLElement>(null);
  useEffect(() => {
    inspectorRef.current?.focus();
  }, [node.id]);

  const issue = node.issue;
  const session = node.session;
  const card = node.card;
  const evidence = issue?.evidence ?? null;
  const task = session?.task_title || issue?.accepted_boundary.what_to_build || card?.currentTask || node.title;
  const currentActivity = session?.operation_status || session?.status || issue?.progress || card?.progress || "Not recorded";
  const latestUpdate = session?.test_results || session?.failure || card?.latestCommandOrTest || evidence?.test_results || "Not recorded";
  const evidenceSummary = session?.test_results || evidence?.test_results || card?.detail.reviewState.evidenceState || "No Evidence Package recorded.";
  const activityTrail = session
    ? [
        session.status ? `Session status: ${session.status}` : "",
        session.parent_session_id
          ? `Repair of Local Agent session: ${session.parent_session_id}`
          : "",
        session.operation_status ? `Current operation: ${session.operation_status}` : "",
        ...(session.commands_run ?? []).map((command) => `Command observed: ${command}`),
        session.changed_files?.length ? `${session.changed_files.length} touched file(s) observed` : "",
        session.evidence_correlation_id ? "Evidence Package acknowledged" : "",
        session.review_outcome ? `Review Decision: ${session.review_outcome}` : "",
        session.review_next_action ? `Next governed action: ${session.review_next_action}` : "",
      ].filter(Boolean)
    : [
        issue?.lifecycle ? `Issue lifecycle: ${issue.lifecycle}` : "",
        issue?.progress ? `Progress: ${issue.progress}` : "",
        evidence?.state ? `Evidence state: ${evidence.state}` : "",
      ].filter(Boolean);
  const actions = card?.detail.governedActions ?? [];
  const evidenceLinks = card?.detail.evidenceLinks ?? [];
  const files = session?.changed_files !== undefined
    ? session.changed_files.map((path) => ({ path, status: "touched" }))
    : card?.detail.filesTouched ?? evidence?.changed_files.map((path) => ({ path, status: "touched" })) ?? [];
  const role = session?.role ?? card?.role ?? node.kind;
  const model = session?.model ?? card?.model ?? "Not recorded";

  return (
    <section
      ref={inspectorRef}
      className="mission-execution-inspector"
      aria-label={`${node.identity} execution inspector`}
      tabIndex={-1}
    >
      <header className="mission-execution-inspector__heading">
        <div>
          <span className="eyebrow">{node.kind === "agent-session" ? "Local Agent session" : node.kind === "ad-hoc-delegation" ? "Ad Hoc Delegation" : "Issue Slice"}</span>
          <h4 id={headingId}>
            {node.kind === "agent-session" ? "Local Agent session / " : `${node.kind} / `}
            {node.identity}
          </h4>
          <p>{node.title}</p>
        </div>
        <div className="mission-execution-inspector__heading-actions">
          <span className={`status status--${node.state}`}>{node.status}</span>
          <button type="button" aria-label="Close Mission Execution Tree inspector" onClick={onClose}>
            Close inspector
          </button>
        </div>
      </header>

      <dl className="mission-execution-inspector__facts">
        <div><dt>Task</dt><dd>{task}</dd></div>
        <div><dt>Role</dt><dd>{role}</dd></div>
        <div><dt>Model</dt><dd>{model}</dd></div>
        <div><dt>Current activity</dt><dd>{currentActivity}</dd></div>
        <div><dt>Latest update</dt><dd>{latestUpdate}</dd></div>
        <div><dt>Risk</dt><dd>{riskLabel(node.risk)}</dd></div>
        <div><dt>Evidence risk</dt><dd>{issue?.evidence.risks.trim() || "None recorded."}</dd></div>
      </dl>

      {node.kind === "issue-slice" && issue ? (
        <section className="mission-execution-inspector__section">
          <h5>Dependencies</h5>
          {issue.blockers.length === 0 ? <p>No dependency blockers.</p> : (
            <ul>
              {issue.blockers.map((blocker) => (
                <li key={blocker.issue_id}>{blocker.issue_id} · {blocker.title} · {blocker.satisfied ? "satisfied" : "open"}</li>
              ))}
            </ul>
          )}
        </section>
      ) : null}

      <section className="mission-execution-inspector__section">
        <h5>Activity trail</h5>
        {activityTrail.length === 0 ? <p>No activity recorded.</p> : <ol>{activityTrail.map((entry) => <li key={entry}>{entry}</li>)}</ol>}
      </section>

      <section className="mission-execution-inspector__section">
        <h5>Evidence and touched files</h5>
        <p>{evidenceSummary}</p>
        {files.length > 0 ? (
          <ul className="mission-execution-inspector__files">
            {files.map((file) => <li key={file.path}><span>{file.path}</span><small>{file.status}</small></li>)}
          </ul>
        ) : <p>No touched files recorded.</p>}
        {evidenceLinks.length > 0 ? (
          <div className="mission-execution-inspector__links">
            {evidenceLinks.map((link: WorkstationEvidenceLink) => (
              <button
                key={link.href}
                type="button"
                onClick={(event) => onOpenEvidence?.(card?.missionId ?? "", link.sessionId, link.href, link.label, event.currentTarget)}
              >
                {link.label}
              </button>
            ))}
          </div>
        ) : null}
        {card?.detail.diffs.map((diff) => (
          <button key={`${diff.href}:${diff.path}`} type="button" onClick={(event) => onOpenDiff?.(diff, event.currentTarget)}>
            {diff.label}
          </button>
        ))}
      </section>

      {node.kind === "agent-session" ? (
        <section className="mission-execution-inspector__section mission-execution-inspector__output" aria-label={`${node.identity} detailed Local Agent output`}>
          <div className="mission-execution-inspector__section-heading">
            <h5>Detailed Local Agent output</h5>
            <span>{outputState === "subscribed" ? "Subscribed while this inspector is open" : outputState === "subscribing" ? "Subscribing…" : "Output subscription unavailable"}</span>
          </div>
          <pre aria-label="Detailed Local Agent output content">{outputLines.length > 0 ? outputLines.join("\n") : "No detailed output received yet."}</pre>
        </section>
      ) : null}

      <section className="mission-execution-inspector__section">
        <h5>Available governed actions</h5>
        {actions.length === 0 ? <p>No governed action is available from this canonical state.</p> : (
          <ul>
            {actions.map((action) => (
              <MissionExecutionGovernedAction
                key={`${action.label}:${action.actionType ?? "none"}`}
                action={action}
                actionState={actionState}
                workstationActionDrafts={workstationActionDrafts}
                onWorkstationActionDraftChange={onWorkstationActionDraftChange}
                onWorkstationAction={onWorkstationAction}
                reviewReasons={reviewReasons}
                onReviewReasonChange={onReviewReasonChange}
                onReviewDecision={onReviewDecision}
                queueReasons={queueReasons}
                onQueueReasonChange={onQueueReasonChange}
                onQueueDecision={onQueueDecision}
                onOpenView={onOpenView}
                agentOptions={agentOptions}
              />
            ))}
          </ul>
        )}
      </section>
    </section>
  );
}

function MissionExecutionGovernedAction({
  action,
  actionState,
  workstationActionDrafts,
  onWorkstationActionDraftChange,
  onWorkstationAction,
  reviewReasons,
  onReviewReasonChange,
  onReviewDecision,
  queueReasons,
  onQueueReasonChange,
  onQueueDecision,
  onOpenView,
  agentOptions,
}: {
  readonly action: WorkstationGovernedAction;
  readonly actionState?: MissionExecutionActionState | null;
  readonly workstationActionDrafts?: Readonly<Record<string, MissionExecutionActionDraft>>;
  readonly onWorkstationActionDraftChange?: (
    key: string,
    draft: MissionExecutionActionDraft,
  ) => void;
  readonly onWorkstationAction?: (
    action: WorkstationGovernedAction,
    draft: MissionExecutionActionDraft,
  ) => void;
  readonly reviewReasons?: Readonly<Record<string, string>>;
  readonly onReviewReasonChange?: (sessionId: string, reason: string) => void;
  readonly onReviewDecision?: (
    sessionId: string,
    decision: ReviewDecision,
    reason: string,
    missionId?: string,
  ) => void;
  readonly queueReasons?: Readonly<Record<string, string>>;
  readonly onQueueReasonChange?: (itemId: string, reason: string) => void;
  readonly onQueueDecision?: (
    itemId: string,
    decision: WorkspaceQueueDecision,
    reason: string,
  ) => void;
  readonly onOpenView?: (view: string) => void;
  readonly agentOptions?: readonly { readonly id: string; readonly model?: string }[];
}): ReactElement {
  const targetId = workstationActionTargetId(action);
  const actionKey = workstationActionKey(action);
  const isReview = action.actionType === "review-decision" && Boolean(action.sessionId && action.reviewDecision);
  const isQueue = action.actionType === "workspace-queue-decision" && Boolean(action.itemId && action.decision);
  const draft = workstationActionDrafts?.[actionKey] ?? { reason: "", agentId: "" };
  const reason = isReview && action.reviewDecision === "repair"
    ? reviewReasons?.[action.sessionId ?? ""] ?? ""
    : isQueue
      ? queueReasons?.[action.itemId ?? ""] ?? ""
      : draft.reason;
  const requiresAgent = action.actionType === "model-assignment-change";
  const actionIdentity = workstationActionStateId(action);
  const actionStatusId = actionState?.itemId === actionIdentity
    ? actionState
    : null;
  const disabled = Boolean(action.disabledReason) ||
    Boolean(actionStatusId?.state === "pending") ||
    (action.requiresReason && !reason.trim()) ||
    (requiresAgent && !draft.agentId.trim());
  const consequence = workstationActionConsequence(action);
  const actionGuidance = action.disabledReason
    ? `Unavailable: ${action.disabledReason}`
    : action.requiresReason && !reason.trim()
      ? `Enter a reason to enable ${action.label}${targetId ? ` for ${targetId}` : ""}.`
      : requiresAgent && !draft.agentId.trim()
        ? `Select an eligible local agent to enable ${action.label}${targetId ? ` for ${targetId}` : ""}.`
        : action.recoveryPath
          ? `Recovery: ${action.recoveryPath}`
          : null;
  const actionId = actionKey.replace(/[^a-zA-Z0-9_-]+/g, "-");
  const consequenceId = `mission-execution-action-consequence-${actionId}`;
  const guidanceId = `mission-execution-action-guidance-${actionId}`;

  const submit = (): void => {
    if (isQueue && action.itemId && action.decision) {
      onQueueDecision?.(action.itemId, action.decision, reason.trim());
      return;
    }
    if (isReview && action.sessionId && action.reviewDecision) {
      onReviewDecision?.(action.sessionId, action.reviewDecision, reason.trim(), action.missionId);
      return;
    }
    if (action.actionType) {
      onWorkstationAction?.(action, draft);
      return;
    }
    if (action.target !== "none") onOpenView?.(action.target);
  };

  return (
    <li className="mission-execution-inspector__action">
      <div className="mission-execution-inspector__action-heading">
        <strong>{action.label}</strong>
        {actionStatusId ? <span role="status">{actionStatusId.message}</span> : null}
      </div>
      <span id={consequenceId}>{consequence}</span>
      {actionGuidance ? (
        <small id={guidanceId} className="mission-execution-inspector__action-recovery">
          {actionGuidance}
        </small>
      ) : null}
      {requiresAgent ? (
        <label>
          <span>Agent</span>
          <select
            aria-label={`${action.label} agent`}
            value={draft.agentId}
            onChange={(event) =>
              onWorkstationActionDraftChange?.(actionKey, {
                ...draft,
                agentId: event.target.value,
              })
            }
          >
            <option value="">Select a local agent</option>
            {(agentOptions ?? []).map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.id}{agent.model ? ` · ${agent.model}` : ""}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      {action.requiresReason ? (
        <label>
          <span>Reason</span>
          <textarea
            rows={2}
            aria-label={`${action.label} reason`}
            value={reason}
            onChange={(event) => {
              if (isReview && action.sessionId) onReviewReasonChange?.(action.sessionId, event.target.value);
              else if (isQueue && action.itemId) onQueueReasonChange?.(action.itemId, event.target.value);
              else onWorkstationActionDraftChange?.(actionKey, { ...draft, reason: event.target.value });
            }}
          />
        </label>
      ) : null}
      <button
        type="button"
        aria-label={action.label}
        aria-describedby={[consequenceId, actionGuidance ? guidanceId : ""].filter(Boolean).join(" ")}
        disabled={disabled}
        onClick={submit}
      >
        {action.label}
      </button>
    </li>
  );
}

function nodeStateLabel(state: MissionExecutionNodeState): string {
  return executionStateLabel(state);
}

function riskLabel(risk: MissionExecutionTreeNode["risk"]): string {
  return executionRiskLabel(risk);
}
