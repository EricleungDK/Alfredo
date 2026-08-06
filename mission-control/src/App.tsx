import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactElement,
  type RefObject,
} from "react";
import type {
  AdHocDelegationProposalRequest,
  AlfredoLaunchContext,
  AgentCapability,
  AgentCapabilityCatalog,
  AgentConsoleMessage,
  ActivityJournalFilters,
  ActivityJournalProjection,
  CodingWorkspaceAcknowledgement,
  CodingWorkspaceSelectionRequest,
  ConversationScope,
  MissionChoiceOption,
  PathAccessLevel,
  ShellTerminalClassification,
  ShellTerminalCommandRecord,
  ShellTerminalCommandStatus,
  MissionDraftCreateRequest,
  MissionDraftDecision,
  MissionDraftProjection,
  ReviewDecision,
  ReviewWorkspaceProjection,
  SessionArtifactProjection,
  SessionArtifactReadRequest,
  WayfinderProjection,
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
import { MissionExecutionTree, type MissionExecutionOutputState } from "./MissionExecutionTree";
import {
  afterTwoAnimationFrames,
  markFrontendPerformance,
  markNativePerformance,
} from "./performance-measurement";
import { applyWorkspaceUpdates } from "./workspace-sync";
import {
  projectIssueAssignmentBoard,
  projectMissionExecutionTree,
  workstationActionKey,
  workstationActionStateId,
  workstationActionTargetId,
  workstationReviewActionStateId,
  type IssueAssignmentBoardProjection,
  type IssueAssignmentBoardRow,
  type WorkstationCardProjection,
  type WorkstationDiffLink,
  type WorkstationEvidenceLink,
  type WorkstationGovernedAction,
  type MissionExecutionTreeProjection,
} from "./workstation-projection";
import { ShellTerminalPanel } from "./ShellTerminalPanel";
import {
  useShellTerminal,
  type ContextualPathGrantRequest,
  type ShellTerminalController,
  type ShellTerminalTranscriptEntry,
} from "./use-shell-terminal";
import "./styles.css";

const AGENT_CONSOLE_USER_CONTENT_CHARACTER_LIMIT = 16_000;

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
  readonly receiptCorrelationId?: string;
  readonly receiptPhase?: string;
}

interface FailedWorkstationActionContinuityState {
  readonly schema_version: 1;
  readonly turns: readonly WorkstationActionTurn[];
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

interface SessionArtifactViewerTarget {
  readonly request: SessionArtifactReadRequest;
  readonly label: string;
  readonly focusPath?: string;
  readonly returnFocus?: HTMLElement | null;
}

interface SessionArtifactViewerState {
  readonly target: SessionArtifactViewerTarget;
  readonly status: "loading" | "ready" | "error";
  readonly artifact: SessionArtifactProjection | null;
  readonly message: string;
  readonly recoverable: boolean;
}

interface AppProps {
  readonly client: WorkspaceClient;
  readonly syncIntervalMs?: number;
}

interface WorkstationContinuityState {
  readonly schema_version: 1;
  readonly commandAuditOpen: boolean;
  readonly selectedIssueId: string | null;
  readonly selectedIssueMissionId: string | null;
  readonly issueFocusTarget: "assignment-board" | "mission-board" | null;
  readonly selectedSessionId: string | null;
  readonly selectedSessionMissionId: string | null;
  readonly selectedWorkstationDiff: WorkstationDiffLink | null;
  readonly selectedExecutionNodeId: string | null;
}

type PersistedWorkstationContinuityState = Partial<
  Omit<WorkstationContinuityState, "issueFocusTarget">
> & {
  readonly issueFocusTarget?: unknown;
  readonly leftLaneMode?: "agent" | "terminal";
};

const WORKSTATION_CONTINUITY_SCHEMA_VERSION = 1;
const FAILED_WORKSTATION_ACTION_CONTINUITY_SCHEMA_VERSION = 1;
const FAILED_WORKSTATION_ACTION_TURN_LIMIT = 100;
const FAILED_WORKSTATION_ACTION_CONTENT_LIMIT = 4_000;
const SESSION_OUTPUT_RENDER_CONTENT_BYTES_LIMIT = 128_000;
const sessionOutputTextEncoder = new TextEncoder();

function parseSlashCommand(content: string): { name: string; argument: string } | null {
  const match = content.trim().match(/^(\/[a-z-]+)(?:\s+([\s\S]*))?$/i);
  if (!match) return null;
  return { name: match[1].toLowerCase(), argument: (match[2] ?? "").trim() };
}

type PromptCompletionKind = "command" | "capability";

interface PromptCompletion {
  readonly value: string;
  readonly detail: string;
  readonly kind: PromptCompletionKind;
  readonly start: number;
}

interface PromptCompletionQuery {
  readonly kind: PromptCompletionKind;
  readonly start: number;
  readonly value: string;
}

const CORE_CAPABILITY_COMPLETIONS: readonly Omit<PromptCompletion, "start">[] = [
  { value: "@workspace", detail: "Coding Workspace context", kind: "capability" },
  { value: "@wayfinder", detail: "Wayfinder planning capability", kind: "capability" },
  { value: "@orchestrator", detail: "Orchestrator authority and execution", kind: "capability" },
  { value: "@mission", detail: "Active Mission formation", kind: "capability" },
];

function promptCompletionQuery(draft: string): PromptCompletionQuery | null {
  const commandMatch = draft.match(/^\/[^\s]*$/);
  if (commandMatch) {
    return { kind: "command", start: 0, value: commandMatch[0] };
  }
  const capabilityMatch = draft.match(/(?:^|\s)(@[^\s]*)$/);
  const capability = capabilityMatch?.[1];
  if (!capability || !capabilityMatch) return null;
  return {
    kind: "capability",
    start: draft.length - capability.length,
    value: capability,
  };
}

function capabilityCompletionOptions(
  query: PromptCompletionQuery | null,
  catalog: AgentCapabilityCatalog,
): readonly PromptCompletion[] {
  if (!query) return [];
  const candidates: readonly Omit<PromptCompletion, "start">[] =
    query.kind === "command"
      ? catalog.commands.map((command) => ({
          value: command.name,
          detail: command.description,
          kind: "command" as const,
        }))
      : [
          ...CORE_CAPABILITY_COMPLETIONS,
          ...catalog.agents.map((agent) => ({
            value: `@${agent.id}`,
            detail: `${agent.role} · ${agent.model || agent.runner}`,
            kind: "capability" as const,
          })),
          ...catalog.skills.map((skill) => ({
            value: `@${skill.name}`,
            detail: skill.description,
            kind: "capability" as const,
          })),
        ];
  const matches = candidates
    .filter((candidate) => candidate.value.toLowerCase().startsWith(query.value.toLowerCase()))
    .map((candidate) => ({ ...candidate, start: query.start }));
  if (
    matches.length === 1 &&
    matches[0].value.toLowerCase() === query.value.toLowerCase()
  ) {
    return [];
  }
  return matches;
}

function canNavigatePromptHistory(
  direction: -1 | 1,
  historyIndex: number | null,
  draft: string,
  selectionStart: number,
  selectionEnd: number,
): boolean {
  if (historyIndex !== null) return true;
  if (direction === 1) return false;
  if (!draft.includes("\n")) return true;
  return selectionStart === 0 && selectionEnd === 0;
}

type CapabilityBoundary =
  | "Coding Workspace"
  | "Wayfinder"
  | "Orchestrator"
  | "Mission"
  | "Frontier Model"
  | "Local Agent"
  | "Unattributed";

function capabilityBoundaryLabel(
  source: string,
  scope: ConversationScope,
): CapabilityBoundary {
  switch (source.trim().toLowerCase()) {
    case "wayfinder":
    case "wayfinder-agent":
      return "Wayfinder";
    case "orchestrator":
      return "Orchestrator";
    case "frontier-model":
      return "Frontier Model";
    case "local-agent":
      return "Local Agent";
    case "agent-console":
    case "mission-commander":
      return scope.kind === "working-directory" ? "Coding Workspace" : "Mission";
    default:
      return "Unattributed";
  }
}

function missionSessionIdentity(missionId: string, sessionId: string): string {
  return JSON.stringify([missionId, sessionId]);
}

interface CodingTaskIntent {
  readonly request: string;
  readonly skillName: string | null;
  readonly acceptanceCriteria: readonly string[];
}

function parseCodingTaskIntent(content: string): CodingTaskIntent | null {
  const slashCommand = parseSlashCommand(content);
  if (slashCommand?.name === "/task" && slashCommand.argument) {
    return {
      request: slashCommand.argument,
      skillName: null,
      acceptanceCriteria: [slashCommand.argument],
    };
  }
  if (slashCommand?.name === "/use" && slashCommand.argument) {
    const skillRequest = slashCommand.argument.match(/^(\S+)\s+([\s\S]+)$/);
    if (!skillRequest?.[2].trim()) return null;
    const skillName = skillRequest[1];
    const request = skillRequest[2].trim();
    return {
      request,
      skillName,
      acceptanceCriteria: [
        `/use ${skillName} ${request}`,
      ],
    };
  }

  const request = content.trim();
  const delegatedCodingVerb =
    "(?:fix|implement|build|add|create|update|modify|change|refactor|repair|debug|write|develop|remove|migrate|optimize|upgrade|integrate|replace|make|handle|resolve|address|patch|troubleshoot|test|investigate|inspect|check|work\\s+on|take\\s+care\\s+of)";
  const delegatedCodingRequest = new RegExp(
      `^(?:(?:can|could|would|will)\\s+you\\s+)?(?:please\\s+)?` +
      `(?:ask|have|get|tell|use|call|spin\\s+up|delegate(?:\\s+to)?|send)\\s+` +
      `(?:an?\\s+|the\\s+)?(?:subagent|sub-agent|local\\s+agent|coding\\s+agent)` +
      `(?:\\s+to)?\\s+${delegatedCodingVerb}\\b`,
    "i",
  );
  const explicitRemediationRequest =
    /^(?:(?:can|could|would|will)\s+you\s+)?(?:please\s+)?(?:fix|repair|implement|refactor|patch|update)\s+(?:(?:the|this|these|a|an)\s+)?(?:(?:failing|failed|broken|buggy|regressed)\s+)?(?:tests?|build|bug|error|issue|code|layout|polling|feature|function|component|module|api|ui|frontend|backend)\b/i;
  const explicitDelegationSuffix = new RegExp(
    `^(?:(?:can|could|would|will)\\s+you\\s+)?(?:please\\s+)?${delegatedCodingVerb}\\b` +
      `[\\s\\S]*\\bwith\\s+a\\s+subagent\\s*[.!?]*\\s*$`,
    "i",
  );
  if (
    !delegatedCodingRequest.test(request) &&
    !explicitRemediationRequest.test(request) &&
    !explicitDelegationSuffix.test(request)
  ) return null;
  return { request, skillName: null, acceptanceCriteria: [request] };
}

function isUnknownRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sameStringList(value: unknown, expected: readonly string[]): boolean {
  return Array.isArray(value) &&
    value.length === expected.length &&
    value.every((item, index) => item === expected[index]);
}

function sameStringRecord(
  value: unknown,
  expected: Readonly<Record<string, string>>,
): boolean {
  if (!isUnknownRecord(value)) return false;
  const entries = Object.entries(expected);
  return Object.keys(value).length === entries.length &&
    entries.every(([key, expectedValue]) => value[key] === expectedValue);
}

export function isExactAdHocDelegationBoundary(
  item: WorkspaceQueueItem,
  request: AdHocDelegationProposalRequest,
  authorizedGoal: string,
): boolean {
  const changes = item.proposed_changes;
  const scope = changes.scope;
  if (!isUnknownRecord(scope)) return false;
  const scopeMissionMatches = request.scope_kind === "working-directory"
    ? scope.mission_id == null
    : scope.mission_id === request.mission_id;
  return item.item_type === "ad-hoc-delegation" &&
    item.status === "pending" &&
    item.source === request.source &&
    typeof request.mission_id === "string" &&
    request.mission_id.length > 0 &&
    item.mission_id === request.mission_id &&
    scope.kind === request.scope_kind &&
    scope.target_id === request.scope_target &&
    scope.label === request.scope_label &&
    scopeMissionMatches &&
    changes.goal === authorizedGoal &&
    sameStringList(changes.acceptance_criteria, request.acceptance_criteria) &&
    sameStringList(changes.allowed_paths, request.allowed_paths) &&
    sameStringRecord(changes.command_policy, request.command_policy) &&
    changes.proposed_agent === request.proposed_agent &&
    changes.originating_message_id === request.originating_message_id;
}

function isEligibleControllerCapability(agent: AgentCapability): boolean {
  const routing = agent.routing.toLowerCase();
  const role = agent.role.toLowerCase();
  const controllerRole = routing
    ? ["controller", "router", "frontier"].includes(routing)
    : role === "frontier";
  return controllerRole &&
    hasLocalExecutionBoundary(agent) &&
    agent.delegate_only === false &&
    agent.requires_approval === false &&
    !agent.model.toLowerCase().endsWith(":cloud");
}

const LOCAL_EXECUTION_PROVIDERS = new Set([
  "command",
  "fake",
  "local",
  "ollama",
  "test",
  "test-harness",
]);
const LOCAL_EXECUTION_RUNNERS = new Set(["command", "fake", "ollama"]);

function hasLocalExecutionBoundary(agent: AgentCapability): boolean {
  return LOCAL_EXECUTION_PROVIDERS.has(agent.provider.trim().toLowerCase()) &&
    LOCAL_EXECUTION_RUNNERS.has(agent.runner.trim().toLowerCase());
}

function isEligibleWorkerCapability(agent: AgentCapability): boolean {
  const routing = agent.routing.toLowerCase();
  const role = agent.role.toLowerCase();
  return agent.availability === "available" &&
    hasLocalExecutionBoundary(agent) &&
    agent.assignable === true &&
    agent.delegate_only === false &&
    agent.requires_approval === false &&
    !agent.model.toLowerCase().endsWith(":cloud") &&
    role !== "frontier" &&
    routing !== "controller" &&
    routing !== "router" &&
    routing !== "frontier" &&
    (routing === "worker" || role === "local-agent");
}

function eligibleControllerId(
  catalog: AgentCapabilityCatalog,
  requestedIdOrModel: string,
): string {
  if (!requestedIdOrModel) return "";
  return catalog.agents.find(
    (agent) =>
      (agent.id === requestedIdOrModel || agent.model === requestedIdOrModel) &&
      agent.availability === "available" &&
      isEligibleControllerCapability(agent),
  )?.id ?? "";
}

function preferredEligibleControllerId(
  catalog: AgentCapabilityCatalog,
  preferredIdsOrModels: readonly string[],
): string {
  for (const requested of preferredIdsOrModels) {
    const eligible = eligibleControllerId(catalog, requested);
    if (eligible) return eligible;
  }
  const defaultController = eligibleControllerId(catalog, catalog.default_agent_id);
  if (defaultController) return defaultController;
  return catalog.agents.find(
    (agent) => agent.availability === "available" && isEligibleControllerCapability(agent),
  )?.id ?? "";
}

function sameCanonicalWorkspace(
  current: Extract<WorkspaceLoadResult, { kind: "ready" | "empty" }>,
  candidate: Extract<WorkspaceLoadResult, { kind: "ready" | "empty" }>,
): boolean {
  return current.kind === candidate.kind &&
    JSON.stringify(current.snapshot) === JSON.stringify(candidate.snapshot);
}

function canonicalSessionIsQueued(
  state: WorkspaceLoadResult | "loading",
  missionId: string,
  sessionId: string,
): boolean {
  if (state === "loading" || (state.kind !== "ready" && state.kind !== "empty")) return false;
  const missions = state.snapshot.missions ?? [];
  const mission = missionId
    ? missions.find((candidate) => candidate.id === missionId)
    : missions.find((candidate) =>
        candidate.sessions.some((session) => session.session_id === sessionId),
      );
  return mission?.sessions.some(
    (session) =>
      session.session_id === sessionId && session.status.toLowerCase() === "queued",
  ) ?? false;
}

function acknowledgementMatchesWorkspaceSelection(
  acknowledgement: CodingWorkspaceAcknowledgement,
  request: CodingWorkspaceSelectionRequest,
  startingLocation: string,
): boolean {
  return acknowledgement.schema_version === 1 &&
    acknowledgement.outcome === "acknowledged" &&
    acknowledgement.correlation_id === request.correlation_id &&
    acknowledgement.starting_location === startingLocation &&
    acknowledgement.selection_mode === request.selection_mode &&
    acknowledgement.active_mission === null;
}

function missionIdFromTitle(title: string): string {
  return title
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}

function workspaceSelectionFailureMessage(
  code: string,
  message: string,
  suggestedWorkspacePath: string | null | undefined,
): string {
  const detail = `${code}: ${message}`;
  if (code !== "workspace-unsafe") return detail;
  if (suggestedWorkspacePath) {
    return `${detail} For a new repository, use ${suggestedWorkspacePath}; for an existing repository, enter its exact path. Do not select Alfredo's install, backend, or runtime.`;
  }
  return `${detail} No safe new repository path is configured. Choose an existing repository outside Alfredo's install, backend, or runtime roots, or relaunch with a separate Starting Location.`;
}

export function App({ client, syncIntervalMs = 1000 }: AppProps) {
  const [state, setState] = useState<WorkspaceLoadResult | "loading">("loading");
  const [connectionStatus, setConnectionStatus] = useState<
    "connected" | "offline" | "reconnecting"
  >("reconnecting");
  const [actionStatus, setActionStatus] = useState<
    "pending" | "acknowledged" | "stale" | "rejected" | null
  >(null);
  const [actionFailure, setActionFailure] = useState<string | null>(null);
  const [consoleHistory, setConsoleHistory] = useState<readonly AgentConsoleMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [scopeDraft, setScopeDraft] = useState<ConversationScope | null>(null);
  const [messageStatus, setMessageStatus] = useState<
    "saving" | "responding" | "rejected" | null
  >(null);
  const messageSubmissionInFlightRef = useRef(false);
  const [messageFailure, setMessageFailure] = useState<string | null>(null);
  const [wayfinder, setWayfinder] = useState<WayfinderProjection | null>(null);
  const [workingContext, setWorkingContext] = useState<WorkingContextProjection | null>(null);
  const [workingContextLoadFailure, setWorkingContextLoadFailure] = useState<string | null>(null);
  const [consoleHistoryLoadFailure, setConsoleHistoryLoadFailure] = useState<string | null>(null);
  const [capabilityLoadFailure, setCapabilityLoadFailure] = useState<string | null>(null);
  const [startupHydration, setStartupHydration] = useState({
    capabilities: false,
    consoleHistory: false,
    workingContext: false,
    workspaceQueue: false,
    shell: false,
  });
  const [usablePaintComplete, setUsablePaintComplete] = useState(false);
  const [contextStatus, setContextStatus] = useState<
    "pending" | "acknowledged" | "stale" | "rejected" | null
  >(null);
  const [contextActionFailure, setContextActionFailure] = useState<string | null>(null);
  const [reviewWorkspace, setReviewWorkspace] = useState<ReviewWorkspaceProjection | null>(null);
  const [reviewWorkspaceLoadFailure, setReviewWorkspaceLoadFailure] = useState<string | null>(null);
  const [reviewStatus, setReviewStatus] = useState<{
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null>(null);
  const [reviewReasons, setReviewReasons] = useState<Record<string, string>>({});
  const [workspaceQueue, setWorkspaceQueue] = useState<WorkspaceQueueProjection | null>(null);
  const [workspaceQueueLoadFailure, setWorkspaceQueueLoadFailure] = useState<string | null>(null);
  const [missionDrafts, setMissionDrafts] = useState<MissionDraftProjection | null>(null);
  const [missionDraftLoadFailure, setMissionDraftLoadFailure] = useState<string | null>(null);
  const [missionDraftStatus, setMissionDraftStatus] = useState<{
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null>(null);
  const [missionDraftReasons, setMissionDraftReasons] = useState<Record<string, string>>({});
  const [queueStatus, setQueueStatus] = useState<{
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null>(null);
  const workspacePath =
    state !== "loading" && (state.kind === "ready" || state.kind === "empty")
      ? state.snapshot.workspace_session.workspace_path
      : "";
  const failedWorkstationActionContinuityKey =
    state !== "loading" && (state.kind === "ready" || state.kind === "empty")
      ? failedWorkstationActionStorageKey(state.snapshot)
      : "";
  const [workstationActionTurns, setWorkstationActionTurns] = useState<
    readonly WorkstationActionTurn[]
  >([]);
  const workstationActionTurnsWorkspaceKeyRef = useRef("");
  const [workstationActionState, setWorkstationActionState] =
    useState<WorkstationActionState | null>(null);
  const [workstationReviewActionStates, setWorkstationReviewActionStates] = useState<
    Readonly<Record<string, WorkstationActionState>>
  >({});
  const [workstationActionDrafts, setWorkstationActionDrafts] = useState<
    Record<string, WorkstationActionDraftState>
  >({});
  const workstationActionStatusRef = useRef<HTMLSpanElement>(null);
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
  const [activityLoadFailure, setActivityLoadFailure] = useState<string | null>(null);
  const [commandAuditOpen, setCommandAuditOpen] = useState(false);
  const [launchContext, setLaunchContext] = useState<AlfredoLaunchContext | null>(null);
  const launchContextLoadInFlightRef = useRef<Promise<AlfredoLaunchContext | null> | null>(null);
  const [codingWorkspacePath, setCodingWorkspacePath] = useState("");
  const [codingWorkspaceStatus, setCodingWorkspaceStatus] = useState<{
    readonly state: "pending" | "acknowledged" | "rejected";
    readonly message: string;
  } | null>(null);
  const [missionChoiceStatus, setMissionChoiceStatus] = useState<{
    readonly state: "pending" | "acknowledged" | "rejected";
    readonly message: string;
  } | null>(null);
  const [missionTitle, setMissionTitle] = useState("");
  useEffect(() => {
    if (launchContext?.phase !== "selection-required") return;
    setCodingWorkspacePath((current) =>
      current.trim() ? current : launchContext.suggested_workspace_path ?? "",
    );
  }, [launchContext?.phase, launchContext?.suggested_workspace_path]);
  const pendingWorkspaceSelectionRef = useRef<CodingWorkspaceSelectionRequest | null>(null);
  const pendingMissionChoiceRef = useRef<{
    readonly correlation_id: string;
    readonly expected_revision: number;
    readonly choice: "resume" | "new";
    readonly mission_id: string;
    readonly mission_title?: string;
  } | null>(null);
  const nextWorkspaceSelectionIdRef = useRef(0);
  const nextMissionChoiceIdRef = useRef(0);
  const [capabilityCatalog, setCapabilityCatalog] = useState<AgentCapabilityCatalog | null>(null);
  const capabilityLoadInFlightRef = useRef<Promise<AgentCapabilityCatalog | null> | null>(null);
  const [selectedControllerId, setSelectedControllerId] = useState("");
  const workspaceStateRef = useRef<WorkspaceLoadResult | "loading">("loading");
  const dispatchedSessionIdsRef = useRef(new Set<string>());
  const queuedSessionRunAttemptsRef = useRef(new Map<string, number>());
  const queuedSessionRetryTimersRef = useRef(new Map<string, number>());
  const startQueuedSessionRef = useRef<(sessionId: string, missionId?: string) => void>(() => {});
  const bootPaintMarkedRef = useRef(false);
  const usablePaintStartedRef = useRef(false);
  const usablePaintMarkedRef = useRef(false);
  const hydratedPaintMarkedRef = useRef(false);
  const runnerClaimMeasurementsRef = useRef(
    new Map<string, { missionId: string; sessionId: string }>(),
  );
  const timelineOrderByKeyRef = useRef(new Map<string, number>());
  const nextTimelineOrderRef = useRef(0);
  const consoleTimelineKeyByMessageIdRef = useRef(new Map<string, string>());
  const initialConsoleTimelineKeysRef = useRef(new Set<string>());
  const registerTimelineTurn = useCallback((key: string): number => {
    const existing = timelineOrderByKeyRef.current.get(key);
    if (existing !== undefined) return existing;
    const order = nextTimelineOrderRef.current;
    nextTimelineOrderRef.current += 1;
    timelineOrderByKeyRef.current.set(key, order);
    return order;
  }, []);
  const consoleTimelineKey = useCallback((message: AgentConsoleMessage): string => {
    const existing = consoleTimelineKeyByMessageIdRef.current.get(message.message_id);
    if (existing) return existing;
    const key = `console:${message.sequence}:${message.message_id}`;
    consoleTimelineKeyByMessageIdRef.current.set(message.message_id, key);
    return key;
  }, []);
  const registerConsoleTimelineMessage = useCallback(
    (message: AgentConsoleMessage, aliasKey?: string): string => {
      const key = aliasKey ?? consoleTimelineKey(message);
      consoleTimelineKeyByMessageIdRef.current.set(message.message_id, key);
      registerTimelineTurn(key);
      return key;
    },
    [consoleTimelineKey, registerTimelineTurn],
  );
  const isInitialConsoleTimelineKey = useCallback(
    (key: string): boolean => initialConsoleTimelineKeysRef.current.has(key),
    [],
  );
  const appendWorkstationActionTurns = useCallback(
    (newTurns: readonly WorkstationActionTurn[]): void => {
      for (const turn of newTurns) {
        registerTimelineTurn(`workstation-action:${turn.id}`);
      }
      setWorkstationActionTurns((turns) => {
        const workspaceTurns =
          workstationActionTurnsWorkspaceKeyRef.current ===
          failedWorkstationActionContinuityKey
            ? turns
            : [];
        workstationActionTurnsWorkspaceKeyRef.current =
          failedWorkstationActionContinuityKey;
        return [...workspaceTurns, ...newTurns];
      });
    },
    [failedWorkstationActionContinuityKey, registerTimelineTurn],
  );
  const appendWorkstationActionTurn = useCallback((turn: WorkstationActionTurn) => {
    appendWorkstationActionTurns([turn]);
  }, [appendWorkstationActionTurns]);
  const registerCommandTimelineTurn = useCallback(
    (commandId: string) => {
      registerTimelineTurn(`command:${commandId}`);
    },
    [registerTimelineTurn],
  );
  const beginVisibleWorkstationAction = useCallback(
    (
      correlationId: string,
      label: string,
      targetId: string,
      onState?: (next: WorkstationActionState) => void,
    ) => {
      const nextState: WorkstationActionState = {
        itemId: targetId,
        state: "pending",
        message: `Waiting for Orchestrator acknowledgement: ${label}.`,
      };
      if (onState) onState(nextState);
      else setWorkstationActionState(nextState);
      appendWorkstationActionTurns([
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
    [appendWorkstationActionTurns],
  );
  const finishVisibleWorkstationAction = useCallback(
    (
      correlationId: string,
      targetId: string,
      result: "acknowledged" | "stale" | "rejected" | "failed",
      message: string,
      acknowledgementCorrelationId = "",
      onState?: (next: WorkstationActionState) => void,
    ) => {
      const receiptMatches =
        result !== "acknowledged" || acknowledgementCorrelationId === correlationId;
      const visibleResult = receiptMatches ? result : "failed";
      const visibleMessage = receiptMatches
        ? message
        : "Orchestrator acknowledgement correlation did not match the requested action.";
      const state =
        visibleResult === "acknowledged"
          ? "accepted"
          : visibleResult === "failed"
            ? "failed"
            : visibleResult === "stale"
              ? "stale"
              : "rejected";
      const recovery =
        visibleResult === "stale"
          ? `${visibleMessage} Refresh the canonical workspace state and retry the action.`
          : visibleMessage;
      const nextState: WorkstationActionState = {
        itemId: targetId,
        state,
        message: recovery,
      };
      if (onState) onState(nextState);
      else setWorkstationActionState(nextState);
      appendWorkstationActionTurns([
        {
          id: `${correlationId}:reaction:${visibleResult}`,
          content:
            visibleResult === "acknowledged"
              ? `Orchestrator accepted workstation action: ${visibleMessage}`
              : `Orchestrator ${
                  visibleResult === "stale"
                    ? "reported stale state"
                    : "rejected workstation action"
                }: ${recovery}`,
          source: "orchestrator",
          outcome: visibleResult,
          receiptCorrelationId:
            visibleResult === "acknowledged" ? acknowledgementCorrelationId : undefined,
          receiptPhase:
            visibleResult === "acknowledged"
              ? "workstation-action-acknowledged"
              : undefined,
        },
      ]);
    },
    [appendWorkstationActionTurns],
  );
  const setVisibleReviewActionState = useCallback((next: WorkstationActionState) => {
    setWorkstationReviewActionStates((current) => ({ ...current, [next.itemId]: next }));
  }, []);
  const hydratedFailedWorkstationActionKeyRef = useRef("");
  const skipFailedWorkstationActionWriteRef = useRef(false);
  useEffect(() => {
    if (
      !failedWorkstationActionContinuityKey ||
      hydratedFailedWorkstationActionKeyRef.current === failedWorkstationActionContinuityKey
    ) {
      return;
    }
    const restored = readFailedWorkstationActionContinuity(
      failedWorkstationActionContinuityKey,
    );
    skipFailedWorkstationActionWriteRef.current = true;
    for (const turn of restored) {
      registerTimelineTurn(`workstation-action:${turn.id}`);
    }
    setWorkstationActionTurns((current) => {
      const sameWorkspace =
        workstationActionTurnsWorkspaceKeyRef.current ===
        failedWorkstationActionContinuityKey;
      workstationActionTurnsWorkspaceKeyRef.current =
        failedWorkstationActionContinuityKey;
      if (!sameWorkspace) return restored;
      const merged = new Map(restored.map((turn) => [turn.id, turn]));
      for (const turn of current) merged.set(turn.id, turn);
      return [...merged.values()];
    });
    hydratedFailedWorkstationActionKeyRef.current = failedWorkstationActionContinuityKey;
  }, [failedWorkstationActionContinuityKey, registerTimelineTurn]);
  useEffect(() => {
    if (
      !failedWorkstationActionContinuityKey ||
      hydratedFailedWorkstationActionKeyRef.current !== failedWorkstationActionContinuityKey
    ) {
      return;
    }
    if (skipFailedWorkstationActionWriteRef.current) {
      skipFailedWorkstationActionWriteRef.current = false;
      return;
    }
    writeFailedWorkstationActionContinuity(
      failedWorkstationActionContinuityKey,
      workstationActionTurns,
    );
  }, [failedWorkstationActionContinuityKey, workstationActionTurns]);
  const controllerPreferenceKey = workspacePath
    ? `alfredo:selected-controller:${/^\/mnt\/[a-z]\//i.test(workspacePath) ? workspacePath.toLowerCase() : workspacePath}`
    : "";
  const shellTerminal = useShellTerminal(client, workspacePath, {
    onWorkstationActionTurn: appendWorkstationActionTurn,
    onCommandTurnAvailable: registerCommandTimelineTurn,
  });

  useEffect(() => {
    if (!workspacePath) return;
    void shellTerminal.load().finally(() => {
      setStartupHydration((current) => ({ ...current, shell: true }));
    });
  }, [shellTerminal.load, workspacePath]);

  useEffect(() => {
    if (commandAuditOpen) void shellTerminal.load();
  }, [commandAuditOpen, shellTerminal.load]);

  const refreshWorkingContext = useCallback(async () => {
    if (!client.loadWorkingContext) {
      setWorkingContextLoadFailure("Working Context transport is unavailable.");
      setStartupHydration((current) => ({ ...current, workingContext: true }));
      return false;
    }
    const result = await client.loadWorkingContext();
    if (result.kind !== "working-context") {
      setWorkingContextLoadFailure(result.message);
      setStartupHydration((current) => ({ ...current, workingContext: true }));
      return false;
    }
    setWorkingContext(result.projection);
    setWorkingContextLoadFailure(null);
    setStartupHydration((current) => ({ ...current, workingContext: true }));
    return true;
  }, [client]);

  const refreshReviewWorkspace = useCallback(async () => {
    if (!client.loadReviewWorkspace) {
      setReviewWorkspaceLoadFailure("Review Workspace transport is unavailable.");
      return false;
    }
    const result = await client.loadReviewWorkspace();
    if (result.kind !== "review-workspace") {
      setReviewWorkspaceLoadFailure(result.message);
      return false;
    }
    setReviewWorkspace(result.projection);
    setReviewWorkspaceLoadFailure(null);
    return true;
  }, [client]);

  const refreshWorkspaceQueue = useCallback(async () => {
    if (!client.loadWorkspaceQueue) {
      setWorkspaceQueueLoadFailure("Workspace Queue transport is unavailable.");
      setStartupHydration((current) => ({ ...current, workspaceQueue: true }));
      return false;
    }
    const result = await client.loadWorkspaceQueue();
    if (result.kind !== "workspace-queue") {
      setWorkspaceQueueLoadFailure(result.message);
      setStartupHydration((current) => ({ ...current, workspaceQueue: true }));
      return false;
    }
    setWorkspaceQueue(result.projection);
    setWorkspaceQueueLoadFailure(null);
    setStartupHydration((current) => ({ ...current, workspaceQueue: true }));
    return true;
  }, [client]);

  const refreshMissionDrafts = useCallback(async (): Promise<boolean> => {
    if (!client.loadMissionDrafts) {
      setMissionDraftLoadFailure("Mission Draft transport is unavailable.");
      return false;
    }
    const result = await client.loadMissionDrafts();
    if (result.kind !== "mission-drafts") {
      setMissionDraftLoadFailure(result.message);
      return false;
    }
    setMissionDrafts(result.projection);
    setMissionDraftLoadFailure(null);
    return true;
  }, [client]);

  const refreshActivityJournal = useCallback(
    async (filters: ActivityJournalFilters) => {
      if (!client.loadActivityJournal) {
        setActivityLoadFailure("Activity Journal transport is unavailable.");
        setActivityStatus("rejected");
        return false;
      }
      setActivityStatus("pending");
      const result = await client.loadActivityJournal(filters);
      if (result.kind !== "activity-journal") {
        setActivityLoadFailure(result.message);
        setActivityStatus("rejected");
        return false;
      }
      setActivityJournal(result.projection);
      setActivityLoadFailure(null);
      setActivityStatus(null);
      return true;
    },
    [client],
  );

  useEffect(() => {
    workspaceStateRef.current = state;
  }, [state]);

  useEffect(() => {
    if (
      state === "loading" ||
      (state.kind !== "ready" && state.kind !== "empty") ||
      runnerClaimMeasurementsRef.current.size === 0
    ) {
      return;
    }
    for (const [identity, pending] of runnerClaimMeasurementsRef.current) {
      const mission = pending.missionId
        ? state.snapshot.missions?.find((candidate) => candidate.id === pending.missionId)
        : state.snapshot.missions?.find((candidate) =>
            candidate.sessions.some(
              (session) => session.session_id === pending.sessionId,
            ),
          );
      const session = mission?.sessions.find(
        (candidate) => candidate.session_id === pending.sessionId,
      );
      if (
        !session ||
        session.status.toLowerCase() === "queued" ||
        !session.runner_started_at
      ) {
        continue;
      }
      runnerClaimMeasurementsRef.current.delete(identity);
      void afterTwoAnimationFrames().then(() =>
        markFrontendPerformance(client, "R6", "end", {
          outcome: "pass",
          mission_id: mission?.id ?? pending.missionId,
          session_id: pending.sessionId,
          rendered_status: session.status,
          runner_started_at: session.runner_started_at,
        }),
      );
    }
  }, [client, state]);

  useEffect(() => {
    if (bootPaintMarkedRef.current) return;
    bootPaintMarkedRef.current = true;
    void afterTwoAnimationFrames().then(() =>
      markFrontendPerformance(client, "S3", "end", { outcome: "pass" }),
    );
  }, [client]);

  useEffect(() => {
    if (
      usablePaintMarkedRef.current ||
      state === "loading" ||
      (state.kind !== "ready" && state.kind !== "empty")
    ) {
      return;
    }
    usablePaintMarkedRef.current = true;
    const snapshot = state.snapshot;
    void (async () => {
      await afterTwoAnimationFrames();
      const detail = {
        outcome: "pass",
        workspace_session_id: snapshot.workspace_session.id,
        active_mission_id: snapshot.active_mission?.id ?? "",
      };
      await markFrontendPerformance(client, "S8", "end", detail);
      await markFrontendPerformance(client, "S9", "start", detail);
      setUsablePaintComplete(true);
    })();
  }, [client, state]);

  useEffect(() => {
    if (
      hydratedPaintMarkedRef.current ||
      !usablePaintComplete ||
      !Object.values(startupHydration).every(Boolean) ||
      state === "loading" ||
      (state.kind !== "ready" && state.kind !== "empty")
    ) {
      return;
    }
    hydratedPaintMarkedRef.current = true;
    const snapshot = state.snapshot;
    void afterTwoAnimationFrames().then(() =>
      markFrontendPerformance(client, "S9", "end", {
        outcome: "pass",
        workspace_session_id: snapshot.workspace_session.id,
        active_mission_id: snapshot.active_mission?.id ?? "",
        hydration: startupHydration,
      }),
    );
  }, [client, startupHydration, state, usablePaintComplete]);

  const refreshLaunchContext = useCallback((): Promise<AlfredoLaunchContext | null> => {
    if (launchContextLoadInFlightRef.current) return launchContextLoadInFlightRef.current;
    if (!client.loadLaunchContext) return Promise.resolve(null);
    const request = (async (): Promise<AlfredoLaunchContext | null> => {
      const result = await client.loadLaunchContext!();
      if (result.kind !== "launch-context") return null;
      setLaunchContext(result.context);
      return result.context;
    })();
    launchContextLoadInFlightRef.current = request;
    void request.finally(() => {
      if (launchContextLoadInFlightRef.current === request) {
        launchContextLoadInFlightRef.current = null;
      }
    });
    return request;
  }, [client]);

  const connect = useCallback((contextOverride?: AlfredoLaunchContext) => {
    setState("loading");
    void (async () => {
      const context = contextOverride ?? (client.loadLaunchContext ? await refreshLaunchContext() : null);
      if (
        context &&
        (context.phase === "selection-required" ||
          context.phase === "mission-choice-required")
      ) {
        setConnectionStatus("connected");
        return;
      }
      await markNativePerformance(client, "S4", "start", { outcome: "pass" });
      const result = await client.loadSnapshot();
      if (
        !usablePaintStartedRef.current &&
        (result.kind === "ready" || result.kind === "empty")
      ) {
        usablePaintStartedRef.current = true;
        void markFrontendPerformance(client, "S8", "start", {
          outcome: "pass",
          workspace_session_id: result.snapshot.workspace_session.id,
          active_mission_id: result.snapshot.active_mission?.id ?? "",
        });
      }
      setState(result);
      setConnectionStatus(
        result.kind === "ready" || result.kind === "empty" ? "connected" : "offline",
      );
    })();
  }, [client, refreshLaunchContext]);

  useEffect(connect, [connect]);

  const refreshCapabilities = useCallback((): Promise<AgentCapabilityCatalog | null> => {
    if (capabilityLoadInFlightRef.current) return capabilityLoadInFlightRef.current;
    const request = (async (): Promise<AgentCapabilityCatalog | null> => {
      if (!client.loadAgentCapabilities) {
        setCapabilityLoadFailure("Capability catalog transport is unavailable.");
        setStartupHydration((current) => ({ ...current, capabilities: true }));
        return null;
      }
      const result = await client.loadAgentCapabilities();
      if (result.kind !== "capabilities") {
        setCapabilityLoadFailure(result.message);
        setStartupHydration((current) => ({ ...current, capabilities: true }));
        return null;
      }
      setCapabilityCatalog(result.catalog);
      setCapabilityLoadFailure(null);
      setStartupHydration((current) => ({ ...current, capabilities: true }));
      return result.catalog;
    })();
    capabilityLoadInFlightRef.current = request;
    void request.finally(() => {
      if (capabilityLoadInFlightRef.current === request) {
        capabilityLoadInFlightRef.current = null;
      }
    });
    return request;
  }, [client]);

  useEffect(() => {
    void refreshCapabilities();
  }, [refreshCapabilities]);

  useEffect(() => {
    if (!capabilityCatalog) return;
    setSelectedControllerId((current) => {
      const persisted = controllerPreferenceKey
        ? window.localStorage.getItem(controllerPreferenceKey) ?? ""
        : "";
      const persistedController = eligibleControllerId(capabilityCatalog, persisted);
      if (persistedController) return persistedController;
      const launched = launchContext?.selected_agent ?? "";
      return preferredEligibleControllerId(capabilityCatalog, [launched, current]);
    });
  }, [capabilityCatalog, controllerPreferenceKey, launchContext?.selected_agent]);

  const selectController = useCallback((agentId: string) => {
    setSelectedControllerId(agentId);
    if (controllerPreferenceKey) {
      window.localStorage.setItem(controllerPreferenceKey, agentId);
    }
  }, [controllerPreferenceKey]);

  const effectiveControllerId = selectedControllerId || (
    capabilityCatalog
      ? eligibleControllerId(capabilityCatalog, capabilityCatalog.default_agent_id)
      : ""
  );

  const refreshConsoleHistory = useCallback(async () => {
    if (!client.loadConsoleHistory) {
      setConsoleHistoryLoadFailure("Console history transport is unavailable.");
      setStartupHydration((current) => ({ ...current, consoleHistory: true }));
      return false;
    }
    const result = await client.loadConsoleHistory();
    if (result.kind !== "history") {
      setConsoleHistoryLoadFailure(result.message);
      setStartupHydration((current) => ({ ...current, consoleHistory: true }));
      return false;
    }
    const messages = [...result.history.messages].sort(
      (left, right) => left.sequence - right.sequence,
    );
    for (const message of messages) {
      const key = registerConsoleTimelineMessage(message);
      initialConsoleTimelineKeysRef.current.add(key);
    }
    setConsoleHistory(messages);
    setConsoleHistoryLoadFailure(null);
    setStartupHydration((current) => ({ ...current, consoleHistory: true }));
    return true;
  }, [client, registerConsoleTimelineMessage]);

  useEffect(() => {
    void refreshConsoleHistory();
  }, [refreshConsoleHistory]);

  useEffect(() => {
    void refreshWorkingContext();
  }, [refreshWorkingContext]);

  useEffect(() => {
    if (workspacePath) void refreshWorkspaceQueue();
  }, [refreshWorkspaceQueue, workspacePath]);

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
    let cancelled = false;
    let timer: number | undefined;

    const schedule = () => {
      if (!cancelled) timer = window.setTimeout(() => void poll(), syncIntervalMs);
    };
    const poll = async () => {
      try {
        const current = workspaceStateRef.current;
        if (
          current === "loading" ||
          (current.kind !== "ready" && current.kind !== "empty")
        ) {
          schedule();
          return;
        }

        if (client.loadUpdates) {
          const updates = await client.loadUpdates(current.snapshot.revision);
          if (cancelled) return;
          if (updates.kind !== "updates") {
            setConnectionStatus("offline");
            if (updates.recoverable) schedule();
            return;
          }
          const applied = applyWorkspaceUpdates(current.snapshot, updates.batch);
          if (applied.kind !== "applied") {
            setConnectionStatus("offline");
            schedule();
            return;
          }
          if (applied.snapshot !== current.snapshot) {
            const next = { ...current, snapshot: applied.snapshot };
            workspaceStateRef.current = next;
            setState(next);
            setConnectionStatus("connected");
            schedule();
            return;
          }
        }

        // Agent lifecycle can change without a navigation revision. Keep the
        // canonical probe, but retain the existing object when nothing changed
        // so quiet ticks do not retrigger every secondary projection.
        const canonical = await client.loadSnapshot();
        if (cancelled) return;
        if (canonical.kind !== "ready" && canonical.kind !== "empty") {
          setConnectionStatus("offline");
          if (canonical.recoverable) schedule();
          return;
        }
        if (!sameCanonicalWorkspace(current, canonical)) {
          workspaceStateRef.current = canonical;
          setState(canonical);
        }
        setConnectionStatus("connected");
        schedule();
      } catch {
        if (cancelled) return;
        setConnectionStatus("offline");
        schedule();
      }
    };

    schedule();
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [client, syncIntervalMs]);

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
      setActionFailure(null);
      setActionStatus("pending");
      const result = await client.submitAction({
        correlation_id: `operations-view-${operationsView}-${current.snapshot.revision}`,
        expected_revision: current.snapshot.revision,
        operations_view: operationsView,
      });
      if (result.kind !== "acknowledged") {
        setActionFailure(result.message);
        setActionStatus(result.kind);
        return;
      }
      if (!client.loadUpdates) {
        setActionFailure("Workspace update transport is unavailable after acknowledgement.");
        setActionStatus("rejected");
        return;
      }
      const updates = await client.loadUpdates(current.snapshot.revision);
      if (updates.kind !== "updates") {
        setActionFailure(updates.message);
        setActionStatus("rejected");
        return;
      }
      const applied = applyWorkspaceUpdates(current.snapshot, updates.batch);
      if (applied.kind !== "applied") {
        setActionFailure("Acknowledged workspace updates could not be applied in canonical order.");
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
      setActionFailure(null);
      setActionStatus("pending");
      const result = await client.switchMission({
        correlation_id: `active-mission-${missionId}-${current.snapshot.revision}`,
        expected_revision: current.snapshot.revision,
        active_mission_id: missionId,
      });
      if (result.kind !== "acknowledged") {
        setActionFailure(result.message);
        setActionStatus(result.kind);
        return;
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        setActionFailure(reloaded.message);
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

  const submitScope = useCallback(async (requestedScope?: ConversationScope) => {
    const targetScope = requestedScope ?? scopeDraft;
    if (
      !targetScope ||
      state === "loading" ||
      (state.kind !== "ready" && state.kind !== "empty") ||
      !client.changeScope ||
      !client.loadUpdates
    ) {
      return;
    }
    const current = state;
    const correlationId = `conversation-scope-${targetScope.kind}-${targetScope.target_id}-${current.snapshot.revision}`;
    beginVisibleWorkstationAction(
      correlationId,
      `Change Conversation Scope to ${targetScope.label}`,
      `scope:${targetScope.kind}:${targetScope.target_id}`,
    );
    setActionFailure(null);
    setActionStatus("pending");
    const result = await client.changeScope({
      correlation_id: correlationId,
      action_type: "conversation-scope-change",
      actor: "mission-commander",
      expected_revision: current.snapshot.revision,
      target: {
        kind: "conversation-scope",
        id: targetScope.target_id,
      },
      scope_kind: targetScope.kind,
      scope_target: targetScope.target_id,
      scope_label: targetScope.label,
    });
    if (result.kind !== "acknowledged") {
      setActionStatus(result.kind);
      finishVisibleWorkstationAction(
        correlationId,
        `scope:${targetScope.kind}:${targetScope.target_id}`,
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
        `scope:${targetScope.kind}:${targetScope.target_id}`,
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
        `scope:${targetScope.kind}:${targetScope.target_id}`,
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
      `scope:${targetScope.kind}:${targetScope.target_id}`,
      "acknowledged",
      `Conversation Scope now targets ${targetScope.label}.`,
      result.acknowledgement.correlation_id,
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
    if (messageSubmissionInFlightRef.current) return;
    messageSubmissionInFlightRef.current = true;
    try {
    const scope = state.snapshot.conversation_scope;
    const content = draft.trim();
    setMessageFailure(null);
    const optimisticMessageId = `console-pending-${Date.now()}`;
    const optimisticSequence = consoleHistory.length + 1;
    const optimisticTimelineKey = `console:${optimisticSequence}:${optimisticMessageId}`;
    registerConsoleTimelineMessage(
      {
        message_id: optimisticMessageId,
        sequence: optimisticSequence,
        role: "user",
        content,
        scope,
        outcome: "proposed",
        source: "mission-commander",
      },
      optimisticTimelineKey,
    );
    setMessageStatus("saving");
    setDraft("");
    setConsoleHistory((messages) => [
      ...messages,
      {
        message_id: optimisticMessageId,
        sequence: optimisticSequence,
        role: "user",
        content,
        scope,
        outcome: "proposed",
        source: "mission-commander",
      },
    ]);
    const result = await client.appendConsoleMessage({
      role: "user",
      content,
      outcome: "proposed",
      source: "mission-commander",
      expected_revision: state.snapshot.revision,
      scope_kind: scope.kind,
      scope_target: scope.target_id,
      scope_label: scope.label,
      scope_mission_id: scope.mission_id ?? undefined,
    });
    if (result.kind !== "message") {
      setConsoleHistory((messages) =>
        messages.filter((message) => message.message_id !== optimisticMessageId),
      );
      setDraft((current) => current || content);
      setMessageFailure(result.message);
      setMessageStatus("rejected");
      return;
    }
    registerConsoleTimelineMessage(result.message, optimisticTimelineKey);
    setConsoleHistory((messages) => [
      ...messages.filter((message) => message.message_id !== optimisticMessageId),
      result.message,
    ]);
    const slashCommand = parseSlashCommand(content);
    if (slashCommand?.name === "/run" && slashCommand.argument) {
      setMessageStatus(null);
      await shellTerminal.submitCommand(slashCommand.argument);
      await refreshWorkingContext();
      return;
    }
    const delegateCodingTask = async (codingTask: CodingTaskIntent): Promise<void> => {
      const taskCapabilityCatalog = capabilityCatalog ?? await refreshCapabilities();
      if (!taskCapabilityCatalog) {
        const message =
          "Coding task delegation is waiting for the capability catalog. " +
          "Retry the prompt after Alfredo reconnects to its agents.";
        setMessageStatus(null);
        setQueueStatus({ state: "rejected", message });
        appendWorkstationActionTurns([
          {
            id: `capability-validation:${result.message.message_id}`,
            content: message,
            source: "orchestrator",
            outcome: "rejected",
          },
        ]);
        return;
      }
      const requestedSkill = codingTask.skillName?.replace(/^\$/, "") ?? null;
      const installedSkill = requestedSkill
        ? taskCapabilityCatalog.skills.find(
            (skill) => skill.name.toLowerCase() === requestedSkill.toLowerCase(),
          )
        : null;
      if (requestedSkill && !installedSkill) {
        const validationMessage =
          `Unknown skill ${requestedSkill}. Use /skills to choose an installed skill.`;
        setMessageStatus(null);
        setQueueStatus({ state: "rejected", message: validationMessage });
        appendWorkstationActionTurns([
          {
            id: `skill-validation:${result.message.message_id}`,
            content: validationMessage,
            source: "orchestrator",
            outcome: "rejected",
          },
        ]);
        await refreshWorkingContext();
        return;
      }
      const correlationId = `chat-task-${result.message.message_id}-${state.snapshot.revision}`;
      const actionLabel = codingTask.skillName
        ? `Propose coding task with skill ${codingTask.skillName}: ${codingTask.request}`
        : `Propose coding task: ${codingTask.request}`;
      beginVisibleWorkstationAction(correlationId, actionLabel, result.message.message_id);
      setMessageStatus(null);
      setQueueStatus({ state: "pending", message: "Coding task proposal pending approval" });
      const worker = taskCapabilityCatalog.agents.find(
        isEligibleWorkerCapability,
      );
      if (!client.submitAdHocDelegationProposal || !worker) {
        const message = client.submitAdHocDelegationProposal
          ? "No available ungated assignable local worker can accept this coding task."
          : "The Orchestrator proposal boundary is unavailable for this coding task.";
        setQueueStatus({ state: "rejected", message });
        finishVisibleWorkstationAction(
          correlationId,
          result.message.message_id,
          "rejected",
          message,
        );
        await refreshWorkingContext();
        return;
      }
      const proposalRequest: AdHocDelegationProposalRequest = {
        correlation_id: correlationId,
        expected_revision: state.snapshot.revision,
        source: "agent-console",
        scope_kind: scope.kind,
        scope_target: scope.target_id,
        scope_label: scope.label,
        mission_id: scope.mission_id ?? state.snapshot.active_mission?.id,
        acceptance_criteria: codingTask.acceptanceCriteria,
        allowed_paths: [state.snapshot.workspace_session.workspace_path],
        command_policy: {},
        proposed_agent: worker.id,
        originating_message_id: result.message.message_id,
      };
      const proposal = await client.submitAdHocDelegationProposal(proposalRequest);
      if (proposal.kind !== "acknowledged") {
        setQueueStatus({ state: proposal.kind, message: proposal.message });
        finishVisibleWorkstationAction(
          correlationId,
          result.message.message_id,
          proposal.kind,
          proposal.message,
        );
        return;
      }
      setQueueStatus({
        state: "acknowledged",
        message: proposal.acknowledgement.effect_summary,
      });
      finishVisibleWorkstationAction(
        correlationId,
        result.message.message_id,
        "acknowledged",
        proposal.acknowledgement.effect_summary,
        proposal.acknowledgement.correlation_id,
      );
      if (
        !client.loadWorkspaceQueue ||
        !client.submitWorkspaceQueueDecision ||
        !client.runWorkstationSession
      ) {
        await refreshWorkspaceQueue();
        await refreshWorkingContext();
        return;
      }
      const queueResult = await client.loadWorkspaceQueue();
      if (queueResult.kind !== "workspace-queue") {
        const message =
          `Automatic approval paused because the canonical Workspace Queue could not be loaded: ` +
          `${queueResult.message} Review the pending proposal and retry.`;
        setQueueStatus({ state: "rejected", message });
        appendWorkstationActionTurns([
          {
            id: `${correlationId}:auto-approval:queue-load-failed`,
            content: message,
            source: "orchestrator",
            outcome: "rejected",
          },
        ]);
        await refreshWorkingContext();
        return;
      }
      setWorkspaceQueue(queueResult.projection);
      const queueItem = queueResult.projection.items.find(
        (item) => item.item_id === proposal.acknowledgement.item_id,
      );
      if (
        queueResult.projection.revision !== proposal.acknowledgement.revision ||
        !queueItem ||
        !isExactAdHocDelegationBoundary(queueItem, proposalRequest, content)
      ) {
        const message =
          "Automatic approval paused because the canonical proposal no longer exactly matches " +
          "the authorized path, command policy, scope, agent, and acceptance criteria. " +
          "Review the pending Workspace Queue item manually.";
        setQueueStatus({ state: "rejected", message });
        appendWorkstationActionTurns([
          {
            id: `${correlationId}:auto-approval:boundary-mismatch`,
            content: message,
            source: "orchestrator",
            outcome: "rejected",
          },
        ]);
        await refreshWorkingContext();
        return;
      }
      const approvalCorrelationId =
        `chat-task-approve-${result.message.message_id}-${queueItem.item_id}-${proposal.acknowledgement.revision}`;
      beginVisibleWorkstationAction(
        approvalCorrelationId,
        `Approve exactly bounded coding task ${queueItem.issue_id}`,
        queueItem.item_id,
      );
      setQueueStatus({ state: "pending", message: "Exact coding task boundary approval pending" });
      const decision = await client.submitWorkspaceQueueDecision({
        correlation_id: approvalCorrelationId,
        action_type: "workspace-queue-decision",
        actor: "mission-commander",
        expected_revision: proposal.acknowledgement.revision,
        target: {
          kind: "workspace-queue-item",
          id: queueItem.item_id,
        },
        item_id: queueItem.item_id,
        decision: "approve",
        reason:
          `Mission Commander explicitly authorized this bounded coding task in Agent Console message ` +
          `${result.message.message_id}.`,
      });
      if (decision.kind !== "acknowledged") {
        setQueueStatus({ state: decision.kind, message: decision.message });
        finishVisibleWorkstationAction(
          approvalCorrelationId,
          queueItem.item_id,
          decision.kind,
          decision.message,
        );
        await refreshWorkingContext();
        return;
      }
      setQueueStatus({
        state: "acknowledged",
        message: decision.acknowledgement.effect_summary,
      });
      finishVisibleWorkstationAction(
        approvalCorrelationId,
        queueItem.item_id,
        "acknowledged",
        decision.acknowledgement.effect_summary,
        decision.acknowledgement.correlation_id,
      );
      if (decision.acknowledgement.session_id) {
        startQueuedSessionRef.current(
          decision.acknowledgement.session_id,
          queueItem.mission_id,
        );
      }
      await refreshWorkspaceQueue();
      await refreshWorkingContext();
      return;
    };
    const codingTask = parseCodingTaskIntent(content);
    if (codingTask) {
      await delegateCodingTask(codingTask);
      return;
    }
    if (client.generateConsoleResponse) {
      setMessageStatus("responding");
      let responseControllerId = "";
      if (!slashCommand) {
        const responseCapabilityCatalog = capabilityCatalog ?? (
          client.loadAgentCapabilities ? await refreshCapabilities() : null
        );
        if (client.loadAgentCapabilities && !responseCapabilityCatalog) {
          const message =
            "Controller response paused because the capability catalog is unavailable. " +
            "Retry after Alfredo reconnects to its local agent registry.";
          setMessageStatus(null);
          appendWorkstationActionTurns([
            {
              id: `controller-capability:${result.message.message_id}`,
              content: message,
              source: "orchestrator",
              outcome: "rejected",
            },
          ]);
          await refreshWorkingContext();
          return;
        }
        const responseLaunchContext = launchContext ?? (
          client.loadLaunchContext ? await refreshLaunchContext() : null
        );
        const persistedController = controllerPreferenceKey
          ? window.localStorage.getItem(controllerPreferenceKey) ?? ""
          : "";
        responseControllerId = responseCapabilityCatalog
          ? preferredEligibleControllerId(responseCapabilityCatalog, [
              persistedController,
              responseLaunchContext?.selected_agent ?? "",
              selectedControllerId,
            ])
          : effectiveControllerId;
        if (responseCapabilityCatalog && !responseControllerId) {
          const message =
            "Controller response paused because no validated available local controller is " +
            "present in the capability catalog. Retry after the agent registry is available.";
          setMessageStatus(null);
          appendWorkstationActionTurns([
            {
              id: `controller-validation:${result.message.message_id}`,
              content: message,
              source: "orchestrator",
              outcome: "rejected",
            },
          ]);
          await refreshWorkingContext();
          return;
        }
      }
      const response = await client.generateConsoleResponse({
        expected_revision: state.snapshot.revision,
        message_id: result.message.message_id,
        scope_kind: scope.kind,
        scope_target: scope.target_id,
        scope_label: scope.label,
        scope_mission_id: scope.mission_id ?? undefined,
        agent_id: responseControllerId || undefined,
      });
      if (response.kind === "message") {
        registerConsoleTimelineMessage(response.message);
        setConsoleHistory((messages) => [...messages, response.message]);
        setWayfinder(
          response.wayfinder && response.wayfinder.mode !== "outside"
            ? response.wayfinder
            : null,
        );
        if (response.route.intent === "coding-task") {
          await delegateCodingTask({
            request: response.route.task_request,
            skillName: null,
            acceptanceCriteria: response.route.acceptance_criteria,
          });
          return;
        }
      } else {
        appendWorkstationActionTurns([
          {
            id: `agent-console-response:${Date.now()}`,
            content: `Controller response failed: ${response.message}`,
            source: "frontier-model",
            outcome: "rejected",
          },
        ]);
      }
    }
    setMessageStatus(null);
    await refreshWorkingContext();
    } finally {
      messageSubmissionInFlightRef.current = false;
    }
  }, [
    appendWorkstationActionTurns,
    beginVisibleWorkstationAction,
    capabilityCatalog?.agents,
    capabilityCatalog?.skills,
    client,
    consoleHistory.length,
    draft,
    effectiveControllerId,
    finishVisibleWorkstationAction,
    launchContext,
    controllerPreferenceKey,
    refreshCapabilities,
    refreshLaunchContext,
    refreshWorkingContext,
    refreshWorkspaceQueue,
    registerConsoleTimelineMessage,
    selectedControllerId,
    shellTerminal,
    state,
  ]);

  const curateWorkingContext = useCallback(
    async (sourceId: string, disposition: "included" | "pinned" | "excluded") => {
      if (!workingContext || !client.curateWorkingContext) return;
      setContextActionFailure(null);
      setContextStatus("pending");
      const result = await client.curateWorkingContext({
        source_id: sourceId,
        disposition,
        expected_context_revision: workingContext.revision,
      });
      if (result.kind !== "acknowledged") {
        setContextActionFailure(result.message);
        setContextStatus(result.kind);
        return;
      }
      const reloaded = await refreshWorkingContext();
      if (!reloaded) {
        setContextActionFailure(
          "Context curation was acknowledged, but the refreshed Working Context could not be loaded.",
        );
      }
      setContextStatus(reloaded ? "acknowledged" : "rejected");
    },
    [client, refreshWorkingContext, workingContext],
  );

  const submitReviewDecision = useCallback(
    async (
      sessionId: string,
      decision: ReviewDecision,
      reason: string,
      missionId?: string,
    ) => {
      if (
        state === "loading" ||
        (state.kind !== "ready" && state.kind !== "empty") ||
        !client.submitReviewDecision
      ) {
        return;
      }
      const current = state;
      const targetMissionId = missionId ?? current.snapshot.active_mission?.id ?? "";
      const actionStateId = workstationReviewActionStateId(
        targetMissionId,
        sessionId,
        decision,
      );
      const correlationId = `review-${decision}-${targetMissionId}-${sessionId}-${current.snapshot.revision}`;
      beginVisibleWorkstationAction(
        correlationId,
        `${reviewDecisionLabel(decision)} for ${sessionId}`,
        actionStateId,
        setVisibleReviewActionState,
      );
      setReviewStatus({ state: "pending", message: "Review decision pending" });
      const result = await client.submitReviewDecision({
        correlation_id: correlationId,
        action_type: "review-decision",
        actor: "mission-commander",
        expected_revision: current.snapshot.revision,
        target: {
          kind: "agent-session",
          id: sessionId,
        },
        mission_id: targetMissionId || undefined,
        session_id: sessionId,
        decision,
        reason,
      });
      if (result.kind !== "acknowledged") {
        setReviewStatus({ state: result.kind, message: result.message });
        finishVisibleWorkstationAction(
          correlationId,
          actionStateId,
          result.kind,
          result.message,
          "",
          setVisibleReviewActionState,
        );
        return;
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        setReviewStatus({ state: "rejected", message: "Review acknowledged but reload failed" });
        finishVisibleWorkstationAction(
          correlationId,
          actionStateId,
          "failed",
          "Review acknowledged but canonical snapshot reload failed.",
          "",
          setVisibleReviewActionState,
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
        actionStateId,
        "acknowledged",
        result.acknowledgement.effect_summary,
        result.acknowledgement.correlation_id,
        setVisibleReviewActionState,
      );
      await refreshReviewWorkspace();
    },
    [
      beginVisibleWorkstationAction,
      client,
      finishVisibleWorkstationAction,
      refreshReviewWorkspace,
      setVisibleReviewActionState,
      state,
    ],
  );

  useEffect(() => {
    if (workstationActionState) workstationActionStatusRef.current?.focus();
  }, [workstationActionState]);

  const startQueuedSession = useCallback(
    (sessionId: string, missionId = "") => {
      if (!sessionId || !client.runWorkstationSession) return;
      const sessionIdentity = missionSessionIdentity(missionId, sessionId);
      if (
        dispatchedSessionIdsRef.current.has(sessionIdentity) ||
        queuedSessionRetryTimersRef.current.has(sessionIdentity)
      ) return;
      const priorAttempts = queuedSessionRunAttemptsRef.current.get(sessionIdentity) ?? 0;
      if (priorAttempts >= 3) return;
      if (
        priorAttempts > 0 &&
        !canonicalSessionIsQueued(workspaceStateRef.current, missionId, sessionId)
      ) return;
      const attempt = priorAttempts + 1;
      queuedSessionRunAttemptsRef.current.set(sessionIdentity, attempt);
      dispatchedSessionIdsRef.current.add(sessionIdentity);
      appendWorkstationActionTurns([
        {
          id: `session-run:${sessionIdentity}:${attempt === 1 ? "queued" : `retry-${attempt}`}`,
          content:
            attempt === 1
              ? `Local Agent ${sessionId} is queued and starting in the background.`
              : `Retrying Local Agent ${sessionId} runner dispatch (attempt ${attempt} of 3).`,
          source: "orchestrator",
          outcome: "queued",
        },
      ]);

      const reloadCanonical = async (): Promise<WorkspaceLoadResult | "loading"> => {
        try {
          const reloaded = await client.loadSnapshot();
          if (reloaded.kind === "ready" || reloaded.kind === "empty") {
            workspaceStateRef.current = reloaded;
            setState(reloaded);
            setConnectionStatus("connected");
            return reloaded;
          }
          return workspaceStateRef.current;
        } catch {
          return workspaceStateRef.current;
        }
      };
      const finishFailedAttempt = async (message: string, transportFailure: boolean) => {
        appendWorkstationActionTurns([
          {
            id: `session-run:${sessionIdentity}:${transportFailure ? "transport-failed" : "failed"}-${attempt}`,
            content: transportFailure
              ? `Local Agent ${sessionId} could not start: ${message}`
              : `Local Agent ${sessionId} failed to run: ${message}`,
            source: "orchestrator",
            outcome: "failed",
          },
        ]);
        const canonical = await reloadCanonical();
        if (
          attempt < 3 &&
          canonicalSessionIsQueued(canonical, missionId, sessionId)
        ) {
          const delayMs = 50 * 2 ** (attempt - 1);
          const timer = window.setTimeout(() => {
            queuedSessionRetryTimersRef.current.delete(sessionIdentity);
            startQueuedSessionRef.current(sessionId, missionId);
          }, delayMs);
          queuedSessionRetryTimersRef.current.set(sessionIdentity, timer);
        } else if (!canonicalSessionIsQueued(canonical, missionId, sessionId)) {
          queuedSessionRunAttemptsRef.current.delete(sessionIdentity);
        }
        dispatchedSessionIdsRef.current.delete(sessionIdentity);
      };

      void (async () => {
        try {
          const result = await client.runWorkstationSession!({
            session_id: sessionId,
            mission_id: missionId || undefined,
          });
          if (result.kind === "session-failed") {
            await finishFailedAttempt(result.message, false);
            return;
          }
          queuedSessionRunAttemptsRef.current.delete(sessionIdentity);
          appendWorkstationActionTurns([
            {
              id: `session-run:${sessionIdentity}:finished`,
              content: `Local Agent ${sessionId} finished with status ${result.session.status}.`,
              source: "orchestrator",
              outcome: result.session.status,
            },
          ]);
          await reloadCanonical();
        } catch (error: unknown) {
          await finishFailedAttempt(
            error instanceof Error ? error.message : String(error),
            true,
          );
        }
      })();
    },
    [appendWorkstationActionTurns, client],
  );

  useEffect(() => {
    startQueuedSessionRef.current = startQueuedSession;
  }, [startQueuedSession]);

  useEffect(
    () => () => {
      for (const timer of queuedSessionRetryTimersRef.current.values()) {
        window.clearTimeout(timer);
      }
      queuedSessionRetryTimersRef.current.clear();
    },
    [],
  );

  useEffect(() => {
    if (state === "loading" || (state.kind !== "ready" && state.kind !== "empty")) return;
    for (const mission of state.snapshot.missions ?? []) {
      for (const session of mission.sessions) {
        if (session.status.toLowerCase() === "queued") {
          startQueuedSession(session.session_id, mission.id);
        }
      }
    }
  }, [startQueuedSession, state]);

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
      const actionStateId = workstationActionStateId({
        label: actionLabel,
        target: "workspace-queue",
        requiresReason: decision !== "approve",
        actionType: "workspace-queue-decision",
        missionId: item?.mission_id,
        itemId,
        decision,
        targetIdentity: { kind: "workspace-queue-item", id: itemId },
      });
      void markFrontendPerformance(client, "R0", "start", {
        outcome: "pass",
        correlation_id: correlationId,
        decision,
      });
      setQueueStatus({ state: "pending", message: "Workspace Queue decision pending" });
      beginVisibleWorkstationAction(correlationId, actionLabel, actionStateId);
      void afterTwoAnimationFrames().then(() =>
        markFrontendPerformance(client, "R0", "end", {
          outcome: "pass",
          correlation_id: correlationId,
          decision,
        }),
      );
      await markNativePerformance(client, "R1", "start", {
        outcome: "pass",
        correlation_id: correlationId,
        decision,
      });
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
      await markNativePerformance(client, "R3", "end", {
        outcome: result.kind === "acknowledged" ? "pass" : "fail",
        correlation_id: correlationId,
        decision,
      });
      if (result.kind !== "acknowledged") {
        setQueueStatus({ state: result.kind, message: result.message });
        finishVisibleWorkstationAction(correlationId, actionStateId, result.kind, result.message);
        return;
      }
      if (result.acknowledgement.session_id) {
        const sessionId = result.acknowledgement.session_id;
        const missionId = item?.mission_id ?? "";
        const identity = missionSessionIdentity(missionId, sessionId);
        runnerClaimMeasurementsRef.current.set(identity, { missionId, sessionId });
        void markFrontendPerformance(client, "R6", "start", {
          outcome: "pass",
          correlation_id: correlationId,
          mission_id: missionId,
          session_id: sessionId,
        });
        startQueuedSession(result.acknowledgement.session_id, item?.mission_id ?? "");
      }
      void markFrontendPerformance(client, "R4", "start", {
        outcome: "pass",
        correlation_id: correlationId,
        decision,
      });
      const reloaded = await client.loadSnapshot();
      void markFrontendPerformance(client, "R4", "end", {
        outcome:
          reloaded.kind === "ready" || reloaded.kind === "empty" ? "pass" : "fail",
        correlation_id: correlationId,
        decision,
      });
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        setQueueStatus({ state: "rejected", message: "Queue acknowledged but reload failed" });
        finishVisibleWorkstationAction(
          correlationId,
          actionStateId,
          "failed",
          "Orchestrator acknowledged the action, but Alfredo could not reload canonical state.",
        );
        setConnectionStatus("offline");
        return;
      }
      void markFrontendPerformance(client, "R5", "start", {
        outcome: "pass",
        correlation_id: correlationId,
        decision,
        revision: reloaded.snapshot.revision,
      });
      setState(reloaded);
      setConnectionStatus("connected");
      setQueueStatus({
        state: "acknowledged",
        message: result.acknowledgement.effect_summary,
      });
      finishVisibleWorkstationAction(
        correlationId,
        actionStateId,
        "acknowledged",
        result.acknowledgement.effect_summary,
        result.acknowledgement.correlation_id,
      );
      void afterTwoAnimationFrames().then(() =>
        markFrontendPerformance(client, "R5", "end", {
          outcome: "pass",
          correlation_id: correlationId,
          decision,
          revision: reloaded.snapshot.revision,
          session_id: result.acknowledgement.session_id ?? "",
        }),
      );
      await refreshWorkspaceQueue();
    },
    [beginVisibleWorkstationAction, client, finishVisibleWorkstationAction, refreshWorkspaceQueue, startQueuedSession, state, workspaceQueue],
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
      if (
        action.actionType === "model-assignment-change" &&
        !capabilityCatalog?.agents.some(
          (agent) => agent.id === draft.agentId && isEligibleWorkerCapability(agent),
        )
      ) return;
      const actionStateId = workstationActionStateId(action);
      const correlationId = `workstation-${action.actionType}-${action.missionId ?? "workspace"}-${targetId}-${action.expectedRevision}`;
      const label = `${action.label} ${targetId}`;
      const request: WorkstationActionRequest = {
        correlation_id: correlationId,
        action_type: action.actionType,
        actor: "mission-commander",
        expected_revision: action.expectedRevision,
        target,
        mission_id: action.missionId,
        issue_id: action.issueId,
        session_id: action.sessionId,
        agent_id: draft.agentId.trim() || undefined,
        reason: draft.reason.trim() || undefined,
        allowed_paths: [],
        command_policy: {},
      };
      beginVisibleWorkstationAction(correlationId, label, actionStateId);
      const result = await client.submitWorkstationAction(request);
      if (result.kind !== "acknowledged") {
        finishVisibleWorkstationAction(correlationId, actionStateId, result.kind, result.message);
        return;
      }
      if (
        ["issue-launch", "issue-retry"].includes(action.actionType) &&
        result.acknowledgement.session_id
      ) {
        startQueuedSession(
          result.acknowledgement.session_id,
          action.missionId ?? state.snapshot.active_mission?.id ?? "",
        );
      }
      const reloaded = await client.loadSnapshot();
      if (reloaded.kind !== "ready" && reloaded.kind !== "empty") {
        finishVisibleWorkstationAction(
          correlationId,
          actionStateId,
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
        actionStateId,
        "acknowledged",
        result.acknowledgement.effect_summary,
        result.acknowledgement.correlation_id,
      );
    },
    [
      beginVisibleWorkstationAction,
      capabilityCatalog,
      client,
      finishVisibleWorkstationAction,
      startQueuedSession,
      state,
    ],
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
        await refreshMissionDrafts();
        const snapshotResult = await client.loadSnapshot();
        if (snapshotResult.kind !== "ready" && snapshotResult.kind !== "empty") {
          const message =
            `Mission Draft was acknowledged, but canonical snapshot reload failed: ` +
            `${snapshotResult.message} Retry the canonical workspace load.`;
          setMissionDraftStatus({ state: "rejected", message });
          finishVisibleWorkstationAction(correlationId, draftId, "failed", message);
          setConnectionStatus("offline");
          return;
        }
        setState(snapshotResult);
        setConnectionStatus("connected");
        setMissionDraftStatus({
          state: "acknowledged",
          message: result.acknowledgement.effect_summary,
        });
        finishVisibleWorkstationAction(
          correlationId,
          draftId,
          "acknowledged",
          result.acknowledgement.effect_summary,
          result.acknowledgement.correlation_id,
        );
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
        await refreshMissionDrafts();
        const snapshotResult = await client.loadSnapshot();
        if (snapshotResult.kind !== "ready" && snapshotResult.kind !== "empty") {
          const message =
            `Mission Draft was acknowledged, but canonical snapshot reload failed: ` +
            `${snapshotResult.message} Retry the canonical workspace load.`;
          setMissionDraftStatus({ state: "rejected", message });
          finishVisibleWorkstationAction(
            correlationId,
            "mission-draft:create",
            "failed",
            message,
          );
          setConnectionStatus("offline");
          return;
        }
        setState(snapshotResult);
        setConnectionStatus("connected");
        setMissionDraftStatus({
          state: "acknowledged",
          message: result.acknowledgement.effect_summary,
        });
        finishVisibleWorkstationAction(
          correlationId,
          "mission-draft:create",
          "acknowledged",
          result.acknowledgement.effect_summary,
          result.acknowledgement.correlation_id,
        );
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
        mission_id: scope.mission_id ?? current.snapshot.active_mission?.id,
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
        result.acknowledgement.correlation_id,
      );
      await refreshWorkspaceQueue();
    },
    [beginVisibleWorkstationAction, client, finishVisibleWorkstationAction, refreshWorkspaceQueue, state],
  );

  const selectCodingWorkspace = useCallback(
    async (selectionMode: "existing" | "create") => {
      if (!launchContext || !client.selectCodingWorkspace || !codingWorkspacePath.trim()) {
        setCodingWorkspaceStatus({
          state: "rejected",
          message: "Enter an exact repository path before requesting selection.",
        });
        return;
      }
      setCodingWorkspaceStatus({
        state: "pending",
        message: "Waiting for Orchestrator acknowledgement.",
      });
      const workspacePath = codingWorkspacePath.trim();
      const pendingRequest = pendingWorkspaceSelectionRef.current;
      const request: CodingWorkspaceSelectionRequest =
        pendingRequest?.workspace_path === workspacePath &&
        pendingRequest.selection_mode === selectionMode
          ? pendingRequest
          : {
              correlation_id:
                `workspace-select-${Date.now()}-${nextWorkspaceSelectionIdRef.current++}`,
              workspace_path: workspacePath,
              selection_mode: selectionMode,
            };
      pendingWorkspaceSelectionRef.current = request;
      const result = await client.selectCodingWorkspace(request);
      if (result.kind !== "acknowledged") {
        if (!result.recoverable) {
          pendingWorkspaceSelectionRef.current = null;
        }
        setCodingWorkspaceStatus({
          state: "rejected",
          message: workspaceSelectionFailureMessage(
            result.code,
            result.message,
            launchContext.suggested_workspace_path,
          ),
        });
        return;
      }
      if (
        !acknowledgementMatchesWorkspaceSelection(
          result.acknowledgement,
          request,
          launchContext.starting_location,
        )
      ) {
        setCodingWorkspaceStatus({
          state: "rejected",
          message:
            "invalid-workspace-acknowledgement: " +
            "The Orchestrator acknowledgement did not match the requested boundary.",
        });
        return;
      }
      pendingWorkspaceSelectionRef.current = null;
      setLaunchContext({
        ...launchContext,
        starting_location: result.acknowledgement.starting_location,
        coding_workspace: result.acknowledgement.coding_workspace,
        active_mission: null,
        revision: 1,
        known_missions: result.acknowledgement.known_missions ?? [],
        phase: "mission-choice-required",
      });
      setCodingWorkspaceStatus({
        state: "acknowledged",
        message: result.acknowledgement.message,
      });
    },
    [client, codingWorkspacePath, launchContext],
  );

  const chooseMission = useCallback(
    async (choice: "resume" | "new", option: MissionChoiceOption | null) => {
      if (
        !launchContext ||
        launchContext.phase !== "mission-choice-required" ||
        !client.chooseMission ||
        !launchContext.coding_workspace
      ) {
        setMissionChoiceStatus({
          state: "rejected",
          message: "Mission choice transport is unavailable until a Coding Workspace is bound.",
        });
        return;
      }
      const title = choice === "new" ? missionTitle.trim() : "";
      const missionId =
        choice === "resume"
          ? option?.id ?? ""
          : missionIdFromTitle(title);
      if (!missionId || (choice === "new" && !title)) {
        setMissionChoiceStatus({
          state: "rejected",
          message: "Enter a Mission title before starting a new Mission.",
        });
        return;
      }
      const expectedRevision = launchContext.revision ?? 1;
      const pending = pendingMissionChoiceRef.current;
      const request =
        pending &&
        pending.expected_revision === expectedRevision &&
        pending.choice === choice &&
        pending.mission_id === missionId &&
        (pending.mission_title ?? "") === title
          ? pending
          : {
              correlation_id:
                `mission-choice-${Date.now()}-${nextMissionChoiceIdRef.current++}`,
              expected_revision: expectedRevision,
              choice,
              mission_id: missionId,
              mission_title: title,
            };
      pendingMissionChoiceRef.current = request;
      setMissionChoiceStatus({
        state: "pending",
        message: choice === "resume" ? "Resuming Mission…" : "Starting new Mission…",
      });
      const result = await client.chooseMission(request);
      if (result.kind !== "acknowledged") {
        if (!result.recoverable) pendingMissionChoiceRef.current = null;
        setMissionChoiceStatus({
          state: "rejected",
          message: `${result.code}: ${result.message}`,
        });
        return;
      }
      const acknowledgement = result.acknowledgement;
      if (
        acknowledgement.schema_version !== 1 ||
        acknowledgement.outcome !== "acknowledged" ||
        acknowledgement.correlation_id !== request.correlation_id ||
        acknowledgement.coding_workspace !== launchContext.coding_workspace ||
        acknowledgement.choice !== choice ||
        acknowledgement.active_mission !== missionId ||
        !acknowledgement.missions.some((mission) => mission.id === missionId)
      ) {
        setMissionChoiceStatus({
          state: "rejected",
          message:
            "invalid-mission-acknowledgement: The Orchestrator acknowledgement did not match the requested Mission.",
        });
        return;
      }
      pendingMissionChoiceRef.current = null;
      const nextContext: AlfredoLaunchContext = {
        ...launchContext,
        active_mission: acknowledgement.active_mission,
        revision: acknowledgement.revision,
        known_missions: acknowledgement.missions,
        phase: "workspace-ready",
      };
      setLaunchContext(nextContext);
      setMissionChoiceStatus({
        state: "acknowledged",
        message: acknowledgement.message,
      });
      connect(nextContext);
    },
    [client, connect, launchContext, missionTitle],
  );

  if (launchContext?.phase === "selection-required") {
    return (
      <CodingWorkspaceGate
        launchContext={launchContext}
        workspacePath={codingWorkspacePath}
        status={codingWorkspaceStatus}
        onWorkspacePathChange={setCodingWorkspacePath}
        onSelect={selectCodingWorkspace}
        missionStatus={null}
        missionTitle={missionTitle}
        onMissionTitleChange={setMissionTitle}
        onChooseMission={chooseMission}
      />
    );
  }

  if (launchContext?.phase === "mission-choice-required") {
    return (
      <CodingWorkspaceGate
        launchContext={launchContext}
        workspacePath={codingWorkspacePath}
        status={codingWorkspaceStatus}
        onWorkspacePathChange={setCodingWorkspacePath}
        onSelect={selectCodingWorkspace}
        missionStatus={missionChoiceStatus}
        missionTitle={missionTitle}
        onMissionTitleChange={setMissionTitle}
        onChooseMission={chooseMission}
      />
    );
  }

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
        {state.recoverable ? <button onClick={() => connect()}>Retry connection</button> : null}
      </div>
    );
  }

  return (
    <CommandDeck
      client={client}
      snapshot={state.snapshot}
      empty={state.kind === "empty"}
      actionStatus={actionStatus}
      actionFailure={actionFailure}
      onSelectView={submitView}
      onSwitchMission={submitMissionSwitch}
      connectionStatus={connectionStatus}
      onReconnect={reconnect}
      consoleHistory={consoleHistory}
      consoleHistoryLoadFailure={consoleHistoryLoadFailure}
      onConsoleHistoryRetry={() => void refreshConsoleHistory()}
      draft={draft}
      onDraftChange={setDraft}
      scopeDraft={scopeDraft}
      onScopeDraftChange={setScopeDraft}
      onApplyScope={submitScope}
      scopeActionAvailable={Boolean(client.changeScope && client.loadUpdates)}
      onSend={submitMessage}
      messageStatus={messageStatus}
      messageFailure={messageFailure}
      wayfinder={wayfinder}
      workingContext={workingContext}
      workingContextLoadFailure={workingContextLoadFailure}
      onWorkingContextRetry={() => void refreshWorkingContext()}
      contextStatus={contextStatus}
      contextActionFailure={contextActionFailure}
      onCurateContext={curateWorkingContext}
      reviewWorkspace={reviewWorkspace}
      reviewWorkspaceLoadFailure={reviewWorkspaceLoadFailure}
      onReviewWorkspaceRetry={() => void refreshReviewWorkspace()}
      reviewStatus={reviewStatus}
      reviewReasons={reviewReasons}
      onReviewReasonChange={(sessionId, reason) =>
        setReviewReasons((current) => ({ ...current, [sessionId]: reason }))
      }
      onReviewDecision={submitReviewDecision}
      workspaceQueue={workspaceQueue}
      workspaceQueueLoadFailure={workspaceQueueLoadFailure}
      onWorkspaceQueueRetry={() => void refreshWorkspaceQueue()}
      missionDrafts={missionDrafts}
      missionDraftLoadFailure={missionDraftLoadFailure}
      onMissionDraftRetry={() => void refreshMissionDrafts()}
      missionDraftStatus={missionDraftStatus}
      missionDraftReasons={missionDraftReasons}
      activityJournal={activityJournal}
      activityFilters={activityFilters}
      activityStatus={activityStatus}
      activityLoadFailure={activityLoadFailure}
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
      workstationReviewActionStates={workstationReviewActionStates}
      workstationActionStatusRef={workstationActionStatusRef}
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
      capabilityCatalog={capabilityCatalog}
      capabilityLoadFailure={capabilityLoadFailure}
      onCapabilityRetry={() => void refreshCapabilities()}
      selectedControllerId={effectiveControllerId}
      onControllerChange={selectController}
      consoleTimelineKey={consoleTimelineKey}
      isInitialConsoleTimelineKey={isInitialConsoleTimelineKey}
      timelineOrderForKey={registerTimelineTurn}
    />
  );
}

function CodingWorkspaceGate({
  launchContext,
  workspacePath,
  status,
  onWorkspacePathChange,
  onSelect,
  missionStatus,
  missionTitle,
  onMissionTitleChange,
  onChooseMission,
}: {
  launchContext: AlfredoLaunchContext;
  workspacePath: string;
  status: {
    readonly state: "pending" | "acknowledged" | "rejected";
    readonly message: string;
  } | null;
  onWorkspacePathChange: (path: string) => void;
  onSelect: (selectionMode: "existing" | "create") => void;
  missionStatus: {
    readonly state: "pending" | "acknowledged" | "rejected";
    readonly message: string;
  } | null;
  missionTitle: string;
  onMissionTitleChange: (title: string) => void;
  onChooseMission: (choice: "resume" | "new", option: MissionChoiceOption | null) => void;
}) {
  const selectionRequired = launchContext.phase === "selection-required";
  const capabilityLabel = selectionRequired ? "Coding Workspace" : "Mission";
  return (
    <div className="command-deck">
      <header className="topbar">
        <div className="wordmark">
          <span className="wordmark__signal" aria-hidden="true" />
          <span>ALFREDO</span>
          <small>WORKSTATION</small>
        </div>
        <div className="session-state">
          <span className="eyebrow">Starting Location</span>
          <strong>{launchContext.starting_location}</strong>
        </div>
      </header>
      <div className="deck-grid">
        <main className="prompt-workspace" aria-label="Agent Console">
          <section
            className="prompt-pane"
            aria-label={`${capabilityLabel} selection`}
          >
            <div className="panel-heading">
              <div>
                <span className="eyebrow">Agent Console / {capabilityLabel}</span>
                <small
                  className="console-capability"
                  aria-label={`Responsible capability: ${capabilityLabel}`}
                >
                  Capability: {capabilityLabel}
                </small>
                <h1>
                  {selectionRequired
                    ? "Choose or create a repository"
                    : "Mission selection required"}
                </h1>
              </div>
            </div>
            <div className="console-stage">
              <div className="console-history" role="region" aria-label="Prompt Transcript">
                <p className="system-line">
                  Starting Location
                  <br />
                  <strong>{launchContext.starting_location}</strong>
                </p>
                {selectionRequired ? (
                  <p>
                    No Coding Workspace or Mission is bound. Choose an exact existing Git
                    repository outside Alfredo's protected roots, or create a new one at the
                    backend-provided safe path.
                  </p>
                ) : (
                  <p>
                    Capability: Mission
                    <br />
                    Coding Workspace
                    <br />
                    <strong>{launchContext.coding_workspace}</strong>
                  </p>
                )}
                {status ? (
                  <p
                    role={status.state === "rejected" ? "alert" : "status"}
                    aria-live="polite"
                    className={`status status--${status.state}`}
                  >
                    {status.message}
                  </p>
                ) : null}
                {!selectionRequired && missionStatus ? (
                  <p
                    role={missionStatus.state === "rejected" ? "alert" : "status"}
                    aria-live="polite"
                    className={`status status--${missionStatus.state}`}
                  >
                    {missionStatus.message}
                  </p>
                ) : null}
              </div>
            </div>
            {selectionRequired ? (
              <div className="prompt-composer-dock" role="region" aria-label="Coding Workspace controls">
                <label className="composer prompt-composer">
                  <span>Coding Workspace path</span>
                  <input
                    aria-label="Coding Workspace path"
                    value={workspacePath}
                    disabled={status?.state === "pending"}
                    onChange={(event) => onWorkspacePathChange(event.target.value)}
                  />
                </label>
                <p className="selection-guidance">
                  {launchContext.suggested_workspace_path ? (
                    <>
                      For a new repository, use {launchContext.suggested_workspace_path}. For an
                      existing repository, enter its exact path; it may be outside Starting
                      Location but must not be Alfredo's install, backend, or runtime.
                    </>
                  ) : (
                    "No safe new repository path is configured. Choose an existing repository outside Alfredo's protected roots, or relaunch with a separate Starting Location."
                  )}
                </p>
                <div className="prompt-toolbar">
                  <button
                    type="button"
                    disabled={!workspacePath.trim() || status?.state === "pending"}
                    onClick={() => onSelect("existing")}
                  >
                    Choose existing repository
                  </button>
                  <button
                    type="button"
                    disabled={!workspacePath.trim() || status?.state === "pending"}
                    onClick={() => onSelect("create")}
                  >
                    Create new repository
                  </button>
                </div>
              </div>
            ) : (
              <div className="prompt-composer-dock" role="region" aria-label="Mission controls">
                <div className="mission-choice-list">
                  <p>
                    Choose exactly one existing Mission to continue, or start a distinct new
                    Mission. Mission-qualified work remains blocked until the acknowledgement.
                  </p>
                  {(launchContext.known_missions ?? []).map((mission) => (
                    <button
                      key={mission.id}
                      type="button"
                      disabled={missionStatus?.state === "pending"}
                      onClick={() => onChooseMission("resume", mission)}
                    >
                      Resume Mission: {mission.title}
                    </button>
                  ))}
                </div>
                <label className="composer prompt-composer">
                  <span>New Mission title</span>
                  <input
                    aria-label="New Mission title"
                    value={missionTitle}
                    disabled={missionStatus?.state === "pending"}
                    onChange={(event) => onMissionTitleChange(event.target.value)}
                  />
                </label>
                <div className="prompt-toolbar">
                  <button
                    type="button"
                    disabled={!missionTitle.trim() || missionStatus?.state === "pending"}
                    onClick={() => onChooseMission("new", null)}
                  >
                    Start New Mission
                  </button>
                </div>
              </div>
            )}
          </section>
        </main>
        <aside className="agent-workstations" aria-label="Mission Work">
          <div className="agent-workstations__heading">
            <div>
              <span className="eyebrow">
                {selectionRequired ? "No Mission selected" : "Mission choice required"}
              </span>
              <h2>Mission Work</h2>
            </div>
          </div>
          <div className="mission-work-scroll">
            <p>
              Mission-qualified work remains unavailable until an explicit Mission choice is
              acknowledged.
            </p>
          </div>
        </aside>
      </div>
    </div>
  );
}

interface WorkstationTranscriptTurn {
  readonly id: string;
  readonly content: string;
  readonly source: string;
  readonly outcome: string;
  readonly capability?: CapabilityBoundary;
  readonly causalOriginMessageId?: string;
  readonly receiptCorrelationId?: string;
  readonly receiptPhase?: string;
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

type PromptTimelineEntry =
  | {
      readonly kind: "console";
      readonly key: string;
      readonly order: number;
      readonly canonicalSequence: number | null;
      readonly message: AgentConsoleMessage;
    }
  | {
      readonly kind: "response-pending";
      readonly key: string;
      readonly order: number;
      readonly canonicalSequence: null;
    }
  | {
      readonly kind: "workstation";
      readonly key: string;
      readonly order: number;
      readonly canonicalSequence: number | null;
      readonly turn: WorkstationTranscriptTurn;
    }
  | {
      readonly kind: "command";
      readonly key: string;
      readonly order: number;
      readonly canonicalSequence: null;
      readonly turn: CommandConsoleTurn;
    };

function comparePromptTimelineEntries(
  left: PromptTimelineEntry,
  right: PromptTimelineEntry,
): number {
  if (left.canonicalSequence !== null && right.canonicalSequence !== null) {
    return left.canonicalSequence - right.canonicalSequence;
  }
  if (left.canonicalSequence !== null) return -1;
  if (right.canonicalSequence !== null) return 1;
  return left.order - right.order || left.key.localeCompare(right.key);
}

function promptTimelineVersion(entry: PromptTimelineEntry): string {
  if (entry.kind === "console") {
    return [
      entry.key,
      entry.message.outcome,
      entry.message.content,
      entry.message.correlation_id ?? "",
      entry.message.action_phase ?? "",
      entry.message.action_outcome ?? "",
      entry.message.action_message ?? "",
    ].join(":");
  }
  if (entry.kind === "workstation") {
    return [
      entry.key,
      entry.turn.outcome,
      entry.turn.content,
      entry.turn.receiptCorrelationId ?? "",
      entry.turn.receiptPhase ?? "",
    ].join(":");
  }
  if (entry.kind === "command") {
    return `${entry.key}:${entry.turn.status}:${entry.turn.exitCode ?? "none"}:${entry.turn.summary}`;
  }
  return entry.key;
}

function buildWorkstationTranscriptTurns(
  snapshot: WorkspaceSnapshot,
): readonly WorkstationTranscriptTurn[] {
  const sessionTurns =
    snapshot.missions?.flatMap((mission) =>
      mission.sessions.flatMap((session) => {
        const turns: WorkstationTranscriptTurn[] = [];
        if (session.runner_started_at && session.launch_correlation_id) {
          turns.push({
            id: `session-running:${mission.id}:${session.session_id}`,
            content:
              `Orchestrator started canonical session ${session.session_id} for ` +
              `${session.issue_id} on ${session.assigned_agent}.`,
            source: "orchestrator",
            outcome: "running",
            capability: "Orchestrator",
            receiptCorrelationId: session.launch_correlation_id,
            receiptPhase: "running",
          });
        }
        if (session.evidence_correlation_id) {
          turns.push({
            id: `session-evidence:${mission.id}:${session.session_id}`,
            content:
              `Local Agent ${session.assigned_agent} submitted validated evidence for ` +
              `${session.session_id}.`,
            source: session.assigned_agent,
            outcome: "evidence-ready",
            capability: "Local Agent",
            receiptCorrelationId: session.evidence_correlation_id,
            receiptPhase: "evidence",
          });
        }
        if (session.review_correlation_id && session.review_outcome) {
          turns.push({
            id: `session-review:${mission.id}:${session.session_id}`,
            content: `Review Decision: ${session.review_outcome} for ${session.session_id}.`,
            source: "mission-commander",
            outcome: "review-decision",
            capability: "Mission",
            receiptCorrelationId: session.review_correlation_id,
            receiptPhase: "review-decision",
          });
          if (
            session.review_outcome === "Approved" ||
            session.review_outcome === "Approved with limitations"
          ) {
            turns.push({
              id: `session-completion:${mission.id}:${session.session_id}`,
              content: `Accepted completion: ${session.session_id} is complete.`,
              source: "orchestrator",
              outcome: "accepted-completion",
              capability: "Orchestrator",
              receiptCorrelationId: session.review_correlation_id,
              receiptPhase: "accepted-completion",
            });
          }
        }
        return turns;
      }),
    ) ?? [];
  return sessionTurns;
}

function buildWorkspaceQueueTranscriptTurns(
  snapshot: WorkspaceSnapshot,
  workspaceQueue: WorkspaceQueueProjection | null,
): readonly WorkstationTranscriptTurn[] {
  if (!workspaceQueue) return [];
  return workspaceQueue.items.flatMap((item) => {
    if (item.item_type !== "ad-hoc-delegation" || !item.proposal_correlation_id) {
      return [];
    }
    const causalOriginMessageId =
      typeof item.proposed_changes.originating_message_id === "string" &&
      item.proposed_changes.originating_message_id
        ? item.proposed_changes.originating_message_id
        : undefined;
    const turns: WorkstationTranscriptTurn[] = [
      {
        id: `queue-proposal:${item.mission_id}:${item.item_id}`,
        content:
          item.source === "agent-console"
            ? `Coding task proposal ${item.issue_id} was recorded from Agent Console.`
            : `Coding task proposal ${item.issue_id} was recorded from ${item.source}.`,
        source: "orchestrator",
        outcome: "proposed",
        capability: "Orchestrator",
        causalOriginMessageId,
        receiptCorrelationId: item.proposal_correlation_id || undefined,
        receiptPhase: item.proposal_correlation_id ? "proposal" : undefined,
      },
    ];
    if (item.status !== "pending" && item.decision_correlation_id) {
      const decisionVerb =
        item.status === "approved"
          ? "approved"
          : item.status === "rejected"
            ? "rejected"
            : "deferred";
      turns.push({
        id: `queue-decision:${item.mission_id}:${item.item_id}:${item.status}`,
        content: `Mission Commander ${decisionVerb} coding task ${item.issue_id}.`,
        source: "mission-commander",
        outcome: item.status,
        capability: "Mission",
        causalOriginMessageId,
        receiptCorrelationId: item.decision_correlation_id || undefined,
        receiptPhase: item.decision_correlation_id ? "decision" : undefined,
      });
    }
    if (item.status === "approved" && item.decision_correlation_id) {
      const session = snapshot.missions
        ?.find((mission) => mission.id === item.mission_id)
        ?.sessions.find((candidate) => candidate.issue_id === item.issue_id);
      if (session) {
        turns.push({
          id: `queue-session:${item.mission_id}:${item.item_id}:${session.session_id}`,
          content:
            `Orchestrator queued coding task ${item.issue_id} as ${session.session_id} ` +
            `on ${session.assigned_agent}.`,
          source: "orchestrator",
          outcome: "queued",
          capability: "Orchestrator",
          causalOriginMessageId,
          receiptCorrelationId: item.decision_correlation_id || undefined,
          receiptPhase: item.decision_correlation_id ? "session-queued" : undefined,
        });
      }
    }
    return turns;
  });
}

function actionTurnHasDurableDelegationProjection(
  turn: WorkstationActionTurn,
  snapshot: WorkspaceSnapshot,
  workspaceQueue: WorkspaceQueueProjection | null,
): boolean {
  if (!workspaceQueue) return false;
  return workspaceQueue.items.some((item) => {
    if (item.item_type !== "ad-hoc-delegation") return false;
    const origin = item.proposed_changes.originating_message_id;
    if (typeof origin === "string") {
      const proposalPrefix = `chat-task-${origin}-`;
      const approvalPrefix = `chat-task-approve-${origin}-${item.item_id}-`;
      if (
        (turn.id.startsWith(proposalPrefix) || turn.id.startsWith(approvalPrefix)) &&
        (turn.id.endsWith(":intent") ||
          turn.id.endsWith(":reaction:pending") ||
          turn.id.endsWith(":reaction:acknowledged"))
      ) {
        return true;
      }
    }
    const session = snapshot.missions
      ?.find((mission) => mission.id === item.mission_id)
      ?.sessions.find((candidate) => candidate.issue_id === item.issue_id);
    if (!session) return false;
    const runPrefix = `session-run:${missionSessionIdentity(item.mission_id, session.session_id)}:`;
    return turn.id === `${runPrefix}queued` || turn.id === `${runPrefix}finished`;
  });
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
  if (status === "executing") return "Executing in the governed sandbox";
  if (status === "outcome-unknown") return "Execution started; final outcome is unknown";
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
  if (status === "executing") return "Command is executing in the governed sandbox.";
  if (status === "outcome-unknown") {
    return "Execution started, but its final outcome was not durably recorded; inspect effects before continuing.";
  }
  if (status === "completed") return `Completed${exitCode === null ? "" : ` with exit ${exitCode}`} and no output.`;
  if (status === "failed") return `Failed${exitCode === null ? "" : ` with exit ${exitCode}`} and no captured output.`;
  return "Command did not produce captured output.";
}

function countOutputLines(output: string): number {
  return output.split(/\r?\n/).filter((line) => line.trim()).length;
}

function appendBoundedSessionOutputChunks(
  current: readonly string[],
  incoming: readonly string[],
): readonly string[] {
  const combined = [...current, ...incoming];
  let retainedBytes = 0;
  let firstRetainedIndex = combined.length;
  for (let index = combined.length - 1; index >= 0; index -= 1) {
    const chunkBytes = sessionOutputTextEncoder.encode(combined[index]).byteLength;
    if (retainedBytes + chunkBytes > SESSION_OUTPUT_RENDER_CONTENT_BYTES_LIMIT) break;
    retainedBytes += chunkBytes;
    firstRetainedIndex = index;
  }
  return combined.slice(firstRetainedIndex);
}

function reviewDecisionLabel(decision: ReviewDecision): string {
  if (decision === "accept") return "Accept evidence";
  if (decision === "repair") return "Request repair";
  return "Escalate human review";
}

function reviewDecisionButtonClass(decision: ReviewDecision): string | undefined {
  if (decision === "repair") return "action--warning";
  if (decision === "escalate-human") return "action--danger";
  return undefined;
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
  client,
  snapshot,
  empty,
  actionStatus,
  actionFailure,
  onSelectView,
  onSwitchMission,
  connectionStatus,
  onReconnect,
  consoleHistory,
  consoleHistoryLoadFailure,
  onConsoleHistoryRetry,
  draft,
  onDraftChange,
  scopeDraft,
  onScopeDraftChange,
  onApplyScope,
  scopeActionAvailable,
  onSend,
  messageStatus,
  messageFailure,
  wayfinder,
  workingContext,
  workingContextLoadFailure,
  onWorkingContextRetry,
  contextStatus,
  contextActionFailure,
  onCurateContext,
  reviewWorkspace,
  reviewWorkspaceLoadFailure,
  onReviewWorkspaceRetry,
  reviewStatus,
  reviewReasons,
  onReviewReasonChange,
  onReviewDecision,
  workspaceQueue,
  workspaceQueueLoadFailure,
  onWorkspaceQueueRetry,
  missionDrafts,
  missionDraftLoadFailure,
  onMissionDraftRetry,
  missionDraftStatus,
  missionDraftReasons,
  activityJournal,
  activityFilters,
  activityStatus,
  activityLoadFailure,
  onActivityFilterChange,
  onActivityRefresh,
  queueStatus,
  latestConsoleMessage,
  queueReasons,
  onQueueReasonChange,
  onQueueDecision,
  workstationActionTurns,
  workstationActionState,
  workstationReviewActionStates,
  workstationActionStatusRef,
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
  capabilityCatalog,
  capabilityLoadFailure,
  onCapabilityRetry,
  selectedControllerId,
  onControllerChange,
  consoleTimelineKey,
  isInitialConsoleTimelineKey,
  timelineOrderForKey,
}: {
  client: WorkspaceClient;
  snapshot: WorkspaceSnapshot;
  empty: boolean;
  actionStatus: "pending" | "acknowledged" | "stale" | "rejected" | null;
  actionFailure: string | null;
  onSelectView: (view: string) => void;
  onSwitchMission: (missionId: string) => void;
  connectionStatus: "connected" | "offline" | "reconnecting";
  onReconnect: () => void;
  consoleHistory: readonly AgentConsoleMessage[];
  consoleHistoryLoadFailure: string | null;
  onConsoleHistoryRetry: () => void;
  draft: string;
  onDraftChange: (draft: string) => void;
  scopeDraft: ConversationScope | null;
  onScopeDraftChange: (scope: ConversationScope) => void;
  onApplyScope: (scope?: ConversationScope) => void;
  scopeActionAvailable: boolean;
  onSend: () => void;
  messageStatus: "saving" | "responding" | "rejected" | null;
  messageFailure: string | null;
  wayfinder: WayfinderProjection | null;
  workingContext: WorkingContextProjection | null;
  workingContextLoadFailure: string | null;
  onWorkingContextRetry: () => void;
  contextStatus: "pending" | "acknowledged" | "stale" | "rejected" | null;
  contextActionFailure: string | null;
  onCurateContext: (
    sourceId: string,
    disposition: "included" | "pinned" | "excluded",
  ) => void;
  reviewWorkspace: ReviewWorkspaceProjection | null;
  reviewWorkspaceLoadFailure: string | null;
  onReviewWorkspaceRetry: () => void;
  reviewStatus: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  reviewReasons: Record<string, string>;
  onReviewReasonChange: (sessionId: string, reason: string) => void;
  onReviewDecision: (
    sessionId: string,
    decision: ReviewDecision,
    reason: string,
    missionId?: string,
  ) => void;
  workspaceQueue: WorkspaceQueueProjection | null;
  workspaceQueueLoadFailure: string | null;
  onWorkspaceQueueRetry: () => void;
  missionDrafts: MissionDraftProjection | null;
  missionDraftLoadFailure: string | null;
  onMissionDraftRetry: () => void;
  missionDraftStatus: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  missionDraftReasons: Record<string, string>;
  activityJournal: ActivityJournalProjection | null;
  activityFilters: ActivityJournalFilters;
  activityStatus: "pending" | "rejected" | null;
  activityLoadFailure: string | null;
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
  workstationReviewActionStates: Readonly<Record<string, WorkstationActionState>>;
  workstationActionStatusRef: RefObject<HTMLSpanElement | null>;
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
  capabilityCatalog: AgentCapabilityCatalog | null;
  capabilityLoadFailure: string | null;
  onCapabilityRetry: () => void;
  selectedControllerId: string;
  onControllerChange: (agentId: string) => void;
  consoleTimelineKey: (message: AgentConsoleMessage) => string;
  isInitialConsoleTimelineKey: (key: string) => boolean;
  timelineOrderForKey: (key: string) => number;
}) {
  const mission = snapshot.active_mission;
  const [contextInspectorOpen, setContextInspectorOpen] = useState(false);
  const [detailViewsOpen, setDetailViewsOpen] = useState(false);
  const [capabilityMenuOpen, setCapabilityMenuOpen] = useState(false);
  const [historyIndex, setHistoryIndex] = useState<number | null>(null);
  const [selectedCompletionIndex, setSelectedCompletionIndex] = useState(0);
  const [dismissedCompletion, setDismissedCompletion] = useState<string | null>(null);
  const [relaunchWorkspace, setRelaunchWorkspace] = useState("");
  const [workspaceRelaunchStatus, setWorkspaceRelaunchStatus] = useState<{
    readonly command: string;
    readonly message: string;
  } | null>(null);
  const transcriptRef = useRef<HTMLDivElement>(null);
  const transcriptShouldFollowRef = useRef(true);
  const lastOptimisticTimelineKeyRef = useRef<string | null>(null);
  const capabilityTriggerRef = useRef<HTMLButtonElement>(null);
  const capabilityMenuRef = useRef<HTMLElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const focusCapabilityOptionOnOpenRef = useRef(false);
  const draftBeforeHistoryRef = useRef("");
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
  const recentWorkspaces = Array.from(
    new Set(
      [launchContext?.coding_workspace ?? "", ...(launchContext?.recent_workspaces ?? [])].filter(
        (workspace) => workspace.trim(),
      ),
    ),
  ).slice(0, 10);
  const selectedRelaunchWorkspace = recentWorkspaces.includes(relaunchWorkspace)
    ? relaunchWorkspace
    : recentWorkspaces[0] ?? "";
  const workspaceRelaunchCommand = selectedRelaunchWorkspace
    ? workstationRelaunchCommand(
        selectedRelaunchWorkspace,
        selectedControllerId || launchContext?.selected_agent || "",
      )
    : "";
  const issueSlicesById = new Map(
    snapshot.mission_board.issue_slices?.map((issue) => [issue.issue_id, issue]),
  );
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
          : issueSlicesById.get(issueId)?.title ?? issueId,
    })),
  ];
  const selectedScope = scopeDraft ?? snapshot.conversation_scope;
  const scopeValue = `${selectedScope.kind}:${selectedScope.target_id}`;
  const contextProjection: WorkingContextProjection = workingContext ?? {
    schema_version: 1,
    revision: 0,
    scope: snapshot.conversation_scope,
    sources: [],
    content_character_count: 0,
  };
  const promptHistory = consoleHistory
    .filter((message) => message.role === "user")
    .map((message) => message.content);
  const completionQuery = promptCompletionQuery(draft);
  const completionSignature = completionQuery
    ? `${completionQuery.kind}:${completionQuery.start}:${completionQuery.value}`
    : null;
  const capabilityQuery = draft.trimStart().startsWith("/")
    ? draft.trimStart().slice(1).toLowerCase()
    : "";
  const skillQuery = capabilityQuery.startsWith("use ")
    ? capabilityQuery.slice(4).trim()
    : capabilityQuery.startsWith("skills ")
      ? capabilityQuery.slice(7).trim()
      : capabilityQuery;
  const visibleCommands = capabilityCatalog?.commands.filter((command) => {
    const query = capabilityQuery.split(/\s+/, 1)[0];
    return !query || `${command.name} ${command.description}`.toLowerCase().includes(query);
  }) ?? [];
  const visibleSkills = capabilityCatalog?.skills.filter((skill) =>
    !skillQuery || `${skill.name} ${skill.description}`.toLowerCase().includes(skillQuery),
  ).slice(0, 8) ?? [];
  const completionOptions = capabilityCatalog
    ? capabilityCompletionOptions(completionQuery, capabilityCatalog)
    : [];
  const visibleCapabilities = completionQuery?.kind === "capability"
    ? completionOptions
    : capabilityCatalog
      ? capabilityCompletionOptions(
          { kind: "capability", start: 0, value: "@" },
          capabilityCatalog,
        ).slice(0, 12)
      : [];
  const showCapabilityMenu =
    Boolean(capabilityCatalog) &&
    (capabilityMenuOpen ||
      Boolean(
        completionOptions.length > 0 &&
          completionSignature &&
          completionSignature !== dismissedCompletion,
      ));
  const completionListVisible = showCapabilityMenu && completionOptions.length > 0;
  const closeCapabilityMenu = (focusTarget: "trigger" | "composer" | "none"): void => {
    setCapabilityMenuOpen(false);
    setDismissedCompletion(completionSignature);
    if (focusTarget === "trigger") capabilityTriggerRef.current?.focus();
    if (focusTarget === "composer") composerRef.current?.focus();
  };
  useEffect(() => {
    setSelectedCompletionIndex(0);
  }, [completionSignature]);
  useEffect(() => {
    if (!showCapabilityMenu || !focusCapabilityOptionOnOpenRef.current) return;
    const firstOption = capabilityMenuRef.current?.querySelector<HTMLButtonElement>(
      "button[data-capability-option]:not(:disabled)",
    );
    const fallback = capabilityMenuRef.current?.querySelector<HTMLButtonElement>(
      "button:not(:disabled)",
    );
    (firstOption ?? fallback)?.focus();
    focusCapabilityOptionOnOpenRef.current = false;
  }, [showCapabilityMenu, visibleCommands.length, visibleSkills.length, visibleCapabilities.length]);
  const controllerAgents = capabilityCatalog?.agents.filter(isEligibleControllerCapability) ?? [];
  const selectedController = capabilityCatalog?.agents.find(
    (agent) => agent.id === selectedControllerId,
  );
  const contextCounts = contextProjection.sources.reduce<Record<string, number>>(
    (counts, source) => ({ ...counts, [source.kind]: (counts[source.kind] ?? 0) + 1 }),
    {},
  );
  const issueAssignmentBoard = projectIssueAssignmentBoard(snapshot);
  const workstationContinuityKey = workstationContinuityStorageKey(snapshot);
  const [selectedIssueId, setSelectedIssueId] = useState<string | null>(null);
  const [selectedIssueMissionId, setSelectedIssueMissionId] = useState<string | null>(null);
  const [issueFocusTarget, setIssueFocusTarget] = useState<"assignment-board" | "mission-board" | null>(null);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [selectedSessionMissionId, setSelectedSessionMissionId] = useState<string | null>(null);
  const [selectedWorkstationDiff, setSelectedWorkstationDiff] = useState<WorkstationDiffLink | null>(null);
  const [selectedExecutionNodeId, setSelectedExecutionNodeId] = useState<string | null>(null);
  const [executionOutputChunks, setExecutionOutputChunks] = useState<readonly string[]>([]);
  const [executionOutputState, setExecutionOutputState] =
    useState<MissionExecutionOutputState>("unavailable");
  const [executionOutputTarget, setExecutionOutputTarget] = useState<string | null>(null);
  const executionOutputTargetRef = useRef<string | null>(null);
  const executionOutputSequenceByTargetRef = useRef(new Map<string, number>());
  const [executionOutputFailure, setExecutionOutputFailure] = useState<{
    readonly message: string;
    readonly recoverable: boolean;
    readonly retrying: boolean;
  } | null>(null);
  const [executionOutputRetry, setExecutionOutputRetry] = useState(0);
  const [sessionArtifactViewer, setSessionArtifactViewer] =
    useState<SessionArtifactViewerState | null>(null);
  const artifactLoadSequenceRef = useRef(0);
  const artifactReturnFocusRef = useRef<HTMLElement | null>(null);
  const sessionArtifactViewerRef = useRef<HTMLElement | null>(null);
  const loadSessionArtifact = useCallback(
    async (target: SessionArtifactViewerTarget): Promise<void> => {
      if (target.returnFocus) artifactReturnFocusRef.current = target.returnFocus;
      const sequence = artifactLoadSequenceRef.current + 1;
      artifactLoadSequenceRef.current = sequence;
      setSessionArtifactViewer({
        target,
        status: "loading",
        artifact: null,
        message: "Loading bounded session evidence…",
        recoverable: true,
      });
      if (!client.loadSessionArtifact) {
        setSessionArtifactViewer({
          target,
          status: "error",
          artifact: null,
          message: "The bounded session evidence transport is unavailable.",
          recoverable: false,
        });
        return;
      }
      const result = await client.loadSessionArtifact(target.request);
      if (artifactLoadSequenceRef.current !== sequence) return;
      if (result.kind !== "session-artifact") {
        setSessionArtifactViewer({
          target,
          status: "error",
          artifact: null,
          message: result.message,
          recoverable: result.recoverable,
        });
        return;
      }
      if (
        result.artifact.mission_id !== target.request.mission_id ||
        result.artifact.session_id !== target.request.session_id
      ) {
        setSessionArtifactViewer({
          target,
          status: "error",
          artifact: null,
          message: "The evidence reader returned a mismatched Mission or Local Agent session.",
          recoverable: true,
        });
        return;
      }
      setSessionArtifactViewer({
        target,
        status: "ready",
        artifact: result.artifact,
        message: "",
        recoverable: true,
      });
    },
    [client],
  );
  const openWorkstationDiff = useCallback(
    (diff: WorkstationDiffLink, returnFocus?: HTMLElement | null): void => {
      setSelectedWorkstationDiff(diff);
      void loadSessionArtifact({
        request: {
          mission_id: diff.missionId,
          session_id: diff.sessionId,
          artifact_ref: diff.href.split("#", 1)[0],
        },
        label: diff.label,
        focusPath: diff.path,
        returnFocus,
      });
    },
    [loadSessionArtifact],
  );
  const openWorkstationEvidence = useCallback(
    (
      missionId: string,
      sessionId: string,
      artifactRef: string,
      label: string,
      returnFocus?: HTMLElement | null,
    ): void => {
      void loadSessionArtifact({
        request: {
          mission_id: missionId,
          session_id: sessionId,
          artifact_ref: artifactRef,
        },
        label,
        returnFocus,
      });
    },
    [loadSessionArtifact],
  );
  const closeSessionArtifact = useCallback((): void => {
    artifactLoadSequenceRef.current += 1;
    const returnFocus = artifactReturnFocusRef.current;
    artifactReturnFocusRef.current = null;
    setSessionArtifactViewer(null);
    window.setTimeout(() => returnFocus?.focus(), 0);
  }, []);
  useLayoutEffect(() => {
    if (!sessionArtifactViewer?.target) return;
    const viewer = sessionArtifactViewerRef.current;
    if (!viewer) return;
    viewer.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    viewer.focus({ preventScroll: true });
  }, [sessionArtifactViewer?.target]);
  useEffect(() => {
    if (!selectedIssueId) return;
    const assignmentDetail =
      issueFocusTarget === "assignment-board"
        ? document.getElementById("issue-assignment-detail")
        : null;
    if (assignmentDetail) {
      assignmentDetail.focus();
      return;
    }
    document.getElementById("issue-slice-inspector")?.focus();
  }, [issueFocusTarget, selectedIssueId]);
  const activeMissionId = snapshot.active_mission?.id ?? null;
  const previousActiveMissionIdRef = useRef(activeMissionId);
  useEffect(() => {
    const previousActiveMissionId = previousActiveMissionIdRef.current;
    previousActiveMissionIdRef.current = activeMissionId;
    if (previousActiveMissionId === activeMissionId) return;
    setSelectedIssueId(null);
    setSelectedIssueMissionId(null);
    setIssueFocusTarget(null);
    setSelectedSessionId(null);
    setSelectedSessionMissionId(null);
    setSelectedExecutionNodeId(null);
  }, [activeMissionId]);
  const selectedIssue =
    (selectedIssueId && selectedIssueMissionId === activeMissionId
      ? issueSlicesById.get(selectedIssueId)
      : null) ??
    snapshot.mission_board.issue_slices?.[0] ??
    null;
  const executionTreeProjection: MissionExecutionTreeProjection = useMemo(
    () => projectMissionExecutionTree(snapshot, { workspaceQueue }),
    [snapshot, workspaceQueue],
  );
  const pendingWorkstationIntent = actionStatus === "pending"
    ? {
        id: `workspace-action-${snapshot.revision}`,
        label: "Awaiting Orchestrator acknowledgement",
        expectedRevision: snapshot.revision,
      }
    : null;
  const selectExecutionNode = useCallback(
    (nodeId: string): void => {
      const node = executionTreeProjection.nodes.find((candidate) => candidate.id === nodeId);
      if (!node?.inspectable) return;
      setSelectedExecutionNodeId(nodeId);
    },
    [executionTreeProjection.nodes],
  );
  const selectedExecutionNode = executionTreeProjection.nodes.find(
    (node) => node.id === selectedExecutionNodeId,
  ) ?? null;
  const selectedExecutionOutputTarget =
    selectedExecutionNode?.kind === "agent-session" && selectedExecutionNode.session
      ? `${selectedExecutionNode.card?.missionId ?? activeMissionId}:${selectedExecutionNode.session.session_id}`
      : null;
  useEffect(() => {
    const selectedNode = selectedExecutionNode;
    if (!selectedNode || selectedNode.kind !== "agent-session" || !selectedNode.session) {
      executionOutputTargetRef.current = null;
      executionOutputSequenceByTargetRef.current.clear();
      setExecutionOutputTarget(null);
      setExecutionOutputChunks([]);
      setExecutionOutputState("unavailable");
      setExecutionOutputFailure(null);
      return;
    }
    const missionId = selectedNode.card?.missionId ?? activeMissionId;
    const sessionId = selectedNode.session.session_id;
    if (!missionId || !client.subscribeToSessionOutput) {
      executionOutputTargetRef.current = null;
      executionOutputSequenceByTargetRef.current.clear();
      setExecutionOutputTarget(null);
      setExecutionOutputChunks([]);
      setExecutionOutputState("unavailable");
      setExecutionOutputFailure(null);
      return;
    }
    let active = true;
    const target = `${missionId}:${sessionId}`;
    const targetChanged = executionOutputTargetRef.current !== target;
    executionOutputTargetRef.current = target;
    setExecutionOutputTarget(target);
    if (targetChanged) {
      executionOutputSequenceByTargetRef.current.clear();
      setExecutionOutputChunks([]);
    }
    let lastSequence = executionOutputSequenceByTargetRef.current.get(target) ?? -1;
    setExecutionOutputFailure(null);
    setExecutionOutputState("subscribing");
    let unsubscribe: (() => void) | undefined;
    try {
      unsubscribe = client.subscribeToSessionOutput(
        { mission_id: missionId, session_id: sessionId },
        (event) => {
          if (
            !active ||
            event.mission_id !== missionId ||
            event.session_id !== sessionId ||
            event.sequence <= lastSequence
          ) {
            return;
          }
          lastSequence = event.sequence;
          executionOutputSequenceByTargetRef.current.set(target, lastSequence);
          if (event.content) {
            setExecutionOutputChunks((current) => appendBoundedSessionOutputChunks(current, [event.content]));
          }
        },
        (subscriptionState) => {
          if (!active) return;
          if (subscriptionState.kind === "subscribed") {
            setExecutionOutputFailure(null);
            setExecutionOutputState("subscribed");
            return;
          }
          setExecutionOutputFailure({
            message: subscriptionState.message,
            recoverable: subscriptionState.recoverable,
            retrying: subscriptionState.retrying,
          });
          setExecutionOutputState("failed");
        },
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setExecutionOutputFailure({ message, recoverable: true, retrying: false });
      setExecutionOutputState("unavailable");
    }
    return () => {
      active = false;
      unsubscribe?.();
    };
  }, [
    activeMissionId,
    client,
    executionOutputRetry,
    executionTreeProjection.revision,
    selectedExecutionNode,
  ]);
  const hydratedWorkstationContinuityKey = useRef<string | null>(null);
  const skipWorkstationContinuityWrite = useRef(false);
  useEffect(() => {
    if (hydratedWorkstationContinuityKey.current === workstationContinuityKey) return;
    const restored = readWorkstationContinuity(workstationContinuityKey);
    const sessionIds = new Set(
      (snapshot.missions ?? []).flatMap((candidate) =>
        candidate.id === activeMissionId
          ? candidate.sessions.map((session) => session.session_id)
          : [],
      ),
    );
    const executionNodeIds = new Set(executionTreeProjection.nodes.map((node) => node.id));
    const issueIds = new Set(snapshot.mission_board.ordered_issue_ids);
    if (restored) {
      skipWorkstationContinuityWrite.current = true;
      onCommandAuditOpenChange(restored.commandAuditOpen);
      setSelectedIssueId(
        restored.selectedIssueId &&
          restored.selectedIssueMissionId === activeMissionId &&
          issueIds.has(restored.selectedIssueId)
          ? restored.selectedIssueId
          : null,
      );
      setSelectedIssueMissionId(
        restored.selectedIssueId &&
          restored.selectedIssueMissionId === activeMissionId &&
          issueIds.has(restored.selectedIssueId)
          ? activeMissionId
          : null,
      );
      setIssueFocusTarget(
        restored.selectedIssueId &&
          restored.selectedIssueMissionId === activeMissionId &&
          issueIds.has(restored.selectedIssueId)
          ? restored.issueFocusTarget
          : null,
      );
      setSelectedSessionId(
        restored.selectedSessionId &&
          restored.selectedSessionMissionId === activeMissionId &&
          sessionIds.has(restored.selectedSessionId)
          ? restored.selectedSessionId
          : null,
      );
      setSelectedSessionMissionId(
        restored.selectedSessionId &&
          restored.selectedSessionMissionId === activeMissionId &&
          sessionIds.has(restored.selectedSessionId)
          ? activeMissionId
          : null,
      );
      setSelectedWorkstationDiff(
        restored.selectedWorkstationDiff &&
          sessionIds.has(restored.selectedWorkstationDiff.sessionId)
          ? restored.selectedWorkstationDiff
          : null,
      );
      setSelectedExecutionNodeId(
        restored.selectedExecutionNodeId && executionNodeIds.has(restored.selectedExecutionNodeId)
          ? restored.selectedExecutionNodeId
          : null,
      );
    }
    hydratedWorkstationContinuityKey.current = workstationContinuityKey;
  }, [
    activeMissionId,
    executionTreeProjection.nodes,
    onCommandAuditOpenChange,
    snapshot.mission_board.ordered_issue_ids,
    snapshot.missions,
    workstationContinuityKey,
  ]);
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
      selectedIssueMissionId,
      issueFocusTarget,
      selectedSessionId,
      selectedSessionMissionId,
      selectedWorkstationDiff,
      selectedExecutionNodeId,
    });
  }, [
    commandAuditOpen,
    issueFocusTarget,
    selectedIssueId,
    selectedIssueMissionId,
    selectedSessionId,
    selectedSessionMissionId,
    selectedWorkstationDiff,
    selectedExecutionNodeId,
    workstationContinuityKey,
  ]);
  const snapshotTranscriptTurns = [
    ...buildWorkstationTranscriptTurns(snapshot),
    ...buildWorkspaceQueueTranscriptTurns(snapshot, workspaceQueue),
  ];
  const causalSnapshotTimelinePositions = new Map<string, number>();
  const causalTurnsByOrigin = new Map<string, WorkstationTranscriptTurn[]>();
  for (const turn of snapshotTranscriptTurns) {
    if (!turn.causalOriginMessageId) continue;
    const turns = causalTurnsByOrigin.get(turn.causalOriginMessageId) ?? [];
    turns.push(turn);
    causalTurnsByOrigin.set(turn.causalOriginMessageId, turns);
  }
  for (const [originMessageId, turns] of causalTurnsByOrigin) {
    const originIndex = consoleHistory.findIndex(
      (message) => message.message_id === originMessageId,
    );
    if (originIndex < 0) continue;
    const origin = consoleHistory[originIndex];
    if (!isInitialConsoleTimelineKey(consoleTimelineKey(origin))) continue;
    let anchorIndex = originIndex;
    for (let index = originIndex + 1; index < consoleHistory.length; index += 1) {
      const candidate = consoleHistory[index];
      if (candidate.role === "user") break;
      if (candidate.role === "assistant") {
        anchorIndex = index;
        break;
      }
    }
    const anchor = consoleHistory[anchorIndex];
    const nextMessage = consoleHistory[anchorIndex + 1];
    const availableSequenceSpan =
      nextMessage && nextMessage.sequence > anchor.sequence
        ? nextMessage.sequence - anchor.sequence
        : 1;
    for (const [index, turn] of turns.entries()) {
      causalSnapshotTimelinePositions.set(
        turn.id,
        anchor.sequence + (availableSequenceSpan * (index + 1)) / (turns.length + 1),
      );
    }
  }
  const commandConsoleTurns = buildCommandConsoleTurns(shellTerminal);
  const contextualGrantRequest = shellTerminal.contextualGrantRequest;
  const consoleTimelineEntries: readonly PromptTimelineEntry[] = consoleHistory.map((message) => {
    const key = consoleTimelineKey(message);
    return {
      kind: "console",
      key,
      order: timelineOrderForKey(key),
      canonicalSequence: isInitialConsoleTimelineKey(key) ? message.sequence : null,
      message,
    };
  });
  const snapshotTimelineEntries: readonly PromptTimelineEntry[] = snapshotTranscriptTurns.map(
    (turn) => {
      const key = `workstation-snapshot:${turn.id}`;
      return {
        kind: "workstation",
        key,
        order: timelineOrderForKey(key),
        canonicalSequence: causalSnapshotTimelinePositions.get(turn.id) ?? null,
        turn,
      };
    },
  );
  const actionTimelineEntries: readonly PromptTimelineEntry[] = workstationActionTurns
    .filter(
      (turn) => !actionTurnHasDurableDelegationProjection(
        turn,
        snapshot,
        workspaceQueue,
      ),
    )
    .map((turn) => {
      const key = `workstation-action:${turn.id}`;
      return {
        kind: "workstation",
        key,
        order: timelineOrderForKey(key),
        canonicalSequence: null,
        turn,
      };
    });
  const commandTimelineEntries: readonly PromptTimelineEntry[] = commandConsoleTurns.map((turn) => {
    const key = `command:${turn.commandId}`;
    return {
      kind: "command",
      key,
      order: timelineOrderForKey(key),
      canonicalSequence: null,
      turn,
    };
  });
  const latestConsoleTimelineKey = consoleHistory.length
    ? consoleTimelineKey(consoleHistory[consoleHistory.length - 1])
    : "console:none";
  const responsePendingTimelineEntry: PromptTimelineEntry | null =
    messageStatus === "responding"
      ? {
          kind: "response-pending",
          key: `response-pending:${latestConsoleTimelineKey}`,
          order: timelineOrderForKey(`response-pending:${latestConsoleTimelineKey}`),
          canonicalSequence: null,
        }
      : null;
  const promptTimelineEntries = [
    ...consoleTimelineEntries,
    ...snapshotTimelineEntries,
    ...actionTimelineEntries,
    ...commandTimelineEntries,
    ...(responsePendingTimelineEntry ? [responsePendingTimelineEntry] : []),
  ].sort(comparePromptTimelineEntries);
  const promptTimelineState = promptTimelineEntries.map(promptTimelineVersion).join("\u0000");
  const optimisticConsoleMessage = [...consoleHistory]
    .reverse()
    .find((message) => message.message_id.startsWith("console-pending-"));
  const optimisticTimelineKey = optimisticConsoleMessage
    ? consoleTimelineKey(optimisticConsoleMessage)
    : null;
  const updateComposerDraft = (nextDraft: string): void => {
    setHistoryIndex(null);
    draftBeforeHistoryRef.current = "";
    setDismissedCompletion(null);
    onDraftChange(nextDraft);
  };
  const applyCompletion = (completion: PromptCompletion): void => {
    const query = completionQuery;
    if (!query) return;
    const replacedUntil = query.start + query.value.length;
    const nextCursor = query.start + completion.value.length + 1;
    const nextDraft =
      `${draft.slice(0, query.start)}${completion.value} ${draft.slice(replacedUntil)}`;
    setHistoryIndex(null);
    draftBeforeHistoryRef.current = "";
    setDismissedCompletion(null);
    setCapabilityMenuOpen(false);
    onDraftChange(nextDraft);
    const restoreComposerFocus = (): void => {
      composerRef.current?.focus();
      composerRef.current?.setSelectionRange(nextCursor, nextCursor);
    };
    restoreComposerFocus();
    window.requestAnimationFrame?.(restoreComposerFocus);
  };
  const applyCommandFromCatalog = (command: AgentCapabilityCatalog["commands"][number]): void => {
    if (completionQuery?.kind === "command") {
      applyCompletion({
        value: command.name,
        detail: command.description,
        kind: "command",
        start: completionQuery.start,
      });
      return;
    }
    const takesArgument = /[<[]/.test(command.usage);
    updateComposerDraft(takesArgument ? `${command.name} ` : command.name);
    closeCapabilityMenu("composer");
  };
  const applySkillFromCatalog = (skill: AgentCapabilityCatalog["skills"][number]): void => {
    updateComposerDraft(`${skill.invocation} `);
    closeCapabilityMenu("composer");
  };
  const navigatePromptHistory = (direction: -1 | 1): void => {
    if (promptHistory.length === 0) return;
    if (direction === -1) {
      const nextIndex = historyIndex === null
        ? promptHistory.length - 1
        : Math.max(0, historyIndex - 1);
      if (historyIndex === null) draftBeforeHistoryRef.current = draft;
      setHistoryIndex(nextIndex);
      onDraftChange(promptHistory[nextIndex]);
      setDismissedCompletion(null);
      return;
    }
    if (historyIndex === null) return;
    const nextIndex = historyIndex + 1;
    if (nextIndex >= promptHistory.length) {
      setHistoryIndex(null);
      onDraftChange(draftBeforeHistoryRef.current);
    } else {
      setHistoryIndex(nextIndex);
      onDraftChange(promptHistory[nextIndex]);
    }
    setDismissedCompletion(null);
  };
  const handleSend = (): void => {
    setHistoryIndex(null);
    draftBeforeHistoryRef.current = "";
    onSend();
  };
  const renderPromptCompletion = (completion: PromptCompletion, index: number): ReactElement => (
    <div
      key={completion.value}
      id={`capability-option-${index}`}
      role="option"
      aria-selected={selectedCompletionIndex === index}
    >
      <button
        type="button"
        data-capability-option
        onMouseDown={(event) => event.preventDefault()}
        onClick={() => applyCompletion(completion)}
      >
        <code>{completion.value}</code>
        <span>{completion.detail}</span>
      </button>
    </div>
  );
  useLayoutEffect(() => {
    const transcript = transcriptRef.current;
    if (!transcript) return;
    const hasNewOptimisticTurn =
      optimisticTimelineKey !== null &&
      optimisticTimelineKey !== lastOptimisticTimelineKeyRef.current;
    if (hasNewOptimisticTurn || transcriptShouldFollowRef.current) {
      transcript.scrollTop = transcript.scrollHeight;
      transcriptShouldFollowRef.current = true;
    }
    if (hasNewOptimisticTurn) {
      lastOptimisticTimelineKeyRef.current = optimisticTimelineKey;
    }
  }, [optimisticTimelineKey, promptTimelineState]);
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
          <section className="prompt-pane" aria-label="Agent Console">
            <div className="panel-heading">
              <div>
                <span className="eyebrow">Agent Console / {mission?.id ?? "none"}</span>
                <small
                  className="console-capability"
                  aria-label="Responsible capability: Mission"
                >
                  Capability: Mission
                </small>
                <h1>{mission?.title ?? "No active mission"}</h1>
              </div>
              {connectionStatus === "offline" ? (
                <button type="button" onClick={onReconnect}>Reconnect</button>
              ) : null}
            </div>

            {wayfinder ? (
              <p
                className="wayfinder-route"
                role="status"
                aria-label="Wayfinder route"
                data-wayfinder-mode={wayfinder.mode}
                data-wayfinder-gate={wayfinder.gate.status}
              >
                <strong>
                  Wayfinder / {wayfinder.mode === "chart" ? "Chart" : "Work-through"} mode
                </strong>
                <small
                  className="console-capability"
                  aria-label="Responsible capability: Wayfinder"
                >
                  Capability: Wayfinder
                </small>
                <span>Shared Understanding Gate {wayfinder.gate.status}</span>
                {wayfinder.continuing ? <small>Continuing the durable active flow.</small> : null}
              </p>
            ) : null}

            <div className="console-stage">
            <div
              ref={transcriptRef}
              className="console-history"
              role="region"
              aria-label="Prompt Transcript"
              aria-live="polite"
              onScroll={(event) => {
                const transcript = event.currentTarget;
                const distanceFromBottom =
                  transcript.scrollHeight - transcript.scrollTop - transcript.clientHeight;
                transcriptShouldFollowRef.current = distanceFromBottom <= 80;
              }}
            >
              {promptTimelineEntries.length === 0 ? (
                <p className="system-line">Canonical workspace state restored.</p>
              ) : null}
              {promptTimelineEntries.map((entry) => {
                if (entry.kind === "console") {
                  const capability = capabilityBoundaryLabel(
                    entry.message.source,
                    entry.message.scope,
                  );
                  return (
                    <article
                      key={entry.key}
                      data-timeline-key={entry.key}
                      data-timeline-kind="console"
                      data-outcome={entry.message.outcome}
                      data-authority={
                        entry.message.outcome === "model-commentary"
                          ? "commentary"
                          : entry.message.correlation_id
                            ? "canonical-receipt"
                            : "canonical-record"
                      }
                    >
                      {entry.message.outcome === "model-commentary" ? (
                        <p className="console-authority-note" role="status">
                          {entry.message.action_message ||
                            "Controller commentary only. No Orchestrator action receipt is attached."}
                        </p>
                      ) : null}
                      <p>{entry.message.content}</p>
                      <small
                        className="console-attribution"
                        aria-label={`Responsible capability: ${capability}`}
                      >
                        Capability: {capability}
                      </small>
                      <small>{entry.message.source} / {entry.message.outcome}</small>
                      {entry.message.correlation_id && entry.message.action_phase ? (
                        <small className="console-receipt-identity">
                          Receipt {entry.message.correlation_id} · {entry.message.action_phase}
                        </small>
                      ) : null}
                    </article>
                  );
                }
                if (entry.kind === "response-pending") {
                  return (
                    <article
                      key={entry.key}
                      className="console-response-pending"
                      data-timeline-key={entry.key}
                      data-timeline-kind="response-pending"
                      data-outcome="pending"
                      aria-live="polite"
                    >
                      <p>Alfredo is working…</p>
                      <small>local controller / responding</small>
                    </article>
                  );
                }
                if (entry.kind === "workstation") {
                  const capability =
                    entry.turn.capability ??
                    capabilityBoundaryLabel(entry.turn.source, snapshot.conversation_scope);
                  return (
                    <article
                      key={entry.key}
                      data-timeline-key={entry.key}
                      data-timeline-kind="workstation"
                      data-outcome={entry.turn.outcome}
                      data-authority={
                        entry.turn.receiptCorrelationId
                          ? "canonical-receipt"
                          : "canonical-record"
                      }
                    >
                      <p>{entry.turn.content}</p>
                      <small
                        className="console-attribution"
                        aria-label={`Responsible capability: ${capability}`}
                      >
                        Capability: {capability}
                      </small>
                      <small>{entry.turn.source} / {entry.turn.outcome}</small>
                      {entry.turn.receiptCorrelationId && entry.turn.receiptPhase ? (
                        <small className="console-receipt-identity">
                          Receipt {entry.turn.receiptCorrelationId} · {entry.turn.receiptPhase}
                        </small>
                      ) : null}
                    </article>
                  );
                }
                return (
                  <CommandConsoleCard
                    key={entry.key}
                    timelineKey={entry.key}
                    turn={entry.turn}
                    actionPending={shellTerminal.actionStatus?.state === "pending"}
                    denialReason={shellTerminal.denialReasons[entry.turn.commandId] ?? ""}
                    onDenialReasonChange={(reason) =>
                      shellTerminal.setDenialReason(entry.turn.commandId, reason)
                    }
                    onDecide={(decision) => {
                      if (entry.turn.record) void shellTerminal.decide(entry.turn.record, decision);
                    }}
                  />
                );
              })}
              {contextualGrantRequest ? (
                <PathGrantConsolePrompt
                  request={contextualGrantRequest}
                  actionPending={shellTerminal.actionStatus?.state === "pending"}
                  onGrant={() =>
                    void shellTerminal.createGrantForRequest(contextualGrantRequest.requestId)
                  }
                  onDeny={() =>
                    void shellTerminal.denyGrantRequest(contextualGrantRequest.requestId)
                  }
                />
              ) : null}
            </div>

            {contextInspectorOpen ? (
              <section className="context-inspector" aria-label="Context Inspector">
                <div className="context-inspector__heading">
                  <div>
                    <span className="eyebrow">Bounded model input</span>
                    <h3>Context Inspector</h3>
                  </div>
                  <code>
                    {workingContext
                      ? `${contextProjection.content_character_count} / 4000 chars`
                      : "Context unavailable"}
                  </code>
                </div>
                {workingContextLoadFailure ? (
                  <div role="alert" className="inline-failure">
                    <span>{`Working Context load failed: ${workingContextLoadFailure}`}</span>
                    <button type="button" onClick={onWorkingContextRetry}>
                      Retry Working Context
                    </button>
                  </div>
                ) : null}
                {workingContext ? (
                  <div className="context-inspector__counts">
                    {Object.entries(contextCounts).map(([kind, count]) => (
                      <small key={kind}>{count} {kind} {count === 1 ? "source" : "sources"}</small>
                    ))}
                  </div>
                ) : null}
                {workingContext ? (
                  <div className="context-inspector__sources">
                    {contextProjection.sources.map((source) => (
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
                ) : workingContextLoadFailure ? null : (
                  <p role="status">Loading Working Context…</p>
                )}
                {contextStatus ? (
                  <span role="status" aria-label="Context curation status">
                    {contextStatus[0].toUpperCase() + contextStatus.slice(1)}
                    {contextActionFailure ? `: ${contextActionFailure}` : ""}
                  </span>
                ) : null}
                <div className="context-target-control">
                  <label>
                    <span>Conversation target</span>
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
                        <option
                          key={`${scope.kind}:${scope.target_id}`}
                          value={`${scope.kind}:${scope.target_id}`}
                        >
                          {scope.kind === "working-directory" ? "Working directory" : scope.label}
                        </option>
                      ))}
                    </select>
                  </label>
                  <button
                    type="button"
                    onClick={() => onApplyScope()}
                    disabled={
                      !scopeActionAvailable ||
                      !scopeDraft ||
                      (scopeDraft.kind === snapshot.conversation_scope.kind &&
                        scopeDraft.target_id === snapshot.conversation_scope.target_id)
                    }
                  >
                    Apply target
                  </button>
                </div>
              </section>
            ) : null}
            </div>

            <div className="prompt-composer-dock" role="region" aria-label="Prompt Composer">
              {showCapabilityMenu ? (
                <section
                  ref={capabilityMenuRef}
                  className="capability-menu"
                  aria-label="Commands and skills"
                  onKeyDown={(event) => {
                    if (event.key !== "Escape") return;
                    event.preventDefault();
                    closeCapabilityMenu("trigger");
                  }}
                >
                  <div className="capability-menu__heading">
                    <div>
                      <span className="eyebrow">Coding-agent controls</span>
                      <strong>Commands & skills</strong>
                    </div>
                    <button
                      type="button"
                      aria-label="Close commands and skills"
                      onClick={() => closeCapabilityMenu("trigger")}
                    >Close</button>
                  </div>
                  <div
                    className="capability-menu__groups"
                    id="prompt-completions"
                    role="listbox"
                    aria-label="Prompt completions"
                    aria-activedescendant={
                      completionOptions.length > 0
                        ? `capability-option-${selectedCompletionIndex}`
                        : undefined
                    }
                  >
                    {completionQuery?.kind === "capability" ? (
                      <div>
                        <small>Capabilities</small>
                        {visibleCapabilities.map(renderPromptCompletion)}
                      </div>
                    ) : completionQuery?.kind === "command" ? (
                      <div>
                        <small>Commands</small>
                        {completionOptions.map(renderPromptCompletion)}
                      </div>
                    ) : (
                      <>
                        <div>
                          <small>Commands</small>
                          {visibleCommands.map((command) => (
                            <div key={command.name} role="option">
                              <button
                                type="button"
                                data-capability-option
                                onClick={() => applyCommandFromCatalog(command)}
                              >
                                <code>{command.usage}</code>
                                <span>{command.description}</span>
                              </button>
                            </div>
                          ))}
                        </div>
                        <div>
                          <small>Installed skills</small>
                          {visibleSkills.length > 0 ? visibleSkills.map((skill) => (
                            <div key={skill.name} role="option">
                              <button
                                type="button"
                                data-capability-option
                                onClick={() => applySkillFromCatalog(skill)}
                              >
                                <code>${skill.name}</code>
                                <span>{skill.description}</span>
                              </button>
                            </div>
                          )) : <span className="capability-menu__empty">No matching skills</span>}
                        </div>
                      </>
                    )}
                  </div>
                </section>
              ) : null}
              {capabilityLoadFailure || consoleHistoryLoadFailure ? (
                <div role="alert" aria-label="Agent Console load recovery">
                  {capabilityLoadFailure ? (
                    <div>
                      <p>Capability catalog load failed: {capabilityLoadFailure}</p>
                      <button type="button" onClick={onCapabilityRetry}>
                        Retry capability catalog
                      </button>
                    </div>
                  ) : null}
                  {consoleHistoryLoadFailure ? (
                    <div>
                      <p>Console history load failed: {consoleHistoryLoadFailure}</p>
                      <button type="button" onClick={onConsoleHistoryRetry}>
                        Retry console history
                      </button>
                    </div>
                  ) : null}
                </div>
              ) : null}
              <div className="prompt-toolbar" aria-label="Prompt controls">
                <button
                  ref={capabilityTriggerRef}
                  type="button"
                  aria-label="Browse commands and skills"
                  aria-expanded={showCapabilityMenu}
                  disabled={!capabilityCatalog}
                  title={capabilityCatalog ? "Browse installed commands and skills" : "Capability catalog unavailable"}
                  onClick={() => {
                    if (showCapabilityMenu) {
                      closeCapabilityMenu("none");
                    } else {
                      setContextInspectorOpen(false);
                      focusCapabilityOptionOnOpenRef.current = true;
                      setCapabilityMenuOpen(true);
                      setDismissedCompletion(null);
                    }
                  }}
                >
                  Commands <kbd>/</kbd>
                </button>
                <button
                  type="button"
                  aria-label={contextInspectorOpen ? "Close context" : "Inspect context"}
                  aria-expanded={contextInspectorOpen}
                  onClick={() => {
                    if (!contextInspectorOpen) {
                      setCapabilityMenuOpen(false);
                      setDismissedCompletion(completionSignature);
                    }
                    setContextInspectorOpen((open) => !open);
                  }}
                >
                  {contextInspectorOpen ? "Close context" : `Context · ${snapshot.conversation_scope.label}`}
                </button>
              </div>
              <div className="prompt-status-line" aria-label="Prompt status line">
                <span role="status" aria-label="Connection status">
                  Connection {connectionStatus[0].toUpperCase() + connectionStatus.slice(1)}
                </span>
                {controllerAgents.length > 0 ? (
                  <label className="controller-picker">
                    <span>Controller</span>
                    <select
                      aria-label="Controller model"
                      value={selectedControllerId}
                      title={selectedController?.availability_reason || "Select the local controller model"}
                      onChange={(event) => onControllerChange(event.target.value)}
                    >
                      {controllerAgents.map((agent) => (
                        <option key={agent.id} value={agent.id} disabled={agent.availability !== "available"}>
                          {agent.id} · {agent.model || agent.runner}
                        </option>
                      ))}
                    </select>
                  </label>
                ) : (
                  <span>
                    Controller {launchContext?.selected_agent || "default"} · {launchContext?.selected_model || "default"}
                  </span>
                )}
                <span title={snapshot.workspace_session.workspace_path}>Workspace {workingDirectoryLabel}</span>
                <span role="status" aria-label="Execution status" aria-live="polite">
                  Execution {activeExecutionState}
                </span>
              </div>
              {workspaceRelaunchCommand ? (
                <section className="prompt-status-line" aria-label="Recent workspaces">
                  <label className="controller-picker">
                    <span>Recent workspace</span>
                    <select
                      aria-label="Workspace to relaunch"
                      value={selectedRelaunchWorkspace}
                      onChange={(event) => {
                        setRelaunchWorkspace(event.target.value);
                        setWorkspaceRelaunchStatus(null);
                      }}
                    >
                      {recentWorkspaces.map((workspace) => (
                        <option key={workspace} value={workspace}>
                          {workspace}
                        </option>
                      ))}
                    </select>
                  </label>
                  <input
                    type="text"
                    aria-label="Workspace relaunch command"
                    readOnly
                    value={workspaceRelaunchCommand}
                    onFocus={(event) => event.currentTarget.select()}
                  />
                  <button
                    type="button"
                    aria-label="Copy workspace relaunch command"
                    onClick={() => {
                      const copiedCommand = workspaceRelaunchCommand;
                      if (!navigator.clipboard?.writeText) {
                        setWorkspaceRelaunchStatus({
                          command: copiedCommand,
                          message: "Clipboard unavailable. Select the command and run it in a terminal.",
                        });
                        return;
                      }
                      void navigator.clipboard.writeText(copiedCommand).then(
                        () =>
                          setWorkspaceRelaunchStatus({
                            command: copiedCommand,
                            message: "Relaunch command copied.",
                          }),
                        () =>
                          setWorkspaceRelaunchStatus({
                            command: copiedCommand,
                            message: "Copy failed. Select the command and run it in a terminal.",
                          }),
                      );
                    }}
                  >
                    Copy relaunch command
                  </button>
                  <span>
                    Run in a terminal to open a separate workstation; this session stays on {workingDirectoryLabel}.
                  </span>
                  {workspaceRelaunchStatus?.command === workspaceRelaunchCommand ? (
                    <span role="status" aria-label="Workspace relaunch status" aria-live="polite">
                      {workspaceRelaunchStatus.message}
                    </span>
                  ) : null}
                </section>
              ) : null}
              <label className="composer prompt-composer">
                <span className="sr-only">Message Alfredo</span>
                <textarea
                  ref={composerRef}
                  aria-label="Message Alfredo"
                  aria-autocomplete="list"
                  aria-controls={completionListVisible ? "prompt-completions" : undefined}
                  aria-expanded={completionListVisible ? showCapabilityMenu : undefined}
                  aria-activedescendant={
                    completionListVisible
                      ? `capability-option-${selectedCompletionIndex}`
                      : undefined
                  }
                  placeholder="Ask about the project, type / for commands, or create a task…"
                  maxLength={AGENT_CONSOLE_USER_CONTENT_CHARACTER_LIMIT}
                  rows={3}
                  value={draft}
                  onChange={(event) => {
                    const nextDraft = event.target.value;
                    const startsFreshSlash =
                      nextDraft.trimStart() === "/" ||
                      (!draft.trimStart().startsWith("/") && nextDraft.trimStart().startsWith("/"));
                    updateComposerDraft(nextDraft);
                    if (startsFreshSlash) setContextInspectorOpen(false);
                  }}
                  onKeyDown={(event) => {
                    const completionIsOpen = showCapabilityMenu && completionOptions.length > 0;
                    if (completionIsOpen && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
                      event.preventDefault();
                      const offset = event.key === "ArrowDown" ? 1 : -1;
                      setSelectedCompletionIndex((current) =>
                        (current + offset + completionOptions.length) % completionOptions.length,
                      );
                      return;
                    }
                    if (
                      completionIsOpen &&
                      (event.key === "Tab" ||
                        (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing))
                    ) {
                      event.preventDefault();
                      applyCompletion(completionOptions[selectedCompletionIndex] ?? completionOptions[0]);
                      return;
                    }
                    if (event.key === "Escape" && showCapabilityMenu) {
                      event.preventDefault();
                      closeCapabilityMenu("composer");
                      return;
                    }
                    const historyDirection =
                      event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : null;
                    if (
                      historyDirection !== null &&
                      !event.altKey &&
                      !event.ctrlKey &&
                      !event.metaKey &&
                      !event.shiftKey &&
                      promptHistory.length > 0 &&
                      canNavigatePromptHistory(
                        historyDirection,
                        historyIndex,
                        draft,
                        event.currentTarget.selectionStart,
                        event.currentTarget.selectionEnd,
                      )
                    ) {
                      event.preventDefault();
                      navigatePromptHistory(historyDirection);
                      return;
                    }
                    if (
                      event.key === "Enter" &&
                      !event.shiftKey &&
                      !event.nativeEvent.isComposing
                    ) {
                      event.preventDefault();
                      handleSend();
                    }
                  }}
                />
                <button
                  type="button"
                  aria-label="Send prompt"
                  disabled={
                    !draft.trim() || messageStatus === "saving" || messageStatus === "responding"
                  }
                  onClick={handleSend}
                >
                  Send
                </button>
                {messageStatus ? (
                  <span role="status" aria-label="Message status">
                    {messageStatus === "saving"
                      ? "Saving prompt…"
                      : messageStatus === "responding"
                        ? "Controller responding…"
                        : messageFailure
                          ? `Prompt was not saved: ${messageFailure} Prompt restored; retry after correcting the reported boundary.`
                          : "Response failed — prompt restored"}
                  </span>
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
            <div className="mission-work-controls">
              <span className="connection-pill">
                {executionTreeProjection.counts.local_agent_sessions} Local Agent sessions
              </span>
              <button
                type="button"
                aria-expanded={commandAuditOpen}
                onClick={() => onCommandAuditOpenChange(!commandAuditOpen)}
              >
                {commandAuditOpen ? "Close command audit" : "Open command audit"}
              </button>
              <button
                type="button"
                aria-expanded={detailViewsOpen}
                onClick={() => setDetailViewsOpen((open) => !open)}
              >
                {detailViewsOpen ? "Close detail views" : "Open detail views"}
              </button>
            </div>
          </div>
          <div className="mission-work-scroll">
            <MissionExecutionTree
              projection={executionTreeProjection}
              selectedNodeId={selectedExecutionNodeId}
              onSelectNode={selectExecutionNode}
              onCloseInspector={() => setSelectedExecutionNodeId(null)}
              outputLines={
                executionOutputTarget === selectedExecutionOutputTarget ? executionOutputChunks : []
              }
              outputState={
                executionOutputTarget === selectedExecutionOutputTarget
                  ? executionOutputState
                  : "unavailable"
              }
              outputFailure={
                executionOutputTarget === selectedExecutionOutputTarget
                  ? executionOutputFailure
                  : null
              }
              onRetryOutput={() => setExecutionOutputRetry((current) => current + 1)}
              onOpenDiff={openWorkstationDiff}
              onOpenEvidence={openWorkstationEvidence}
              workstationActionDrafts={workstationActionDrafts}
              onWorkstationActionDraftChange={onWorkstationActionDraftChange}
              onWorkstationAction={onWorkstationAction}
              actionState={workstationActionState}
              actionStates={workstationReviewActionStates}
              reviewReasons={reviewReasons}
              onReviewReasonChange={onReviewReasonChange}
              onReviewDecision={onReviewDecision}
              queueReasons={queueReasons}
              onQueueReasonChange={onQueueReasonChange}
              onQueueDecision={onQueueDecision}
              onOpenView={onSelectView}
              agentOptions={capabilityCatalog?.agents.filter(isEligibleWorkerCapability) ?? []}
            />
            {sessionArtifactViewer ? (
              <SessionArtifactViewer
                viewerRef={sessionArtifactViewerRef}
                state={sessionArtifactViewer}
                onRetry={() => void loadSessionArtifact(sessionArtifactViewer.target)}
                onClose={closeSessionArtifact}
              />
            ) : selectedWorkstationDiff ? (
              <div className="workstation-local-selection">
                <span>Saved review diff: {selectedWorkstationDiff.path}</span>
                <small>{selectedWorkstationDiff.sessionId}</small>
                <button
                  type="button"
                  onClick={(event) => openWorkstationDiff(selectedWorkstationDiff, event.currentTarget)}
                >
                  Load saved review diff
                </button>
              </div>
            ) : null}
            {pendingWorkstationIntent ? (
              <div className="workstation-pending" role="status" aria-label="Pending workstation intent">
                <span>{pendingWorkstationIntent.label}</span>
                <small>Expected revision {pendingWorkstationIntent.expectedRevision}</small>
              </div>
            ) : null}
          {detailViewsOpen && snapshot.operations_view === "workspace-queue" ? null : <IssueAssignmentBoard
            projection={issueAssignmentBoard}
            selectedIssueId={
              issueFocusTarget === "assignment-board" && selectedIssueMissionId === activeMissionId
                ? selectedIssueId
                : null
            }
            agentOptions={capabilityCatalog?.agents.filter(isEligibleWorkerCapability) ?? []}
            workstationActionState={workstationActionState}
            workstationActionDrafts={workstationActionDrafts}
            workstationActionStatusRef={workstationActionStatusRef}
            onSelectIssue={(issueId) => {
              setSelectedIssueId(issueId);
              setSelectedIssueMissionId(activeMissionId);
              setIssueFocusTarget("assignment-board");
              setSelectedSessionId(null);
              setSelectedSessionMissionId(null);
            }}
            onWorkstationActionDraftChange={onWorkstationActionDraftChange}
            onWorkstationAction={onWorkstationAction}
          />}
          {detailViewsOpen || commandAuditOpen ? (
          <div className="workstation-panel">
            {detailViewsOpen ? (
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
                    {actionFailure ? `: ${actionFailure}` : ""}
                  </span>
                ) : null}

                <section className="mission-surface">
                  {snapshot.operations_view === "review-workspace" ? (
                    <ReviewWorkspace
                      projection={reviewWorkspace}
                      loadFailure={reviewWorkspaceLoadFailure}
                      onRetry={onReviewWorkspaceRetry}
                      status={reviewStatus}
                      reasons={reviewReasons}
                      onReasonChange={onReviewReasonChange}
                      onDecision={onReviewDecision}
                      onOpenEvidence={openWorkstationEvidence}
                    />
                  ) : snapshot.operations_view === "workspace-queue" ? (
                    <WorkspaceQueue
                      projection={workspaceQueue}
                      loadFailure={workspaceQueueLoadFailure}
                      onRetry={onWorkspaceQueueRetry}
                      missionDrafts={missionDrafts}
                      missionDraftLoadFailure={missionDraftLoadFailure}
                      onMissionDraftRetry={onMissionDraftRetry}
                      missionDraftStatus={missionDraftStatus}
                      missionDraftReasons={missionDraftReasons}
                      status={queueStatus}
                      reasons={queueReasons}
                      onReasonChange={onQueueReasonChange}
                      onDecision={onQueueDecision}
                      onMissionDraftReasonChange={onMissionDraftReasonChange}
                      onMissionDraftDecision={onMissionDraftDecision}
                    />
                  ) : snapshot.operations_view === "activity" ? (
                    <ActivityJournal
                      projection={activityJournal}
                      filters={activityFilters}
                      status={activityStatus}
                      loadFailure={activityLoadFailure}
                      onFilterChange={onActivityFilterChange}
                      onRefresh={onActivityRefresh}
                      fallbackMissionId={mission?.id ?? ""}
                      onOpenEvidence={openWorkstationEvidence}
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
                                      href={`#${attention.queue_item_id || attention.attention_id}`}
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
                                        setSelectedIssueMissionId(activeMissionId);
                                        setIssueFocusTarget("mission-board");
                                        setSelectedSessionId(null);
                                        setSelectedSessionMissionId(null);
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
                              selectedSessionId={
                                selectedSessionMissionId === activeMissionId
                                  ? selectedSessionId
                                  : null
                              }
                              onSelectSession={(sessionId) => {
                                setSelectedSessionId(sessionId);
                                setSelectedSessionMissionId(activeMissionId);
                              }}
                            />
                          ) : null}
                        </div>
                      )}
                    </>
                  )}
                </section>
            </section>
            ) : null}
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
          ) : null}
          </div>
        </aside>
      </div>
    </div>
  );
}

function IssueAssignmentBoard({
  projection,
  selectedIssueId,
  agentOptions,
  workstationActionState,
  workstationActionDrafts,
  workstationActionStatusRef,
  onSelectIssue,
  onWorkstationActionDraftChange,
  onWorkstationAction,
}: {
  projection: IssueAssignmentBoardProjection;
  selectedIssueId: string | null;
  agentOptions: readonly { readonly id: string; readonly model: string }[];
  workstationActionState: WorkstationActionState | null;
  workstationActionDrafts: Record<string, WorkstationActionDraftState>;
  workstationActionStatusRef: RefObject<HTMLSpanElement | null>;
  onSelectIssue: (issueId: string) => void;
  onWorkstationActionDraftChange: (key: string, draft: WorkstationActionDraftState) => void;
  onWorkstationAction: (
    action: WorkstationGovernedAction,
    draft: WorkstationActionDraftState,
  ) => void;
}): ReactElement {
  const selectedRow =
    projection.rows.find((row) => row.issueId === selectedIssueId) ?? null;
  return (
    <section className="issue-assignment-board" aria-label="Issue Assignment Board">
      <div className="mission-work-section-heading">
        <div>
          <span className="eyebrow">Ownership matrix</span>
          <h3>Issue Assignment Board</h3>
        </div>
        <span className="connection-pill">{projection.rows.length} issues</span>
      </div>
      <div className="issue-assignment-board__scroll">
        <table aria-label="Issue Assignment Board">
          <thead>
            <tr>
              <th scope="col">Issue</th>
              <th scope="col">Owner</th>
              <th scope="col">State</th>
              <th scope="col">Blockers</th>
              <th scope="col">Workstation</th>
              <th scope="col">Actions</th>
            </tr>
          </thead>
          <tbody>
            {projection.rows.map((row) => {
              const rowActions = row.governedActions.filter(isExecutableWorkstationAction);
              const rowActionState =
                workstationActionState &&
                rowActions.some(
                  (action) => workstationActionStateId(action) === workstationActionState.itemId,
                )
                  ? workstationActionState
                  : null;
              return (
                <tr
                  key={row.issueId}
                  aria-label={`${row.issueId} ${row.title} ${row.owner} ${row.readinessState} ${
                    row.blockerSummaries.length ? row.blockerSummaries.join(", ") : "Clear"
                  } ${row.workstationSessionId ? `Session ${row.workstationSessionId}` : "No workstation"}`}
                  data-selected={selectedIssueId === row.issueId}
                  data-state={row.state}
                >
                  <td data-label="Issue">
                    <button
                      type="button"
                      aria-label={`Inspect assignment ${row.issueId}`}
                      onClick={() => onSelectIssue(row.issueId)}
                    >
                      {row.issueId}
                    </button>
                    <small>{row.title}</small>
                  </td>
                  <td data-label="Owner">
                    <strong>{row.owner}</strong>
                    <small>{row.assignmentState}</small>
                  </td>
                  <td data-label="State">
                    <span className="status">{row.readinessState}</span>
                    <small>{row.lifecycleState}</small>
                  </td>
                  <td data-label="Blockers">
                    {row.blockerSummaries.length === 0 ? (
                      <span>Clear</span>
                    ) : (
                      row.blockerSummaries.map((summary) => (
                        <small key={summary}>{summary}</small>
                      ))
                    )}
                  </td>
                  <td data-label="Workstation">
                    {row.workstationSessionId ? (
                      <>
                        <strong>Session {row.workstationSessionId}</strong>
                        <small>{row.workstationAgent ?? "Unassigned workstation"}</small>
                        {row.workstationStatus ? <small>{row.workstationStatus}</small> : null}
                      </>
                    ) : (
                      <span>No workstation</span>
                    )}
                  </td>
                  <td data-label="Actions">
                    {rowActionState ? (
                      <small
                        ref={workstationActionStatusRef}
                        role={
                          rowActionState.state === "rejected" || rowActionState.state === "failed"
                            ? "alert"
                            : "status"
                        }
                        aria-label={`${row.issueId} issue assignment action state`}
                        className="connection-pill"
                        tabIndex={-1}
                      >
                        {rowActionState.state}: {rowActionState.message}
                      </small>
                    ) : null}
                    <IssueAssignmentActions
                      actions={rowActions}
                      actionState={rowActionState}
                      drafts={workstationActionDrafts}
                      agentOptions={agentOptions}
                      onDraftChange={onWorkstationActionDraftChange}
                      onAction={onWorkstationAction}
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {selectedRow ? <IssueAssignmentDetail row={selectedRow} /> : null}
    </section>
  );
}

function IssueAssignmentActions({
  actions,
  actionState,
  drafts,
  agentOptions,
  onDraftChange,
  onAction,
}: {
  actions: readonly WorkstationGovernedAction[];
  actionState: WorkstationActionState | null;
  drafts: Record<string, WorkstationActionDraftState>;
  agentOptions: readonly { readonly id: string; readonly model: string }[];
  onDraftChange: (key: string, draft: WorkstationActionDraftState) => void;
  onAction: (action: WorkstationGovernedAction, draft: WorkstationActionDraftState) => void;
}): ReactElement | null {
  if (actions.length === 0) return null;
  const pending = actionState?.state === "pending";
  return (
    <div className="issue-assignment-board__actions">
      {actions.map((action) => {
        const targetId = workstationActionTargetId(action);
        const key = workstationActionKey(action);
        const draft = drafts[key] ?? { reason: "", agentId: "" };
        const needsAgent = action.actionType === "model-assignment-change";
        const hasEligibleAgent = agentOptions.some((agent) => agent.id === draft.agentId);
        const disabledDescription = workstationActionDisabledDescription(
          action,
          targetId,
          draft,
          pending,
        );
        const helpId = disabledDescription ? `${workstationDomId(`issue-board:${key}`)}-help` : undefined;
        const disabled =
          pending ||
          Boolean(action.disabledReason) ||
          !targetId ||
          (action.requiresReason && !draft.reason.trim()) ||
          (needsAgent && !hasEligibleAgent);
        return (
          <div className="issue-assignment-board__action" key={key}>
            {needsAgent ? (
              <label>
                <span>Agent</span>
                {agentOptions.length > 0 ? (
                  <select
                    aria-label={`Issue assignment agent ${targetId}`}
                    value={draft.agentId}
                    onChange={(event) =>
                      onDraftChange(key, { ...draft, agentId: event.target.value })
                    }
                  >
                    <option value="">Select a local agent</option>
                    {agentOptions.map((agent) => (
                      <option key={agent.id} value={agent.id}>
                        {agent.id} · {agent.model || "local runner"}
                      </option>
                    ))}
                  </select>
                ) : (
                  <span
                    role="status"
                    aria-label={`Issue assignment worker unavailable ${targetId}`}
                  >
                    Worker assignment unavailable: no eligible local workers were discovered.
                  </span>
                )}
              </label>
            ) : null}
            {action.requiresReason ? (
              <label>
                <span>Reason</span>
                <textarea
                  aria-label={`Issue assignment reason ${targetId}`}
                  rows={2}
                  value={draft.reason}
                  onChange={(event) =>
                    onDraftChange(key, {
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
              onClick={() => onAction(action, draft)}
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
  );
}

function IssueAssignmentDetail({ row }: { row: IssueAssignmentBoardRow }): ReactElement {
  return (
    <section
      id="issue-assignment-detail"
      className="issue-assignment-detail"
      aria-label="Issue Assignment Detail"
      tabIndex={-1}
    >
      <div className="issue-inspector__heading">
        <div>
          <span className="eyebrow">Local issue detail</span>
          <h4>{row.issueId}</h4>
        </div>
        <span className="status">{row.readinessState}</span>
      </div>
      <p>{row.title}</p>
      <dl>
        <div>
          <dt>Owner</dt>
          <dd>{row.owner}</dd>
        </div>
        <div>
          <dt>Assignment</dt>
          <dd>{row.assignmentState}</dd>
        </div>
        <div>
          <dt>Lifecycle</dt>
          <dd>{row.lifecycleState}</dd>
        </div>
        <div>
          <dt>Workstation</dt>
          <dd>{row.workstationSessionId ?? "No workstation"}</dd>
        </div>
      </dl>
      <section>
        <h5>Blockers</h5>
        {row.blockerSummaries.length === 0 ? (
          <p>No blockers</p>
        ) : (
          <ul>
            {row.blockerSummaries.map((summary) => (
              <li key={summary}>{summary}</li>
            ))}
          </ul>
        )}
      </section>
    </section>
  );
}

function CommandConsoleCard({
  timelineKey,
  turn,
  actionPending,
  denialReason,
  onDenialReasonChange,
  onDecide,
}: {
  readonly timelineKey: string;
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
    <article
      className="command-console-card"
      data-timeline-key={timelineKey}
      data-timeline-kind="command"
      data-outcome={turn.status}
    >
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
      <small
        className="console-attribution"
        aria-label="Responsible capability: Orchestrator"
      >
        Capability: Orchestrator
      </small>
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

function failedWorkstationActionStorageKey(snapshot: WorkspaceSnapshot): string {
  const workspaceIdentity = encodeURIComponent(
    `${snapshot.workspace_session.id}:${snapshot.workspace_session.workspace_path}`,
  );
  return `alfredo:failed-workstation-actions:v1:${workspaceIdentity}`;
}

function workstationActionCorrelationId(turnId: string): string | null {
  const match = turnId.match(
    /^(.*):(?:intent|reaction:(?:pending|acknowledged|rejected|stale|failed))$/,
  );
  return match?.[1] || null;
}

function failedWorkstationActionTurns(
  turns: readonly WorkstationActionTurn[],
): readonly WorkstationActionTurn[] {
  const terminalOutcomes = new Set(["rejected", "stale", "failed"]);
  const failedCorrelations = new Set(
    turns.flatMap((turn) => {
      if (!terminalOutcomes.has(turn.outcome)) return [];
      const correlationId = workstationActionCorrelationId(turn.id);
      return correlationId ? [correlationId] : [];
    }),
  );
  const selected = turns.filter((turn) => {
    if (terminalOutcomes.has(turn.outcome)) return true;
    const correlationId = workstationActionCorrelationId(turn.id);
    return correlationId !== null && failedCorrelations.has(correlationId);
  });
  const deduplicated = new Map<string, WorkstationActionTurn>();
  for (const turn of selected) deduplicated.set(turn.id, turn);
  return [...deduplicated.values()].slice(-FAILED_WORKSTATION_ACTION_TURN_LIMIT);
}

function readFailedWorkstationActionContinuity(key: string): readonly WorkstationActionTurn[] {
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return [];
    const value = JSON.parse(raw) as Partial<FailedWorkstationActionContinuityState>;
    if (
      value.schema_version !== FAILED_WORKSTATION_ACTION_CONTINUITY_SCHEMA_VERSION ||
      !Array.isArray(value.turns)
    ) {
      return [];
    }
    const validated = value.turns.flatMap((candidate: unknown) => {
      if (!candidate || typeof candidate !== "object") return [];
      const turn = candidate as Partial<WorkstationActionTurn>;
      if (
        typeof turn.id !== "string" ||
        !turn.id ||
        turn.id.length > 512 ||
        typeof turn.content !== "string" ||
        !turn.content ||
        turn.content.length > FAILED_WORKSTATION_ACTION_CONTENT_LIMIT ||
        typeof turn.source !== "string" ||
        !turn.source ||
        turn.source.length > 128 ||
        typeof turn.outcome !== "string" ||
        !turn.outcome ||
        turn.outcome.length > 128
      ) {
        return [];
      }
      return [{
        id: turn.id,
        content: turn.content,
        source: turn.source,
        outcome: turn.outcome,
      }];
    });
    return failedWorkstationActionTurns(validated);
  } catch {
    return [];
  }
}

function writeFailedWorkstationActionContinuity(
  key: string,
  turns: readonly WorkstationActionTurn[],
): void {
  try {
    const value: FailedWorkstationActionContinuityState = {
      schema_version: FAILED_WORKSTATION_ACTION_CONTINUITY_SCHEMA_VERSION,
      turns: failedWorkstationActionTurns(turns),
    };
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Best-effort local failure continuity must never block canonical workspace state.
  }
}

function readWorkstationContinuity(key: string): WorkstationContinuityState | null {
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return null;
    const value = JSON.parse(raw) as PersistedWorkstationContinuityState;
    if (value.schema_version !== WORKSTATION_CONTINUITY_SCHEMA_VERSION) return null;
    return {
      schema_version: WORKSTATION_CONTINUITY_SCHEMA_VERSION,
      commandAuditOpen:
        typeof value.commandAuditOpen === "boolean"
          ? value.commandAuditOpen
          : value.leftLaneMode === "terminal",
      selectedIssueId: stringOrNull(value.selectedIssueId),
      selectedIssueMissionId: stringOrNull(value.selectedIssueMissionId),
      issueFocusTarget: issueFocusTargetOrNull(value.issueFocusTarget),
      selectedSessionId: stringOrNull(value.selectedSessionId),
      selectedSessionMissionId: stringOrNull(value.selectedSessionMissionId),
      selectedWorkstationDiff: workstationDiffOrNull(value.selectedWorkstationDiff),
      selectedExecutionNodeId: stringOrNull(value.selectedExecutionNodeId),
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

function issueFocusTargetOrNull(value: unknown): "assignment-board" | "mission-board" | null {
  return value === "assignment-board" || value === "mission-board" ? value : null;
}

function workstationDiffOrNull(value: unknown): WorkstationDiffLink | null {
  if (!value || typeof value !== "object") return null;
  const diff = value as Partial<WorkstationDiffLink>;
  return typeof diff.label === "string" &&
    typeof diff.path === "string" &&
    typeof diff.href === "string" &&
    typeof diff.missionId === "string" &&
    typeof diff.cardId === "string" &&
    typeof diff.sessionId === "string"
    ? {
        label: diff.label,
        path: diff.path,
        href: diff.href,
        missionId: diff.missionId,
        cardId: diff.cardId,
        sessionId: diff.sessionId,
      }
    : null;
}

function SessionArtifactViewer({
  viewerRef,
  state,
  onRetry,
  onClose,
}: {
  viewerRef: RefObject<HTMLElement | null>;
  state: SessionArtifactViewerState;
  onRetry: () => void;
  onClose: () => void;
}): ReactElement {
  const artifact = state.artifact;
  return (
    <section
      ref={viewerRef}
      className="session-artifact-viewer"
      aria-label="Session evidence viewer"
      tabIndex={-1}
    >
      <header>
        <div>
          <span className="eyebrow">Bounded session evidence</span>
          <h4>{artifact?.label ?? state.target.label}</h4>
          {state.target.focusPath ? <small>{state.target.focusPath}</small> : null}
        </div>
        <button type="button" aria-label="Close session evidence viewer" onClick={onClose}>
          Close
        </button>
      </header>
      {state.status === "loading" ? (
        <p role="status">Loading bounded session evidence…</p>
      ) : null}
      {state.status === "error" ? (
        <div className="inline-failure" role="alert">
          <span>{`Evidence load failed: ${state.message}`}</span>
          {state.recoverable ? (
            <button type="button" onClick={onRetry}>Retry evidence</button>
          ) : null}
        </div>
      ) : null}
      {state.status === "ready" && artifact ? (
        <div className="session-artifact-viewer__content">
          <div className="session-artifact-viewer__metadata">
            <span>{artifact.session_id}</span>
            <span>{artifact.media_type}</span>
            <span>{artifact.byte_count.toLocaleString()} bytes shown</span>
          </div>
          {artifact.truncated ? (
            <p role="status">
              Content is truncated at {artifact.content_limit_bytes.toLocaleString()} bytes.
            </p>
          ) : null}
          <pre aria-label={`${artifact.label} content`}>{artifact.content}</pre>
        </div>
      ) : null}
    </section>
  );
}

function WorkstationCard({
  card,
  expanded,
  pinned,
  selectedSessionId,
  agentOptions,
  onToggleExpanded,
  onTogglePinned,
  onSelectSession,
  onOpenDiff,
  onOpenEvidence,
  queueReasons,
  onQueueReasonChange,
  onQueueDecision,
  reviewReasons,
  onReviewReasonChange,
  onReviewDecision,
  workstationActionDrafts,
  onWorkstationActionDraftChange,
  onWorkstationAction,
  actionState,
  actionStatusRef,
}: {
  card: WorkstationCardProjection;
  expanded: boolean;
  pinned: boolean;
  selectedSessionId: string | null;
  agentOptions: readonly AgentCapability[];
  onToggleExpanded: () => void;
  onTogglePinned: () => void;
  onSelectSession: (sessionId: string) => void;
  onOpenDiff: (diff: WorkstationDiffLink, returnFocus: HTMLElement) => void;
  onOpenEvidence: (link: WorkstationEvidenceLink, returnFocus: HTMLElement) => void;
  queueReasons: Record<string, string>;
  onQueueReasonChange: (itemId: string, reason: string) => void;
  onQueueDecision: (itemId: string, decision: WorkspaceQueueDecision, reason: string) => void;
  reviewReasons: Record<string, string>;
  onReviewReasonChange: (sessionId: string, reason: string) => void;
  onReviewDecision: (
    sessionId: string,
    decision: ReviewDecision,
    reason: string,
    missionId?: string,
  ) => void;
  workstationActionDrafts: Record<string, WorkstationActionDraftState>;
  onWorkstationActionDraftChange: (key: string, draft: WorkstationActionDraftState) => void;
  onWorkstationAction: (
    action: WorkstationGovernedAction,
    draft: WorkstationActionDraftState,
  ) => void;
  actionState: WorkstationActionState | null;
  actionStatusRef: RefObject<HTMLSpanElement | null>;
}) {
  const queueActions = card.detail.governedActions.filter(
    (action) =>
      action.actionType === "workspace-queue-decision" && action.itemId && action.decision,
  );
  const reviewActions = card.detail.governedActions.filter(isReviewWorkstationAction);
  const workstationActions = card.detail.governedActions.filter(isExecutableWorkstationAction);
  const queueItemId = queueActions[0]?.itemId ?? null;
  const queueReason = queueItemId ? queueReasons[queueItemId] ?? "" : "";
  const workstationActionTargetIds = workstationActions.map(workstationActionStateId);
  const reviewActionTargetIds = reviewActions.map(workstationActionStateId);
  const matchingActionState =
    actionState &&
    (queueActions.some((action) => workstationActionStateId(action) === actionState.itemId) ||
      workstationActionTargetIds.includes(actionState.itemId) ||
      reviewActionTargetIds.includes(actionState.itemId))
      ? actionState
      : queueActions.length === 0 &&
          reviewActions.length === 0 &&
          workstationActions.length === 0 &&
          card.status === "waiting-approval"
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
  const lastActivity = workstationActivity(card.lastActivity);
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
          <dd>
            {lastActivity ? (
              <time dateTime={lastActivity.dateTime}>{lastActivity.label}</time>
            ) : (
              "Not recorded"
            )}
          </dd>
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
          ref={actionStatusRef}
          role={
            matchingActionState.state === "rejected" || matchingActionState.state === "failed"
              ? "alert"
              : "status"
          }
          aria-label={`${card.name} workstation action state`}
          className="connection-pill"
          tabIndex={-1}
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
      {reviewActions.length > 0 ? (
        <div className="workstation-card__decision-actions">
          {reviewActions.map((action) => {
            const sessionId = action.sessionId;
            const reviewStateId = workstationActionStateId(action);
            const reason = reviewReasons[reviewStateId] ?? "";
            const pending = matchingActionState?.state === "pending";
            const disabledDescription = workstationReviewActionDisabledDescription(
              action,
              reason,
              pending,
            );
            const helpId = disabledDescription
              ? `${workstationDomId(`review:${sessionId}:${action.reviewDecision}`)}-help`
              : undefined;
            const disabled = pending || (action.requiresReason && !reason.trim());
            return (
              <div className="workstation-card__direct-action" key={`review:${sessionId}:${action.reviewDecision}`}>
                {action.requiresReason ? (
                  <label className="composer">
                    <span>Reason</span>
                    <textarea
                      aria-label={`Workstation review reason ${sessionId}`}
                      rows={2}
                      value={reason}
                      onChange={(event) => onReviewReasonChange(reviewStateId, event.target.value)}
                    />
                  </label>
                ) : null}
                <button
                  type="button"
                  aria-label={`${action.label} ${sessionId}`}
                  aria-describedby={helpId}
                  disabled={disabled}
                  className={reviewDecisionButtonClass(action.reviewDecision)}
                  onClick={() =>
                    onReviewDecision(
                      sessionId,
                      action.reviewDecision,
                      action.requiresReason ? reason : "",
                      action.missionId,
                    )
                  }
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
      {workstationActions.length > 0 ? (
        <div className="workstation-card__decision-actions">
          {workstationActions.map((action) => {
            const targetId = workstationActionTargetId(action);
            const key = workstationActionKey(action);
            const draft = workstationActionDrafts[key] ?? { reason: "", agentId: "" };
            const pending = matchingActionState?.state === "pending";
            const needsAgent = action.actionType === "model-assignment-change";
            const hasEligibleAgent = agentOptions.some((agent) => agent.id === draft.agentId);
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
              (needsAgent && !hasEligibleAgent);
            return (
              <div className="workstation-card__direct-action" key={key}>
                {needsAgent ? (
                  <label>
                    <span>Agent</span>
                    {agentOptions.length > 0 ? (
                      <select
                        aria-label={`Workstation action agent ${targetId}`}
                        value={hasEligibleAgent ? draft.agentId : ""}
                        onChange={(event) =>
                          onWorkstationActionDraftChange(key, {
                            ...draft,
                            agentId: event.target.value,
                          })
                        }
                      >
                        <option value="">Select a local agent</option>
                        {agentOptions.map((agent) => (
                          <option key={agent.id} value={agent.id}>
                            {agent.id} · {agent.model || agent.runner}
                          </option>
                        ))}
                      </select>
                    ) : (
                      <span role="status" aria-label={`Workstation action worker unavailable ${targetId}`}>
                        Worker assignment unavailable: no eligible local workers were discovered.
                      </span>
                    )}
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
                    key={`${diff.href}:${diff.path}`}
                    type="button"
                    aria-label={`Open diff ${diff.path}`}
                    onClick={(event) => onOpenDiff(diff, event.currentTarget)}
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
                    <button
                      type="button"
                      aria-label={`Open evidence ${link.label}`}
                      onClick={(event) => onOpenEvidence(link, event.currentTarget)}
                    >
                      {link.label}
                    </button>
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
                onClick={() => onSelectSession(card.id)}
              >
                {card.detail.originatingSessionId}
              </button>
              {selectedSessionId === card.id ? (
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

const WORKSTATION_ACTIVITY_TIMESTAMP_PATTERN =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/;

function workstationActivity(timestamp: string): { readonly dateTime: string; readonly label: string } | null {
  if (!WORKSTATION_ACTIVITY_TIMESTAMP_PATTERN.test(timestamp)) return null;
  const parsed = new Date(timestamp);
  if (Number.isNaN(parsed.getTime())) return null;
  return {
    dateTime: timestamp,
    label: parsed.toISOString().replace("T", " ").replace(/\.\d{3}Z$/, " UTC"),
  };
}

function workstationRelaunchCommand(workspace: string, controllerId: string): string {
  const controllerArgument = controllerId ? ` --agent ${shellQuote(controllerId)}` : "";
  return `cd -- ${shellQuote(workspace)} && alfredo workstation${controllerArgument}`;
}

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'"'"'`)}'`;
}

function workstationDomId(value: string): string {
  return `workstation-${value.replace(/[^a-zA-Z0-9_-]+/g, "-")}`;
}

function workstationCardSummary(card: WorkstationCardProjection): string {
  const blockers = card.approvalBlockers.length
    ? ` Approval blockers: ${card.approvalBlockers.map(workstationSentence).join(" ")}`
    : "";
  const lastActivity = workstationActivity(card.lastActivity)?.label ?? "Not recorded";
  return `${workstationSentence(card.status)} ${workstationSentence(card.currentTask)} Next action: ${workstationSentence(card.nextAction)}${blockers} Last activity: ${workstationSentence(lastActivity)}`;
}

const WORKSTATION_STATUS_DESCRIPTIONS: Record<WorkstationCardProjection["status"], string> = {
  queued: "Queued work has been acknowledged and is waiting for the Local Agent runner to claim it.",
  "waiting-approval": "Waiting for approval. Resolve the visible approval blocker before work can continue.",
  blocked: "Blocked work needs a recovery decision before progress can continue.",
  failed: "Failed work needs review, repair, retry, or human escalation.",
  reviewing: "Review is in progress. Monitor the review workspace for the next decision.",
  "review-ready": "Evidence is ready for review. Card controls submit through Review Workspace validation.",
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

function workstationReviewActionDisabledDescription(
  action: WorkstationGovernedAction & {
    actionType: "review-decision";
    reviewDecision: ReviewDecision;
    sessionId: string;
  },
  reason: string,
  pending: boolean,
): string | null {
  const pendingDescription = workstationPendingActionDescription(action.label, pending);
  if (pendingDescription) return pendingDescription;
  if (action.requiresReason && !reason.trim()) {
    return workstationRequiredInputDescription(action.label, action.sessionId, "a review reason");
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
    | "issue-approve"
    | "issue-launch"
    | "issue-retry"
    | "session-cancel"
    | "model-assignment-change";
} {
  return (
    action.actionType === "issue-approve" ||
    action.actionType === "issue-launch" ||
    action.actionType === "issue-retry" ||
    action.actionType === "session-cancel" ||
    action.actionType === "model-assignment-change"
  );
}

function isReviewWorkstationAction(
  action: WorkstationGovernedAction,
): action is WorkstationGovernedAction & {
  actionType: "review-decision";
  reviewDecision: ReviewDecision;
  sessionId: string;
} {
  return action.actionType === "review-decision" && Boolean(action.reviewDecision && action.sessionId);
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
  if (actionTarget === "review-workspace") return "Card controls submit through Review Workspace validation";
  if (actionTarget === "activity") return "Use Activity Journal review history";
  return "Local monitoring only";
}

function missionIdFromAppLocalHref(href: string | undefined): string {
  const encoded = href?.match(/^app-local:\/\/missions\/([^/]+)/)?.[1];
  if (!encoded) return "";
  try {
    return decodeURIComponent(encoded);
  } catch {
    return "";
  }
}

function ActivityJournal({
  projection,
  filters,
  status,
  loadFailure,
  onFilterChange,
  onRefresh,
  fallbackMissionId,
  onOpenEvidence,
}: {
  projection: ActivityJournalProjection | null;
  filters: ActivityJournalFilters;
  status: "pending" | "rejected" | null;
  loadFailure: string | null;
  onFilterChange: (filters: ActivityJournalFilters) => void;
  onRefresh: () => void;
  fallbackMissionId: string;
  onOpenEvidence: (
    missionId: string,
    sessionId: string,
    artifactRef: string,
    label: string,
    returnFocus: HTMLElement,
  ) => void;
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

      {loadFailure ? (
        <div role="alert" className="inline-failure">
          <span>{`Activity Journal load failed: ${loadFailure}`}</span>
          <button type="button" onClick={onRefresh}>Retry Activity Journal</button>
        </div>
      ) : null}

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
          {entries.map((entry) => {
            const missionHref = entry.affected_entities.find((entity) =>
              entity.href.startsWith("app-local://missions/"),
            )?.href;
            const missionId = missionIdFromAppLocalHref(missionHref) || fallbackMissionId;
            const sessionId = entry.affected_entities.find(
              (entity) =>
                entity.entity_type === "agent-session" ||
                entity.entity_type === "evidence-package",
            )?.entity_id ?? "";
            return <li key={entry.entry_id}>
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
                      {entity.href && entity.entity_type !== "evidence-package" ? (
                        <a href={entity.href}>{entity.label}</a>
                      ) : (
                        <strong>{entity.label}</strong>
                      )}
                    </div>
                  ))}
                </div>
                {entry.evidence_links.length > 0 ? (
                  <div className="activity-entry__evidence">
                    {entry.evidence_links.map((link, index) => (
                      <button
                        key={link}
                        type="button"
                        disabled={!missionId || !sessionId}
                        onClick={(event) =>
                          onOpenEvidence(
                            missionId,
                            sessionId,
                            link,
                            `Activity evidence ${index + 1}`,
                            event.currentTarget,
                          )
                        }
                      >
                        Open activity evidence {index + 1}
                      </button>
                    ))}
                  </div>
                ) : null}
              </article>
            </li>;
          })}
        </ol>
      )}
    </section>
  );
}

function WorkspaceQueue({
  projection,
  loadFailure,
  onRetry,
  missionDrafts,
  missionDraftLoadFailure,
  onMissionDraftRetry,
  missionDraftStatus,
  missionDraftReasons,
  status,
  reasons,
  onReasonChange,
  onDecision,
  onMissionDraftReasonChange,
  onMissionDraftDecision,
}: {
  projection: WorkspaceQueueProjection | null;
  loadFailure: string | null;
  onRetry: () => void;
  missionDrafts: MissionDraftProjection | null;
  missionDraftLoadFailure: string | null;
  onMissionDraftRetry: () => void;
  missionDraftStatus: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  missionDraftReasons: Record<string, string>;
  status: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  reasons: Record<string, string>;
  onReasonChange: (itemId: string, reason: string) => void;
  onDecision: (itemId: string, decision: WorkspaceQueueDecision, reason: string) => void;
  onMissionDraftReasonChange: (draftId: string, reason: string) => void;
  onMissionDraftDecision: (
    draftId: string,
    decision: MissionDraftDecision,
    reason: string,
  ) => void;
}) {
  const decisionStatusRef = useRef<HTMLSpanElement>(null);
  const missionDraftDecisionStatusRef = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (status) decisionStatusRef.current?.focus();
  }, [status]);
  useEffect(() => {
    if (missionDraftStatus) missionDraftDecisionStatusRef.current?.focus();
  }, [missionDraftStatus]);
  if (!projection) {
    return (
      <div className="empty-state">
        <span className="eyebrow">Governance inbox</span>
        <h1>Workspace Queue</h1>
        {loadFailure ? (
          <>
            <p role="alert">Workspace Queue load failed: {loadFailure}</p>
            <button type="button" onClick={onRetry}>Retry Workspace Queue</button>
          </>
        ) : <p>Loading queue items.</p>}
      </div>
    );
  }

  const pendingItems = projection.items.filter((item) => item.status === "pending");
  const pendingGroups = projection.groups.flatMap((group) => {
    const items = group.items.filter((item) => item.status === "pending");
    return items.length > 0 ? [{ ...group, item_count: items.length, items }] : [];
  });
  const pendingDrafts = missionDrafts?.drafts.filter((draft) => draft.status === "draft") ?? [];
  const pendingDraftProjection = missionDrafts
    ? { ...missionDrafts, drafts: pendingDrafts }
    : null;
  const pendingDecisionCount = pendingItems.length + pendingDrafts.length;
  const decisionInboxComplete =
    !loadFailure && !missionDraftLoadFailure && missionDrafts !== null;
  const decisionCountLabel = decisionInboxComplete
    ? `${pendingDecisionCount} decisions pending`
    : loadFailure || missionDraftLoadFailure
      ? "Decision sources unavailable"
      : "Loading decisions";

  return (
    <section className="workspace-queue" aria-label="Workspace Queue">
      <div className="mission-heading">
        <div>
          <span className="eyebrow">Governance inbox</span>
          <h1>Workspace Queue</h1>
        </div>
        <div className="mission-count">
          <strong>{decisionCountLabel}</strong>
        </div>
      </div>
      {loadFailure ? (
        <div role="alert" className="inline-failure">
          <span>{`Workspace Queue load failed: ${loadFailure}`}</span>
          <button type="button" onClick={onRetry}>Retry Workspace Queue</button>
        </div>
      ) : null}
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
      {missionDraftStatus ? (
        <span
          ref={missionDraftDecisionStatusRef}
          role="status"
          aria-label="Mission Draft decision status"
          className="connection-pill"
          tabIndex={-1}
        >
          {missionDraftStatus.state[0].toUpperCase() + missionDraftStatus.state.slice(1)}: {missionDraftStatus.message}
        </span>
      ) : null}
      {pendingDrafts.length > 0 || missionDraftLoadFailure ? (
        <MissionDrafts
          projection={pendingDraftProjection}
          loadFailure={missionDraftLoadFailure}
          onRetry={onMissionDraftRetry}
          status={missionDraftStatus}
          reasons={missionDraftReasons}
          onReasonChange={onMissionDraftReasonChange}
          onDecision={onMissionDraftDecision}
        />
      ) : null}
      {decisionInboxComplete && pendingDecisionCount === 0 ? (
        <div className="empty-state">
          <h2>No decisions pending</h2>
          <p>New governance requests will appear here when they need your attention.</p>
        </div>
      ) : pendingItems.length > 0 ? (
        <div className="queue-groups">
          {pendingGroups.map((group) => (
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
      ) : null}
    </section>
  );
}

function MissionDrafts({
  projection,
  loadFailure,
  onRetry,
  status,
  reasons,
  onReasonChange,
  onDecision,
}: {
  projection: MissionDraftProjection | null;
  loadFailure: string | null;
  onRetry: () => void;
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
        {loadFailure ? (
          <>
            <p role="alert">Mission Draft load failed: {loadFailure}</p>
            <button type="button" onClick={onRetry}>Retry Mission Drafts</button>
          </>
        ) : <p>Loading Mission Drafts.</p>}
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
      {loadFailure ? (
        <div role="alert" className="inline-failure">
          <span>{`Mission Draft load failed: ${loadFailure}`}</span>
          <button type="button" onClick={onRetry}>Retry Mission Drafts</button>
        </div>
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
  loadFailure,
  onRetry,
  status,
  reasons,
  onReasonChange,
  onDecision,
  onOpenEvidence,
}: {
  projection: ReviewWorkspaceProjection | null;
  loadFailure: string | null;
  onRetry: () => void;
  status: {
    state: "pending" | "acknowledged" | "stale" | "rejected";
    message: string;
  } | null;
  reasons: Record<string, string>;
  onReasonChange: (sessionId: string, reason: string) => void;
  onDecision: (sessionId: string, decision: ReviewDecision, reason: string) => void;
  onOpenEvidence: (
    missionId: string,
    sessionId: string,
    artifactRef: string,
    label: string,
    returnFocus: HTMLElement,
  ) => void;
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
        {loadFailure ? (
          <>
            <p role="alert">Review Workspace load failed: {loadFailure}</p>
            <button type="button" onClick={onRetry}>Retry Review Workspace</button>
          </>
        ) : <p>Loading review evidence.</p>}
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
      {loadFailure ? (
        <div role="alert" className="inline-failure">
          <span>{`Review Workspace load failed: ${loadFailure}`}</span>
          <button type="button" onClick={onRetry}>Retry Review Workspace</button>
        </div>
      ) : null}
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
                  <h3>Evidence artifacts</h3>
                  {item.evidence.artifact_links.length === 0 ? (
                    <p>No safe artifact is registered.</p>
                  ) : (
                    <div className="context-inspector__actions">
                      {item.evidence.artifact_links.map((link, index) => {
                        const label = /(?:review\.diff|review_diff)(?:$|\/)/i.test(link)
                          ? "Review diff"
                          : `Evidence artifact ${index + 1}`;
                        return (
                          <button
                            key={link}
                            type="button"
                            onClick={(event) =>
                              onOpenEvidence(
                                item.mission_id,
                                item.session_id,
                                link,
                                label,
                                event.currentTarget,
                              )
                            }
                          >
                            Open {label.toLowerCase()}
                          </button>
                        );
                      })}
                    </div>
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
