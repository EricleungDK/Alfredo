/**
 * PROTOTYPE — throw this away after resolving the Mission Execution Tree decision.
 *
 * Three variants of Mission Work, switchable via `?variant=`, at the dedicated
 * mission-execution-tree Vite mode root.
 */
import {
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import "./mission-execution-tree-prototype.css";
import { SupervisionReviewPrototype } from "./SupervisionReviewPrototype";

type VariantKey = "A" | "B" | "C";
type MissionView = "outline" | "archive";
type WorkKind = "issue" | "ad-hoc" | "session";
type WorkStatus =
  | "working"
  | "queued"
  | "evidence-ready"
  | "repair-ready"
  | "failed"
  | "blocked"
  | "complete";
type AttentionKind = "decision" | "risk" | "watch" | null;
type GovernedAction =
  | "Review evidence"
  | "Launch repair"
  | "Retry session"
  | "Cancel session"
  | "Inspect blocker"
  | "View evidence"
  | "Archive ticket"
  | "Restore to outline";

interface WorkDetails {
  readonly task: string;
  readonly latest: string;
  readonly command: string;
  readonly files: readonly string[];
  readonly evidence: readonly string[];
  readonly trail: readonly string[];
}

interface WorkNode {
  readonly id: string;
  readonly kind: WorkKind;
  readonly title: string;
  readonly owner: string;
  readonly status: WorkStatus;
  readonly attention: AttentionKind;
  readonly summary: string;
  readonly nextAction: string;
  readonly progress: number;
  readonly updated: string;
  readonly actions: readonly GovernedAction[];
  readonly details: WorkDetails;
  readonly children: readonly WorkNode[];
}

interface NodeLocation {
  readonly node: WorkNode;
  readonly parents: readonly WorkNode[];
  readonly depth: number;
}

interface BlockerRecommendation {
  readonly blockedIssueId: string;
  readonly consequence: "create-issue" | "launch-session";
  readonly outputId: string;
  readonly action: string;
  readonly rationale: string;
  readonly acceptance: string;
  readonly assignedTo: string;
}

const VARIANTS: ReadonlyArray<{
  readonly key: VariantKey;
  readonly name: string;
}> = [
  { key: "A", name: "Work spine" },
  { key: "B", name: "Attention lanes" },
  { key: "C", name: "Focus desk" },
];

const MISSION_NODES: readonly WorkNode[] = [
  {
    id: "ISS-53",
    kind: "issue",
    title: "Prototype attention-driven Local Agent supervision",
    owner: "Codex · Frontier Architect",
    status: "evidence-ready",
    attention: "decision",
    summary:
      "A Variant C review now shows what Alfredo notices, surfaces, and may safely do.",
    nextAction: "Try the supervision cases and choose the policy",
    progress: 96,
    updated: "Just now",
    actions: ["View evidence"],
    details: {
      task:
        "Choose how Alfredo joins canonical Local Agent state with independent liveness, durable attention, and safe recovery.",
      latest:
        "Mounted five plain-language supervision cases inside the approved Focus desk.",
      command: "npm run prototype:tree",
      files: [
        "SupervisionReviewPrototype.tsx",
        "supervision-review-prototype.css",
      ],
      evidence: [
        "23 deterministic fault comparisons",
        "8 exposed invariants",
        "10,000 randomized action sequences",
      ],
      trail: [
        "Separated canonical state from advisory observations",
        "Verified attention-before-cursor restart cuts",
        "Prepared Mission Commander policy review",
      ],
    },
    children: [
      {
        id: "session-53-codex",
        kind: "session",
        title: "Supervision interaction prototype",
        owner: "Codex · Frontier Model",
        status: "evidence-ready",
        attention: null,
        summary:
          "Disposable scenario explorer complete; no product or Mission state changed.",
        nextAction: "Await the ISS-53 review decision",
        progress: 100,
        updated: "Just now",
        actions: ["View evidence"],
        details: {
          task:
            "Make the supervision state model understandable inside Variant C.",
          latest:
            "Healthy, missed completion, stopped runner, restart, and conflicting-fact cases are interactive.",
          command:
            "python3 -m albert_mvp.prototypes.supervision_loop --demo all",
          files: [
            "SupervisionReviewPrototype.tsx",
            "supervision-review-prototype.css",
          ],
          evidence: [
            "Token-free healthy path",
            "Idempotent restart replay",
            "Fail-closed identity and result cases",
          ],
          trail: [
            "Completed pure reducer evidence",
            "Corrected the terminal-only handoff",
            "Mounted the review in Mission Work",
          ],
        },
        children: [],
      },
    ],
  },
  {
    id: "ISS-46",
    kind: "issue",
    title: "Prototype the Mission Execution Tree",
    owner: "Codex · Frontier Architect",
    status: "complete",
    attention: null,
    summary:
      "Focus desk / Variant C was approved by the Mission Commander.",
    nextAction: "No action",
    progress: 100,
    updated: "Yesterday",
    actions: ["View evidence"],
    details: {
      task:
        "Choose the hierarchy, summaries, attention rules, and governed actions for Mission Work.",
      latest:
        "The approved Focus desk remains the Mission Work host for later prototypes.",
      command: "npm run prototype:tree",
      files: [
        "MissionExecutionTreePrototype.tsx",
        "mission-execution-tree-prototype.css",
      ],
      evidence: [
        "3 structurally different variants",
        "Desktop + constrained-width reflow",
        "Keyboard-visible expansion and action controls",
      ],
      trail: [
        "Loaded the accepted conversational-workstation direction",
        "Preserved Orchestrator-owned action truth",
        "Captured the approved Variant C decision",
      ],
    },
    children: [
      {
        id: "session-46-codex",
        kind: "session",
        title: "Interaction prototype session",
        owner: "Codex · Frontier Model",
        status: "evidence-ready",
        attention: null,
        summary:
          "Prototype artifact complete; no production workstation code changed.",
        nextAction: "Await parent evidence decision",
        progress: 100,
        updated: "2 min ago",
        actions: ["View evidence"],
        details: {
          task:
            "Create a disposable UI artifact that makes the supervision trade-offs concrete.",
          latest:
            "All variants share the same canonical-looking state and expose their local UI state.",
          command: "npm run typecheck",
          files: [
            "MissionExecutionTreePrototype.tsx",
            "mission-execution-tree-prototype.css",
          ],
          evidence: [
            "TypeScript boundary",
            "Read-only action simulations",
            "Visible prototype state",
          ],
          trail: [
            "Mapped the current Mission Work contract",
            "Added three UI structures",
            "Stopped before product implementation",
          ],
        },
        children: [
          {
            id: "ADHOC-17",
            kind: "ad-hoc",
            title: "Check screen-reader hierarchy",
            owner: "Gemma 4 12B · Accessibility scout",
            status: "working",
            attention: "watch",
            summary:
              "Reading tree labels, expansion state, and action-dialog focus order.",
            nextAction: "Monitor; no decision needed",
            progress: 61,
            updated: "18 sec ago",
            actions: ["Cancel session"],
            details: {
              task:
                "Inspect whether the prototype hierarchy and action previews remain understandable without color.",
              latest:
                "Confirmed that every status and attention marker has a text label.",
              command: "accessibility tree snapshot",
              files: ["MissionExecutionTreePrototype.tsx"],
              evidence: ["Landmark inventory", "Accessible-name snapshot"],
              trail: [
                "Received bounded accessibility brief",
                "Inspected hierarchy semantics",
                "Checking constrained-width reading order",
              ],
            },
            children: [
              {
                id: "session-17-gemma",
                kind: "session",
                title: "Accessibility scout session",
                owner: "Gemma 4 12B · Ollama",
                status: "working",
                attention: null,
                summary:
                  "Evaluating names, roles, state text, and focus visibility.",
                nextAction: "Continue bounded inspection",
                progress: 61,
                updated: "18 sec ago",
                actions: ["Cancel session"],
                details: {
                  task: "Return an accessibility observation report only.",
                  latest:
                    "The status vocabulary does not rely on colored dots alone.",
                  command: "inspect accessibility snapshot",
                  files: [],
                  evidence: ["In-progress observation notes"],
                  trail: [
                    "Session acknowledged",
                    "Prototype opened at compact width",
                    "Reading order inspection active",
                  ],
                },
                children: [],
              },
            ],
          },
        ],
      },
    ],
  },
  {
    id: "ISS-48",
    kind: "issue",
    title: "Prototype a Rust Orchestrator vertical slice",
    owner: "Qwen 2.5 Coder 14B · Local Agent",
    status: "working",
    attention: "risk",
    summary:
      "Receipt recovery is progressing, but one nested reconciliation check failed.",
    nextAction: "Decide whether to launch the prepared repair",
    progress: 54,
    updated: "1 min ago",
    actions: ["Cancel session"],
    details: {
      task:
        "Compare one Rust authority slice against the current Python receipt and recovery contract.",
      latest:
        "The main slice is healthy; its delegated stale-receipt scenario needs repair.",
      command: "cargo test receipt_recovery",
      files: ["src/receipt.rs", "tests/receipt_recovery.rs"],
      evidence: ["18 passing checks", "1 failed reconciliation scenario"],
      trail: [
        "Created isolated worktree",
        "Implemented the bounded slice",
        "Delegated stale-receipt reproduction",
      ],
    },
    children: [
      {
        id: "session-48-qwen",
        kind: "session",
        title: "Rust authority slice session",
        owner: "Qwen 2.5 Coder 14B · Ollama",
        status: "working",
        attention: null,
        summary:
          "Running focused recovery tests while delegated evidence is repaired.",
        nextAction: "Continue current test pass",
        progress: 54,
        updated: "1 min ago",
        actions: ["Cancel session"],
        details: {
          task:
            "Implement and measure an exact-replay receipt boundary in the prototype worktree.",
          latest: "18 focused tests pass; parent session remains active.",
          command: "cargo test receipt_recovery -- --nocapture",
          files: ["src/receipt.rs", "tests/receipt_recovery.rs"],
          evidence: ["Focused test log", "Prototype diff"],
          trail: [
            "Task packet validated",
            "Rust prototype compiled",
            "Recovery tests active",
          ],
        },
        children: [
          {
            id: "ADHOC-21",
            kind: "ad-hoc",
            title: "Reproduce stale receipt substitution",
            owner: "Gemma 4 26B · Repair candidate",
            status: "repair-ready",
            attention: "decision",
            summary:
              "Initial reproduction used the wrong Mission identity; a reviewed repair is ready.",
            nextAction: "Launch exactly one repair",
            progress: 100,
            updated: "4 min ago",
            actions: ["Launch repair", "View evidence"],
            details: {
              task:
                "Reproduce cross-Mission receipt substitution without changing production state.",
              latest:
                "Frontier review recorded a repair reason and preserved the prior evidence boundary.",
              command: "cargo test rejects_cross_mission_receipt",
              files: ["tests/receipt_recovery.rs"],
              evidence: [
                "Failed test transcript",
                "Review reason: wrong Mission fixture",
                "Repair packet staged",
              ],
              trail: [
                "Nested delegation approved",
                "Evidence returned with one invalid fixture",
                "Repair review persisted",
              ],
            },
            children: [
              {
                id: "session-21-gemma",
                kind: "session",
                title: "Stale receipt reproduction session",
                owner: "Gemma 4 26B · Ollama",
                status: "failed",
                attention: null,
                summary:
                  "Terminal unsuccessful session; evidence remains available for repair.",
                nextAction: "Use parent repair action",
                progress: 100,
                updated: "4 min ago",
                actions: ["View evidence"],
                details: {
                  task: "Produce a bounded failing receipt-recovery fixture.",
                  latest:
                    "The fixture failed for the wrong reason and was rejected for repair.",
                  command: "cargo test rejects_cross_mission_receipt",
                  files: ["tests/receipt_recovery.rs"],
                  evidence: ["Failure output", "Frontier review"],
                  trail: [
                    "Session started",
                    "Evidence package validated",
                    "Review outcome: repair required",
                  ],
                },
                children: [],
              },
            ],
          },
        ],
      },
    ],
  },
  {
    id: "ISS-47",
    kind: "issue",
    title: "Identify the local-model optimization strategy",
    owner: "Unassigned",
    status: "blocked",
    attention: null,
    summary:
      "Waiting for architecture evidence before choosing model-residency and scheduling work.",
    nextAction: "Inspect architecture blocker",
    progress: 0,
    updated: "Yesterday",
    actions: ["Inspect blocker"],
    details: {
      task:
        "Identify optimizations across model startup, residency, scheduling, and outcome latency.",
      latest:
        "The ticket is intentionally not launchable until its architecture dependency closes.",
      command: "No process started",
      files: [],
      evidence: ["Baseline latency report"],
      trail: [
        "Research question specified",
        "Native dependency recorded",
        "Waiting without consuming a worker",
      ],
    },
    children: [],
  },
  {
    id: "ISS-44",
    kind: "issue",
    title: "Prototype the Coding Workspace-to-Mission journey",
    owner: "Codex · Frontier Architect",
    status: "complete",
    attention: null,
    summary:
      "Conversational workstation direction accepted by the Mission Commander.",
    nextAction: "No action",
    progress: 100,
    updated: "3 days ago",
    actions: ["View evidence"],
    details: {
      task:
        "Choose the interaction from Starting Location through safe-frontier dispatch.",
      latest:
        "Variant A was accepted with a steady, directly inspectable Mission Execution Tree.",
      command: "npm run test:prototype-journey",
      files: ["WorkspaceMissionJourneyPrototype.tsx"],
      evidence: ["Mission Commander decision", "4/4 prototype checks"],
      trail: [
        "Compared three variants",
        "Refined the conversational direction",
        "Captured the accepted decision",
      ],
    },
    children: [],
  },
];

const BLOCKER_RECOMMENDATION: BlockerRecommendation = {
  blockedIssueId: "ISS-47",
  consequence: "create-issue",
  outputId: "ISS-49",
  action: "Create a tracked architecture-decision Issue Slice",
  rationale:
    "The required architecture decision has no tracked Issue Slice to own its evidence and completion state.",
  acceptance:
    "Record the selected backend boundary, rejected alternatives, migration seam, and evidence needed before model optimization can begin.",
  assignedTo: "Codex · Frontier Architect",
};

const RECOMMENDED_BLOCKER_ISSUE: WorkNode = {
  id: BLOCKER_RECOMMENDATION.outputId,
  kind: "issue",
  title: "Choose Alfredo's backend modernization architecture",
  owner: BLOCKER_RECOMMENDATION.assignedTo,
  status: "queued",
  attention: null,
  summary:
    "Created from the Mission Commander's approved recommendation for the ISS-47 blocker.",
  nextAction: "Review the accepted brief, then launch a Local Agent session",
  progress: 0,
  updated: "Just now",
  actions: [],
  details: {
    task: BLOCKER_RECOMMENDATION.acceptance,
    latest:
      "The Issue Slice and dependency link were created; no Local Agent session has started.",
    command: "No process started",
    files: [],
    evidence: ["Mission Commander approval receipt", "ISS-47 dependency link"],
    trail: [
      "Orchestrator recommended a tracked Issue Slice",
      "Mission Commander approved the exact consequence",
      "Blocking dependency linked to ISS-47",
    ],
  },
  children: [],
};

function missionNodesWithApprovedBlocker(
  approved: boolean,
): readonly WorkNode[] {
  if (!approved) return MISSION_NODES;
  const blockerIndex = MISSION_NODES.findIndex(
    (node) => node.id === BLOCKER_RECOMMENDATION.blockedIssueId,
  );
  return [
    ...MISSION_NODES.slice(0, blockerIndex),
    RECOMMENDED_BLOCKER_ISSUE,
    ...MISSION_NODES.slice(blockerIndex),
  ];
}

function flattenNodes(
  nodes: readonly WorkNode[],
  parents: readonly WorkNode[] = [],
): readonly NodeLocation[] {
  return nodes.flatMap((node) => [
    { node, parents, depth: parents.length },
    ...flattenNodes(node.children, [...parents, node]),
  ]);
}

const ALL_LOCATIONS = flattenNodes(MISSION_NODES);
const MISSION_COUNTS = {
  activeAgentSessions: ALL_LOCATIONS.filter(
    ({ node }) => node.kind === "session" && node.status === "working",
  ).length,
  adHoc: ALL_LOCATIONS.filter(({ node }) => node.kind === "ad-hoc").length,
  blockedRecords: ALL_LOCATIONS.filter(
    ({ node }) => node.status === "blocked",
  ).length,
  issueSlices: ALL_LOCATIONS.filter(({ node }) => node.kind === "issue").length,
  needsYouRecords: ALL_LOCATIONS.filter(
    ({ node }) => node.attention === "decision" || node.attention === "risk",
  ).length,
  sessions: ALL_LOCATIONS.filter(({ node }) => node.kind === "session").length,
  totalRecords: ALL_LOCATIONS.length,
} as const;

function countTreeRecords(nodes: readonly WorkNode[]) {
  const locations = flattenNodes(nodes);
  return {
    adHoc: locations.filter(({ node }) => node.kind === "ad-hoc").length,
    issueSlices: locations.filter(({ node }) => node.kind === "issue").length,
    sessions: locations.filter(({ node }) => node.kind === "session").length,
    totalRecords: locations.length,
  } as const;
}

function variantFromLocation(): VariantKey {
  const params = new URLSearchParams(window.location.search);
  if (params.get("review") === "supervision") return "C";
  const candidate = params.get("variant");
  return candidate === "B" || candidate === "C" ? candidate : "A";
}

function supervisionReviewFromLocation(): boolean {
  return new URLSearchParams(window.location.search).get("review") === "supervision";
}

function statusLabel(status: WorkStatus): string {
  const labels: Record<WorkStatus, string> = {
    working: "Working",
    queued: "Queued",
    "evidence-ready": "Evidence ready",
    "repair-ready": "Repair ready",
    failed: "Failed",
    blocked: "Blocked",
    complete: "Complete",
  };
  return labels[status];
}

function kindLabel(kind: WorkKind): string {
  if (kind === "ad-hoc") return "Ad Hoc Delegation";
  if (kind === "session") return "Local Agent session";
  return "Issue Slice";
}

function attentionLabel(attention: Exclude<AttentionKind, null>): string {
  if (attention === "decision") return "Decision needed";
  if (attention === "risk") return "Risk";
  return "Watch";
}

function StatusPill({
  status,
  attention,
}: {
  readonly status: WorkStatus;
  readonly attention?: AttentionKind;
}) {
  return (
    <span className="met-status-group">
      <span className="met-status" data-status={status}>
        <i aria-hidden="true" />
        {statusLabel(status)}
      </span>
      {attention && (
        <span className="met-attention" data-attention={attention}>
          {attentionLabel(attention)}
        </span>
      )}
    </span>
  );
}

function ProgressMeter({ value }: { readonly value: number }) {
  return (
    <span
      aria-label={`${value}% progress`}
      aria-valuemax={100}
      aria-valuemin={0}
      aria-valuenow={value}
      className="met-progress"
      role="progressbar"
    >
      <span style={{ width: `${value}%` }} />
    </span>
  );
}

function NodeGlyph({ kind }: { readonly kind: WorkKind }) {
  const glyph =
    kind === "issue" ? (
      <svg viewBox="0 0 18 18">
        <path d="M3 3.5h12v3a2 2 0 0 0 0 4v3H3v-3a2 2 0 0 0 0-4v-3Z" />
        <path d="M9 4v2M9 8v2M9 12v1" />
      </svg>
    ) : kind === "session" ? (
      <svg viewBox="0 0 18 18">
        <path d="M9 2v2" />
        <path d="M6.25 4h5.5A2.25 2.25 0 0 1 14 6.25v4.5A2.25 2.25 0 0 1 11.75 13h-5.5A2.25 2.25 0 0 1 4 10.75v-4.5A2.25 2.25 0 0 1 6.25 4Z" />
        <path d="M6.5 8h.01M11.5 8h.01M7 11h4M6 13v2M12 13v2M4 7H2.5M15.5 7H14" />
      </svg>
    ) : (
      <svg viewBox="0 0 18 18">
        <circle cx="5" cy="4" r="1.5" />
        <circle cx="13" cy="9" r="1.5" />
        <circle cx="5" cy="14" r="1.5" />
        <path d="M5 5.5v2A1.5 1.5 0 0 0 6.5 9h5M5 12.5v-2A1.5 1.5 0 0 1 6.5 9" />
      </svg>
    );

  return (
    <span aria-hidden="true" className="met-node-glyph" data-kind={kind}>
      {glyph}
    </span>
  );
}

function TreeBranch({
  nodes,
  expanded,
  getInlineAction,
  selectedId,
  compact = false,
  onInlineAction,
  onSelect,
  onToggle,
}: {
  readonly nodes: readonly WorkNode[];
  readonly expanded: ReadonlySet<string>;
  readonly getInlineAction?: (
    node: WorkNode,
  ) =>
    | Extract<GovernedAction, "Archive ticket" | "Restore to outline">
    | undefined;
  readonly selectedId: string;
  readonly compact?: boolean;
  readonly onInlineAction?: (
    nodeId: string,
    action: Extract<
      GovernedAction,
      "Archive ticket" | "Restore to outline"
    >,
  ) => void;
  readonly onSelect: (id: string) => void;
  readonly onToggle: (id: string) => void;
}) {
  return (
    <ul className="met-tree" data-compact={compact || undefined} role="tree">
      {nodes.map((node) => {
        const hasChildren = node.children.length > 0;
        const isExpanded = expanded.has(node.id);
        const isSelected = node.id === selectedId;
        const inlineAction = getInlineAction?.(node);
        return (
          <li
            aria-expanded={hasChildren ? isExpanded : undefined}
            aria-selected={isSelected}
            key={node.id}
            role="treeitem"
          >
            <div
              className="met-tree-row"
              data-attention={node.attention ?? undefined}
              data-kind={node.kind}
              data-selected={isSelected || undefined}
            >
              <button
                aria-label={
                  hasChildren
                    ? `${isExpanded ? "Collapse" : "Expand"} ${node.title}`
                    : `${node.title} has no children`
                }
                className="met-tree-toggle"
                data-state={
                  hasChildren
                    ? isExpanded
                      ? "expanded"
                      : "collapsed"
                    : "leaf"
                }
                disabled={!hasChildren}
                onClick={() => onToggle(node.id)}
                title={
                  hasChildren
                    ? `${isExpanded ? "Collapse" : "Expand"} ${node.title}`
                    : undefined
                }
                type="button"
              >
                {hasChildren ? (
                  <svg aria-hidden="true" viewBox="0 0 16 16">
                    <path d="M3 8h10" />
                    {!isExpanded && <path d="M8 3v10" />}
                  </svg>
                ) : (
                  <span aria-hidden="true">·</span>
                )}
              </button>
              <button
                aria-current={isSelected ? "true" : undefined}
                className="met-tree-select"
                onClick={() => onSelect(node.id)}
                type="button"
              >
                <NodeGlyph kind={node.kind} />
                <span className="met-tree-copy">
                  <span className="met-tree-heading">
                    <span className="met-tree-id">{node.id}</span>
                    <strong>{node.title}</strong>
                  </span>
                  {!compact && <span>{node.summary}</span>}
                  <span className="met-tree-meta">
                    <span>{node.owner}</span>
                    <span>{node.updated}</span>
                  </span>
                </span>
              </button>
              <span className="met-tree-state">
                <StatusPill attention={node.attention} status={node.status} />
                {!compact && <ProgressMeter value={node.progress} />}
              </span>
              {inlineAction && onInlineAction ? (
                <button
                  aria-label={`${inlineAction} ${node.id}`}
                  className="met-tree-inline-action"
                  data-action={inlineAction}
                  onClick={() => onInlineAction(node.id, inlineAction)}
                  type="button"
                >
                  <span aria-hidden="true">
                    <svg viewBox="0 0 18 18">
                      <path d="M3 6h12v9H3zM2 3h14v3H2zM7 9h4" />
                    </svg>
                  </span>
                  {inlineAction === "Archive ticket" ? "Archive" : "Restore"}
                </button>
              ) : null}
            </div>
            {hasChildren && isExpanded && (
              <TreeBranch
                compact={compact}
                expanded={expanded}
                getInlineAction={getInlineAction}
                nodes={node.children}
                onInlineAction={onInlineAction}
                onSelect={onSelect}
                onToggle={onToggle}
                selectedId={selectedId}
              />
            )}
          </li>
        );
      })}
    </ul>
  );
}

function Breadcrumbs({ location }: { readonly location: NodeLocation }) {
  return (
    <ol aria-label="Work hierarchy" className="met-breadcrumbs">
      {[...location.parents, location.node].map((node) => (
        <li key={node.id}>{node.id}</li>
      ))}
    </ol>
  );
}

function Inspector({
  location,
  onAction,
}: {
  readonly location: NodeLocation;
  readonly onAction: (action: GovernedAction) => void;
}) {
  const { node } = location;
  return (
    <section aria-label={`${node.title} inspector`} className="met-inspector">
      <header className="met-inspector__header">
        <div>
          <Breadcrumbs location={location} />
          <span>{kindLabel(node.kind)}</span>
          <h3>{node.title}</h3>
          <p>{node.summary}</p>
        </div>
        <StatusPill attention={node.attention} status={node.status} />
      </header>

      <dl className="met-inspector__facts">
        <div>
          <dt>Owner</dt>
          <dd>{node.owner}</dd>
        </div>
        <div>
          <dt>Next action</dt>
          <dd>{node.nextAction}</dd>
        </div>
        <div>
          <dt>Last activity</dt>
          <dd>{node.updated}</dd>
        </div>
        <div>
          <dt>Progress</dt>
          <dd>
            {node.progress}% <ProgressMeter value={node.progress} />
          </dd>
        </div>
      </dl>

      {node.actions.includes("Review evidence") && (
        <aside className="met-review-help">
          <strong>Decision needed · three review outcomes</strong>
          <p>
            Accept evidence, request repair, or ask for human review. None of
            these choices starts a Local Agent automatically.
          </p>
        </aside>
      )}

      <div className="met-inspector__detail-grid">
        <article>
          <span>Accepted task</span>
          <p>{node.details.task}</p>
        </article>
        <article>
          <span>Latest update</span>
          <p>{node.details.latest}</p>
        </article>
        <article>
          <span>Latest command / test</span>
          <code>{node.details.command}</code>
        </article>
        <article>
          <span>Touched files</span>
          <p>
            {node.details.files.length > 0
              ? node.details.files.join(" · ")
              : "No files touched"}
          </p>
        </article>
      </div>

      <div className="met-inspector__lower">
        <section aria-label="Evidence">
          <h4>Evidence</h4>
          <ul>
            {node.details.evidence.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </section>
        <section aria-label="Activity trail">
          <h4>Activity trail</h4>
          <ol>
            {node.details.trail.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ol>
        </section>
      </div>

      <footer className="met-inspector__actions">
        <span>Governed actions</span>
        {node.actions.map((action) => (
          <button
            data-action={action}
            key={action}
            onClick={() => onAction(action)}
            type="button"
          >
            {action}
          </button>
        ))}
        {node.actions.length === 0 ? (
          <p>No action available in the current canonical state.</p>
        ) : null}
      </footer>
    </section>
  );
}

function AttentionStrip({
  locations,
  onSelect,
}: {
  readonly locations: readonly NodeLocation[];
  readonly onSelect: (id: string) => void;
}) {
  const attentionItems = locations.filter(
    ({ node }) => node.attention === "decision" || node.attention === "risk",
  );
  return (
    <section aria-label="Mission attention" className="met-attention-strip">
      <header>
        <span>Needs you</span>
        <strong>{attentionItems.length} decisions or risks</strong>
      </header>
      <div>
        {attentionItems.map(({ node }) => (
          <button key={node.id} onClick={() => onSelect(node.id)} type="button">
            <span data-attention={node.attention}>
              {node.attention ? attentionLabel(node.attention) : "Attention"}
            </span>
            <strong>{node.title}</strong>
            <small>{node.nextAction}</small>
          </button>
        ))}
      </div>
    </section>
  );
}

function ConsoleContext() {
  const [draft, setDraft] = useState("");
  const [previewPrompt, setPreviewPrompt] = useState<string | null>(null);

  const previewSend = () => {
    const prompt = draft.trim();
    if (!prompt) return;
    setPreviewPrompt(prompt);
    setDraft("");
  };

  return (
    <section aria-label="Agent Console" className="met-console">
      <header className="met-console__header">
        <div>
          <span>AGENT CONSOLE</span>
          <strong>Reliable Alfredo modernization</strong>
        </div>
        <span className="met-console__connection">
          <i aria-hidden="true" />
          Local · connected
        </span>
      </header>
      <div className="met-console__transcript">
        <article data-role="user">
          <span>you</span>
          <p>
            Show me how Alfredo notices a finished, stopped, or unreachable
            Local Agent inside the approved Focus desk.
          </p>
        </article>
        <article data-role="assistant">
          <span>alfredo · orchestrator</span>
          <p>
            ISS-53 is selected in Mission Work. Its review keeps canonical
            Mission state, independent checks, operational attention, and typed
            Orchestrator effects visibly separate.
          </p>
        </article>
        <article data-role="event">
          <span>canonical receipt</span>
          <p>
            No action taken · {MISSION_COUNTS.needsYouRecords} decision / risk
            records · {MISSION_COUNTS.activeAgentSessions} active Local Agent
            sessions
          </p>
        </article>
        <article data-role="assistant">
          <span>alfredo · mission</span>
          <p>
            Select any work item in Mission Work to inspect its exact task,
            current activity, evidence, and available governed action.
          </p>
        </article>
        {previewPrompt ? (
          <>
            <article data-role="user">
              <span>you · prototype prompt</span>
              <p>{previewPrompt}</p>
            </article>
            <article aria-live="polite" data-role="event">
              <span>prototype receipt</span>
              <p>
                Prompt captured in this throwaway prototype · no backend or
                Mission state changed
              </p>
            </article>
          </>
        ) : null}
      </div>
      <div className="met-console__composer">
        <span>you@alfredo:~$</span>
        <textarea
          aria-label="Message Alfredo"
          onChange={(event) => setDraft(event.target.value)}
          placeholder="Continue the Mission conversation…"
          value={draft}
        />
        <button disabled={!draft.trim()} onClick={previewSend} type="button">
          Send
        </button>
      </div>
    </section>
  );
}

function MissionHeader() {
  return (
    <header className="met-mission-header">
      <div>
        <span>MISSION WORK · ALFREDO</span>
        <h2>Reliable, observable, and faster modernization</h2>
      </div>
      <dl>
        <div>
          <dt>Active Local Agents</dt>
          <dd>{MISSION_COUNTS.activeAgentSessions} sessions</dd>
        </div>
        <div>
          <dt>Needs you</dt>
          <dd>{MISSION_COUNTS.needsYouRecords} decision / risk records</dd>
        </div>
        <div>
          <dt>Blocked</dt>
          <dd>{MISSION_COUNTS.blockedRecords} waiting record</dd>
        </div>
      </dl>
    </header>
  );
}

function PrototypeState({
  variant,
  selected,
  expanded,
  notice,
}: {
  readonly variant: VariantKey;
  readonly selected: WorkNode;
  readonly expanded: ReadonlySet<string>;
  readonly notice: string;
}) {
  return (
    <output className="met-prototype-state">
      <span>PROTOTYPE STATE</span>
      <strong>
        Variant {variant} · selected {selected.id} · {expanded.size} expanded
      </strong>
      <p>{notice}</p>
    </output>
  );
}

function VariantA({
  allLocations,
  expanded,
  location,
  missionNodes,
  notice,
  onAction,
  onSelect,
  onToggle,
}: VariantProps) {
  return (
    <div className="met-variant met-variant-a">
      <AttentionStrip locations={allLocations} onSelect={onSelect} />
      <section className="met-work-spine">
        <header>
          <div>
            <span>Execution hierarchy</span>
            <strong>Work → session → delegated work → session</strong>
          </div>
          <small>Completed work stays visible at the end.</small>
        </header>
        <TreeBranch
          expanded={expanded}
          nodes={missionNodes}
          onSelect={onSelect}
          onToggle={onToggle}
          selectedId={location.node.id}
        />
      </section>
      <Inspector location={location} onAction={onAction} />
      <PrototypeState
        expanded={expanded}
        notice={notice}
        selected={location.node}
        variant="A"
      />
    </div>
  );
}

interface VariantProps {
  readonly allLocations: readonly NodeLocation[];
  readonly archivedIssueIds: ReadonlySet<string>;
  readonly expanded: ReadonlySet<string>;
  readonly location: NodeLocation;
  readonly missionNodes: readonly WorkNode[];
  readonly missionView: MissionView;
  readonly notice: string;
  readonly onAction: (action: GovernedAction) => void;
  readonly onMissionViewChange: (view: MissionView) => void;
  readonly onSelect: (id: string) => void;
  readonly onToggle: (id: string) => void;
}

function LaneCard({
  location,
  selected,
  onSelect,
}: {
  readonly location: NodeLocation;
  readonly selected: boolean;
  readonly onSelect: (id: string) => void;
}) {
  const { node, parents } = location;
  const activeChildren = node.children.filter(
    (child) => child.status !== "complete",
  );
  return (
    <button
      className="met-lane-card"
      data-selected={selected || undefined}
      onClick={() => onSelect(node.id)}
      type="button"
    >
      <span className="met-lane-card__crumb">
        {parents.length > 0
          ? parents.map((parent) => parent.id).join(" / ")
          : kindLabel(node.kind)}
      </span>
      <span className="met-lane-card__title">
        <NodeGlyph kind={node.kind} />
        <strong>{node.title}</strong>
      </span>
      <StatusPill attention={node.attention} status={node.status} />
      <p>{node.summary}</p>
      <span className="met-lane-card__meta">
        <span>{node.owner}</span>
        <span>{node.updated}</span>
      </span>
      <ProgressMeter value={node.progress} />
      {activeChildren.length > 0 && (
        <span className="met-lane-card__crew">
          {activeChildren.length} direct child
          {activeChildren.length === 1 ? "" : "ren"} ·{" "}
          {activeChildren.map((child) => child.id).join(", ")}
        </span>
      )}
      <span className="met-lane-card__next">{node.nextAction} →</span>
    </button>
  );
}

function VariantB(props: VariantProps) {
  const actionable = props.allLocations.filter(
    ({ node }) => node.attention === "decision" || node.attention === "risk",
  );
  const moving = props.allLocations.filter(
    ({ node }) =>
      node.status === "working" &&
      node.attention !== "risk" &&
      node.attention !== "decision",
  );
  const later = props.allLocations.filter(
    ({ node }) => node.status === "queued" || node.status === "blocked",
  );
  const closed = props.allLocations.filter(
    ({ node }) =>
      node.status === "complete" ||
      (node.status === "failed" && node.attention === null),
  );
  const lanes: ReadonlyArray<{
    readonly title: string;
    readonly summary: string;
    readonly items: readonly NodeLocation[];
  }> = [
    {
      title: "Needs you",
      summary: "Decisions and risks, regardless of depth",
      items: actionable,
    },
    {
      title: "In motion",
      summary: "Working sessions that do not need intervention",
      items: moving,
    },
    {
      title: "Waiting",
      summary: "Queued or blocked without consuming attention",
      items: later,
    },
    {
      title: "Closed",
      summary: "Completed and terminal unsuccessful history",
      items: closed,
    },
  ];

  return (
    <div className="met-variant met-variant-b">
      <section aria-label="Mission state lanes" className="met-lanes">
        {lanes.map((lane) => (
          <section key={lane.title}>
            <header>
              <span>{lane.items.length}</span>
              <div>
                <h3>{lane.title}</h3>
                <p>{lane.summary}</p>
              </div>
            </header>
            <div>
              {lane.items.length > 0 ? (
                lane.items.map((item) => (
                  <LaneCard
                    key={item.node.id}
                    location={item}
                    onSelect={props.onSelect}
                    selected={item.node.id === props.location.node.id}
                  />
                ))
              ) : (
                <p className="met-lane-empty">Nothing in this lane.</p>
              )}
            </div>
          </section>
        ))}
      </section>
      <Inspector location={props.location} onAction={props.onAction} />
      <PrototypeState
        expanded={props.expanded}
        notice={props.notice}
        selected={props.location.node}
        variant="B"
      />
    </div>
  );
}

function ArchiveEmpty() {
  return (
    <section aria-label="Empty Mission archive" className="met-archive-empty">
      <span aria-hidden="true" className="met-archive-empty__icon">
        <svg viewBox="0 0 24 24">
          <path d="M4 7h16v13H4zM3 4h18v3H3zM9 11h6" />
        </svg>
      </span>
      <div>
        <span>Archive</span>
        <h3>No archived tickets</h3>
        <p>
          Completed Issue Slices stay in the Mission outline until you archive
          them. Archiving preserves their evidence, activity, and nested
          session history.
        </p>
      </div>
    </section>
  );
}

function VariantC(props: VariantProps) {
  const outlineNodes = props.missionNodes.filter(
    (node) => !props.archivedIssueIds.has(node.id),
  );
  const archiveNodes = props.missionNodes.filter((node) =>
    props.archivedIssueIds.has(node.id),
  );
  const outlineCounts = countTreeRecords(outlineNodes);
  const archiveCounts = countTreeRecords(archiveNodes);
  const visibleNodes =
    props.missionView === "archive" ? archiveNodes : outlineNodes;
  const visibleIds = new Set(
    flattenNodes(visibleNodes).map(({ node }) => node.id),
  );
  const selectedIsVisible = visibleIds.has(props.location.node.id);

  const changeView = (view: MissionView) => {
    props.onMissionViewChange(view);
    const nextNodes = view === "archive" ? archiveNodes : outlineNodes;
    const nextIds = new Set(
      flattenNodes(nextNodes).map(({ node }) => node.id),
    );
    if (!nextIds.has(props.location.node.id) && nextNodes[0]) {
      props.onSelect(nextNodes[0].id);
    }
  };

  return (
    <div className="met-variant met-variant-c">
      <MissionReadingKey />
      <section className="met-focus-desk">
        <aside
          aria-label={
            props.missionView === "archive"
              ? "Mission archive"
              : "Mission outline"
          }
        >
          <header className="met-outline-header">
            <nav
              aria-label="Mission record views"
              className="met-outline-tabs"
              role="tablist"
            >
              <button
                aria-selected={props.missionView === "outline"}
                data-active={props.missionView === "outline" || undefined}
                onClick={() => changeView("outline")}
                role="tab"
                type="button"
              >
                <NodeGlyph kind="issue" />
                <span>Mission outline</span>
                <strong>{outlineCounts.totalRecords}</strong>
              </button>
              <button
                aria-selected={props.missionView === "archive"}
                data-active={props.missionView === "archive" || undefined}
                onClick={() => changeView("archive")}
                role="tab"
                type="button"
              >
                <span aria-hidden="true" className="met-outline-tab-icon">
                  <svg viewBox="0 0 18 18">
                    <path d="M3 6h12v9H3zM2 3h14v3H2zM7 9h4" />
                  </svg>
                </span>
                <span>Archive</span>
                <strong>{archiveCounts.issueSlices}</strong>
              </button>
            </nav>
            <strong>
              {props.missionView === "archive"
                ? `${archiveCounts.issueSlices} archived ${
                    archiveCounts.issueSlices === 1 ? "ticket" : "tickets"
                  } · ${archiveCounts.totalRecords} preserved ${
                    archiveCounts.totalRecords === 1 ? "record" : "records"
                  }`
                : `${outlineCounts.totalRecords} active outline · ${outlineCounts.issueSlices} tickets · ${outlineCounts.adHoc} ad hoc · ${outlineCounts.sessions} agent sessions`}
            </strong>
          </header>
          {visibleNodes.length > 0 ? (
            <TreeBranch
              compact
              expanded={props.expanded}
              getInlineAction={(node) => {
                if (node.kind !== "issue" || node.status !== "complete") {
                  return undefined;
                }
                return props.archivedIssueIds.has(node.id)
                  ? "Restore to outline"
                  : "Archive ticket";
              }}
              nodes={visibleNodes}
              onInlineAction={(nodeId, action) => {
                props.onSelect(nodeId);
                props.onAction(action);
              }}
              onSelect={props.onSelect}
              onToggle={props.onToggle}
              selectedId={props.location.node.id}
            />
          ) : (
            <div className="met-outline-empty">
              <strong>Archive is empty</strong>
              <span>Archive a completed ticket directly from its outline row.</span>
            </div>
          )}
        </aside>
        <div className="met-focus-inspector">
          {selectedIsVisible ? (
            <>
              <div className="met-focus-banner">
                <span
                  data-attention={
                    props.missionView === "archive"
                      ? "none"
                      : (props.location.node.attention ?? "none")
                  }
                >
                  {props.missionView === "archive"
                    ? "Archived"
                    : props.location.node.attention
                      ? attentionLabel(props.location.node.attention)
                      : "Inspecting"}
                </span>
                <p>
                  {props.missionView === "archive"
                    ? "Hidden from the active outline · history preserved"
                    : props.location.node.nextAction}
                </p>
              </div>
              {props.location.node.id === "ISS-53" ? (
                <SupervisionReviewPrototype />
              ) : (
                <Inspector location={props.location} onAction={props.onAction} />
              )}
            </>
          ) : (
            <ArchiveEmpty />
          )}
        </div>
      </section>
      <PrototypeState
        expanded={props.expanded}
        notice={props.notice}
        selected={props.location.node}
        variant="C"
      />
    </div>
  );
}

function MissionReadingKey() {
  return (
    <section aria-label="Mission outline key" className="met-reading-key">
      <div className="met-reading-key__types">
        <span>
          <NodeGlyph kind="issue" />
          <span>
            <strong>Issue Slice</strong>
            <small>A tracked Mission ticket.</small>
          </span>
        </span>
        <span>
          <NodeGlyph kind="ad-hoc" />
          <span>
            <strong>Ad Hoc Delegation</strong>
            <small>A bounded extra task, not a new ticket.</small>
          </span>
        </span>
        <span>
          <NodeGlyph kind="session" />
          <span>
            <strong>Local Agent session</strong>
            <small>One agent attempt on a work record.</small>
          </span>
        </span>
      </div>
      <div className="met-reading-key__states">
        <span data-key="working">Working</span>
        <span data-key="healthy">Complete</span>
        <span data-key="attention">
          Decision / waiting / repair / blocked
        </span>
        <span data-key="danger">Failed / risk</span>
        <span data-key="queued">Queued</span>
      </div>
    </section>
  );
}

function ActionPreview({
  action,
  blockerActionApproved,
  node,
  onClose,
  onCommit,
  onOpenRecommendedWork,
}: {
  readonly action: GovernedAction;
  readonly blockerActionApproved: boolean;
  readonly node: WorkNode;
  readonly onClose: () => void;
  readonly onCommit: (outcome: string) => void;
  readonly onOpenRecommendedWork: () => void;
}) {
  const [reason, setReason] = useState("");
  const reasonInput =
    action === "Cancel session"
      ? {
          help:
            "Saved with the terminal cancellation and its audit receipt. This is not a prompt for the Local Agent.",
          label: "Cancellation note",
          placeholder: "Why should this session stop?",
        }
      : action === "Retry session"
        ? {
            help:
              "Passed to the fresh retry so it knows what the previous attempt must change.",
            label: "Retry instruction",
            placeholder: "What should the next attempt do differently?",
          }
        : null;
  const commit = (outcome: string) => {
    if (reasonInput && reason.trim().length < 3) return;
    onCommit(outcome);
  };
  const reviewInstruction =
    node.details.evidence
      .find((item) => item.startsWith("Review reason:"))
      ?.replace("Review reason:", "")
      .trim() ?? node.details.latest;
  const archivedRecordCount = flattenNodes([node]).length;

  return (
    <div
      aria-labelledby="met-action-title"
      className="met-action-backdrop"
      onMouseDown={(event) => {
        if (event.currentTarget === event.target) onClose();
      }}
      role="dialog"
    >
      <section className="met-action-dialog">
        <header>
          <div>
            <span>PROTOTYPE · {action.toUpperCase()} · NO BACKEND EFFECT</span>
            <h3 id="met-action-title">{node.title}</h3>
            <p>
              <strong>{node.id}</strong>
              <span>{node.owner}</span>
            </p>
          </div>
          <button aria-label="Close action preview" onClick={onClose} type="button">
            ×
          </button>
        </header>

        {action === "Review evidence" && (
          <>
            <div className="met-action-evidence">
              <span>Evidence Package</span>
              {node.details.evidence.map((item) => (
                <strong key={item}>✓ {item}</strong>
              ))}
              <code>{node.details.command}</code>
            </div>
            <p className="met-action-note">
              A real decision would carry the current expected revision and
              create a visible Mission Commander / Orchestrator receipt.
            </p>
            <section
              aria-label="Review outcome meanings"
              className="met-review-outcomes"
            >
              <article>
                <strong>Accept evidence</strong>
                <p>
                  Marks an Issue Slice Complete and PR-ready, but does not
                  merge it. For Ad Hoc work, it completes that bounded work.
                </p>
              </article>
              <article>
                <strong>Request repair</strong>
                <p>
                  Records what is insufficient and exposes one separate
                  <em> Launch repair</em> action. It creates no ticket and
                  starts no agent yet.
                </p>
              </article>
              <article>
                <strong>Ask for human review</strong>
                <p>
                  Pauses the work for a person to inspect. It creates no
                  ticket and starts no agent.
                </p>
              </article>
            </section>
          </>
        )}

        {action === "Launch repair" && (
          <section className="met-action-packet">
            <header>
              <span>Repair task packet</span>
              <strong>Sent to one child Local Agent session</strong>
            </header>
            <dl>
              <div>
                <dt>Assigned agent</dt>
                <dd>{node.owner}</dd>
              </div>
              <div>
                <dt>Repair instruction</dt>
                <dd>{reviewInstruction}</dd>
              </div>
              <div>
                <dt>Original task</dt>
                <dd>{node.details.task}</dd>
              </div>
              <div>
                <dt>Prior evidence</dt>
                <dd>{node.details.evidence.join(" · ")}</dd>
              </div>
              <div>
                <dt>Allowed files</dt>
                <dd>
                  {node.details.files.length > 0
                    ? node.details.files.join(" · ")
                    : "No file paths registered"}
                </dd>
              </div>
              <div>
                <dt>Command policy</dt>
                <dd>
                  <code>{node.details.command}</code>
                </dd>
              </div>
            </dl>
            <p>
              These fields—not a new free-form reason—form the inherited repair
              task packet. No new Issue Slice or Ad Hoc Delegation is created.
            </p>
          </section>
        )}

        {action === "Inspect blocker" && (
          <section className="met-action-packet" data-tone="attention">
            <header>
              <span>
                {blockerActionApproved
                  ? "Approved blocker action"
                  : "Model-recommended blocker action"}
              </span>
              <strong>
                {blockerActionApproved
                  ? `${BLOCKER_RECOMMENDATION.outputId} created and linked`
                  : BLOCKER_RECOMMENDATION.action}
              </strong>
            </header>
            {blockerActionApproved ? (
              <>
                <dl>
                  <div>
                    <dt>Linked blocking work</dt>
                    <dd>
                      {BLOCKER_RECOMMENDATION.outputId} · Choose Alfredo&apos;s
                      backend modernization architecture
                    </dd>
                  </div>
                  <div>
                    <dt>Current state</dt>
                    <dd>Queued · no Local Agent session started</dd>
                  </div>
                  <div>
                    <dt>Effect on {node.id}</dt>
                    <dd>
                      Still Blocked until{" "}
                      {BLOCKER_RECOMMENDATION.outputId} completes
                    </dd>
                  </div>
                  <div>
                    <dt>Next governed action</dt>
                    <dd>Open the new Issue Slice, review its brief, then launch</dd>
                  </div>
                </dl>
                <p>
                  Approval created the recommended work and dependency edge; it
                  did not claim that the blocker was already resolved.
                </p>
              </>
            ) : (
              <>
                <dl>
                  <div>
                    <dt>Recommendation</dt>
                    <dd>{BLOCKER_RECOMMENDATION.action}</dd>
                  </div>
                  <div>
                    <dt>Why this action</dt>
                    <dd>{BLOCKER_RECOMMENDATION.rationale}</dd>
                  </div>
                  <div>
                    <dt>On human approval</dt>
                    <dd>
                      Create {BLOCKER_RECOMMENDATION.outputId}, assign{" "}
                      {BLOCKER_RECOMMENDATION.assignedTo}, and link it as this
                      blocker
                    </dd>
                  </div>
                  <div>
                    <dt>Execution effect</dt>
                    <dd>No Local Agent session starts from this recommendation</dd>
                  </div>
                  <div>
                    <dt>Proposed acceptance</dt>
                    <dd>{BLOCKER_RECOMMENDATION.acceptance}</dd>
                  </div>
                  <div>
                    <dt>Routing rule</dt>
                    <dd>
                      Missing tracked work → create an Issue Slice. Existing
                      tracked work → recommend a Local Agent session.
                    </dd>
                  </div>
                </dl>
                <p>
                  {node.id} remains Blocked after approval. It becomes
                  launchable only when the resulting blocking work satisfies
                  its accepted completion condition.
                </p>
              </>
            )}
          </section>
        )}

        {(action === "Archive ticket" || action === "Restore to outline") && (
          <section className="met-action-packet">
            <header>
              <span>Mission outline organization</span>
              <strong>
                {action === "Archive ticket"
                  ? "Move completed history out of the active outline"
                  : "Return completed history to the active outline"}
              </strong>
            </header>
            <dl>
              <div>
                <dt>Completed Issue Slice</dt>
                <dd>
                  {node.id} · {node.title}
                </dd>
              </div>
              <div>
                <dt>Records moved together</dt>
                <dd>
                  {archivedRecordCount}{" "}
                  {archivedRecordCount === 1 ? "record" : "records"}, including
                  nested work and sessions
                </dd>
              </div>
              <div>
                <dt>Preserved</dt>
                <dd>Evidence, activity, task details, and session history</dd>
              </div>
              <div>
                <dt>Unchanged</dt>
                <dd>
                  Completion state, GitHub ticket, and Mission authority
                </dd>
              </div>
            </dl>
            <p>
              This prototype treats archive as reversible Mission Work
              organization. It does not delete or close anything.
            </p>
          </section>
        )}

        {(action === "View evidence" || action === "Retry session") && (
          <div className="met-action-evidence">
            <span>{action === "View evidence" ? "Registered evidence" : "Retry boundary"}</span>
            {node.details.evidence.map((item) => (
              <strong key={item}>• {item}</strong>
            ))}
          </div>
        )}

        {action === "Cancel session" && (
          <p className="met-action-note">
            Cancellation preserves this session as terminal unsuccessful work
            and asks the runner to stop. The required note records why the
            Mission Commander ended it.
          </p>
        )}

        {reasonInput && (
          <label className="met-action-reason">
            <strong>{reasonInput.label} · required</strong>
            <span>{reasonInput.help}</span>
            <textarea
              autoFocus
              onChange={(event) => setReason(event.target.value)}
              placeholder={reasonInput.placeholder}
              value={reason}
            />
          </label>
        )}

        <footer>
          <button onClick={onClose} type="button">
            Close
          </button>
          {action === "Review evidence" && (
            <>
              <button
                onClick={() =>
                  commit(
                    "Human review request previewed; work would pause and no ticket or agent would be created",
                  )
                }
                type="button"
              >
                Ask for human review
              </button>
              <button
                onClick={() =>
                  commit(
                    "Repair request previewed; no ticket or session would be created until a separate Launch repair action",
                  )
                }
                type="button"
              >
                Request repair
              </button>
              <button
                className="met-action-primary"
                onClick={() =>
                  commit(
                    "Evidence acceptance previewed; the Issue Slice would become Complete and PR-ready, not merged",
                  )
                }
                type="button"
              >
                Accept evidence
              </button>
            </>
          )}
          {action === "Launch repair" && (
            <button
              className="met-action-primary"
              onClick={() =>
                commit(
                  "Repair launch previewed; exactly one child session would receive the displayed inherited repair task packet",
                )
              }
              type="button"
            >
              Preview launch
            </button>
          )}
          {action === "Inspect blocker" &&
            (blockerActionApproved ? (
              <button
                className="met-action-primary"
                onClick={onOpenRecommendedWork}
                type="button"
              >
                Open {BLOCKER_RECOMMENDATION.outputId}
              </button>
            ) : (
              <button
                className="met-action-primary"
                onClick={() =>
                  commit(
                    `Blocker recommendation approved; ${BLOCKER_RECOMMENDATION.outputId} would be created and linked while ${node.id} remains Blocked`,
                  )
                }
                type="button"
              >
                Approve &amp; create {BLOCKER_RECOMMENDATION.outputId}
              </button>
            ))}
          {action === "Cancel session" && (
            <button
              className="met-action-danger"
              disabled={reason.trim().length < 3}
              onClick={() =>
                commit(
                  "Cancellation previewed; the terminal session would retain the Mission Commander cancellation note",
                )
              }
              type="button"
            >
              Preview cancellation
            </button>
          )}
          {action === "Retry session" && (
            <button
              className="met-action-primary"
              disabled={reason.trim().length < 3}
              onClick={() => commit("Retry previewed")}
              type="button"
            >
              Preview retry
            </button>
          )}
          {action === "Archive ticket" && (
            <button
              onClick={() =>
                commit(
                  "Completed Issue Slice archived from the active outline with all history preserved",
                )
              }
              type="button"
            >
              Archive from outline
            </button>
          )}
          {action === "Restore to outline" && (
            <button
              onClick={() =>
                commit(
                  "Completed Issue Slice restored to the active outline with all history preserved",
                )
              }
              type="button"
            >
              Restore to outline
            </button>
          )}
        </footer>
      </section>
    </div>
  );
}

function PrototypeSwitcher({
  current,
  onChange,
}: {
  readonly current: VariantKey;
  readonly onChange: (variant: VariantKey) => void;
}) {
  const index = VARIANTS.findIndex((variant) => variant.key === current);
  const move = (offset: number) => {
    onChange(VARIANTS[(index + offset + VARIANTS.length) % VARIANTS.length].key);
  };
  return (
    <nav aria-label="Prototype variants" className="met-switcher">
      <button aria-label="Previous variant" onClick={() => move(-1)} type="button">
        ←
      </button>
      <span>
        <small>VARIANT</small>
        <strong>
          {VARIANTS[index].key} · {VARIANTS[index].name}
        </strong>
      </span>
      <button aria-label="Next variant" onClick={() => move(1)} type="button">
        →
      </button>
    </nav>
  );
}

function WorkstationChrome({
  actionPanel,
  children,
  variant,
}: {
  readonly actionPanel?: ReactNode;
  readonly children: ReactNode;
  readonly variant: VariantKey;
}) {
  return (
    <div className="met-prototype" data-variant={variant}>
      <header className="met-topbar">
        <div className="met-brand">
          <span aria-hidden="true">A</span>
          <div>
            <strong>ALFREDO</strong>
            <small>throwaway Mission Execution Tree prototype</small>
          </div>
        </div>
        <div className="met-context">
          <span>CODING WORKSPACE</span>
          <strong>EricleungDK / Alfredo</strong>
        </div>
        <div className="met-context">
          <span>CONTEXT</span>
          <strong>Modernization Mission</strong>
        </div>
      </header>
      <main className="met-workstation">
        <ConsoleContext />
        <section aria-label="Mission Work" className="met-mission-work">
          <MissionHeader />
          <div className="met-mission-stage">
            {children}
            {actionPanel}
          </div>
        </section>
      </main>
    </div>
  );
}

export function MissionExecutionTreePrototype() {
  const [variant, setVariant] = useState<VariantKey>(variantFromLocation);
  const [supervisionReview, setSupervisionReview] = useState(
    supervisionReviewFromLocation,
  );
  const [missionView, setMissionView] = useState<MissionView>("outline");
  const [archivedIssueIds, setArchivedIssueIds] = useState<
    ReadonlySet<string>
  >(() => new Set());
  const [approvedBlockerRecommendations, setApprovedBlockerRecommendations] =
    useState<ReadonlySet<string>>(() => new Set());
  const [selectedId, setSelectedId] = useState(() =>
    variantFromLocation() === "C" ? "ISS-53" : "ISS-46",
  );
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(
    () =>
      new Set([
        "ISS-53",
        "ISS-46",
        "session-46-codex",
        "ISS-48",
        "session-48-qwen",
        "ADHOC-21",
      ]),
  );
  const [activeAction, setActiveAction] = useState<GovernedAction | null>(null);
  const [notice, setNotice] = useState(
    "No prototype action previewed. Canonical Mission state is unchanged.",
  );
  const blockerRecommendationApproved = approvedBlockerRecommendations.has(
    BLOCKER_RECOMMENDATION.blockedIssueId,
  );
  const missionNodes = useMemo(
    () => missionNodesWithApprovedBlocker(blockerRecommendationApproved),
    [blockerRecommendationApproved],
  );
  const allLocations = useMemo(() => flattenNodes(missionNodes), [missionNodes]);
  const locationById = useMemo(
    () =>
      new Map(
        allLocations.map((candidate) => [candidate.node.id, candidate]),
      ),
    [allLocations],
  );
  const location = locationById.get(selectedId) ?? allLocations[0];

  const setVariantInUrl = (nextVariant: VariantKey) => {
    const params = new URLSearchParams(window.location.search);
    params.set("variant", nextVariant);
    window.history.replaceState(null, "", `${window.location.pathname}?${params}`);
    setVariant(nextVariant);
    if (nextVariant === "C") setSelectedId("ISS-53");
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (
        target?.matches("input, textarea, select, button, [contenteditable='true']")
      ) {
        return;
      }
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      const currentIndex = VARIANTS.findIndex((item) => item.key === variant);
      const offset = event.key === "ArrowRight" ? 1 : -1;
      const next =
        VARIANTS[
          (currentIndex + offset + VARIANTS.length) % VARIANTS.length
        ].key;
      setVariantInUrl(next);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [variant]);

  useEffect(() => {
    const onPopState = () => {
      setVariant(variantFromLocation());
      setSupervisionReview(supervisionReviewFromLocation());
    };
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  const props = useMemo<VariantProps>(
    () => ({
      allLocations,
      archivedIssueIds,
      expanded,
      location,
      missionNodes,
      missionView,
      notice,
      onAction: setActiveAction,
      onMissionViewChange: setMissionView,
      onSelect: setSelectedId,
      onToggle: (id: string) => {
        setExpanded((current) => {
          const next = new Set(current);
          if (next.has(id)) next.delete(id);
          else next.add(id);
          return next;
        });
      },
    }),
    [
      allLocations,
      archivedIssueIds,
      expanded,
      location,
      missionNodes,
      missionView,
      notice,
    ],
  );

  return (
    <>
      <WorkstationChrome
        actionPanel={
          activeAction ? (
            <ActionPreview
              action={activeAction}
              blockerActionApproved={blockerRecommendationApproved}
              node={location.node}
              onClose={() => setActiveAction(null)}
              onOpenRecommendedWork={() => {
                setSelectedId(BLOCKER_RECOMMENDATION.outputId);
                setMissionView("outline");
                setActiveAction(null);
              }}
              onCommit={(outcome) => {
                if (activeAction === "Archive ticket") {
                  setArchivedIssueIds((current) => {
                    const next = new Set(current);
                    next.add(location.node.id);
                    return next;
                  });
                  setMissionView("archive");
                }
                if (activeAction === "Restore to outline") {
                  setArchivedIssueIds((current) => {
                    const next = new Set(current);
                    next.delete(location.node.id);
                    return next;
                  });
                  setMissionView("outline");
                }
                if (
                  activeAction === "Inspect blocker" &&
                  location.node.id === BLOCKER_RECOMMENDATION.blockedIssueId
                ) {
                  setApprovedBlockerRecommendations((current) => {
                    const next = new Set(current);
                    next.add(location.node.id);
                    return next;
                  });
                  setSelectedId(BLOCKER_RECOMMENDATION.outputId);
                  setMissionView("outline");
                }
                setNotice(
                  `${outcome} for ${location.node.id}. Simulation only; canonical Mission state is unchanged.`,
                );
                setActiveAction(null);
              }}
            />
          ) : undefined
        }
        variant={variant}
      >
        {variant === "A" && <VariantA {...props} />}
        {variant === "B" && <VariantB {...props} />}
        {variant === "C" && <VariantC {...props} />}
      </WorkstationChrome>
      {!supervisionReview ? (
        <PrototypeSwitcher current={variant} onChange={setVariantInUrl} />
      ) : null}
    </>
  );
}
