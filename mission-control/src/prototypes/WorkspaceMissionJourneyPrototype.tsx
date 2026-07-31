/**
 * PROTOTYPE — throw this away after resolving the journey decision.
 *
 * Three variants of the Coding Workspace-to-Mission journey, switchable via
 * `?variant=`, at the dedicated workspace-mission-journey Vite mode root.
 */
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import "./workspace-mission-journey-prototype.css";

type VariantKey = "A" | "B" | "C";
type WorkspaceChoice = "none" | "existing" | "new";
type MissionChoice = "none" | "resume" | "new";
type FormationRoute = "none" | "ad-hoc" | "discovery" | "wayfinding";
type PlanningState = "not-started" | "planning" | "ready" | "published";
type FrontierState = "waiting" | "ready" | "running";

interface JourneyState {
  readonly workspace: WorkspaceChoice;
  readonly mission: MissionChoice;
  readonly route: FormationRoute;
  readonly routeOverridden: boolean;
  readonly planning: PlanningState;
  readonly frontier: FrontierState;
  readonly conversationMessages: readonly string[];
}

interface JourneyActions {
  readonly selectWorkspace: (workspace: Exclude<WorkspaceChoice, "none">) => void;
  readonly selectMission: (mission: Exclude<MissionChoice, "none">) => void;
  readonly selectRoute: (route: Exclude<FormationRoute, "none">) => void;
  readonly finishPlanning: () => void;
  readonly publishPlan: () => void;
  readonly startFrontier: () => void;
  readonly sendConversationMessage: (message: string) => void;
  readonly reset: () => void;
}

interface VariantProps {
  readonly journey: JourneyState;
  readonly actions: JourneyActions;
}

const INITIAL_JOURNEY: JourneyState = {
  workspace: "none",
  mission: "none",
  route: "none",
  routeOverridden: false,
  planning: "not-started",
  frontier: "waiting",
  conversationMessages: [],
};

const CONVERSATIONAL_DEMO_JOURNEY: JourneyState = {
  workspace: "existing",
  mission: "resume",
  route: "wayfinding",
  routeOverridden: false,
  planning: "planning",
  frontier: "waiting",
  conversationMessages: [
    "Use the Alfredo repository. I want the workstation to feel as direct as the coding agents I already use.",
    "Resume the modernization Mission. Keep React and Tauri, and help me work out the rest with Wayfinding here in the conversation.",
  ],
};

const VARIANTS: ReadonlyArray<{
  readonly key: VariantKey;
  readonly name: string;
}> = [
  { key: "A", name: "Conversational workstation" },
  { key: "B", name: "Journey canvas" },
  { key: "C", name: "Execution tree" },
];

const RECOMMENDED_ROUTE: Exclude<FormationRoute, "none"> = "wayfinding";

const ROUTE_LABELS: Record<Exclude<FormationRoute, "none">, string> = {
  "ad-hoc": "Ad Hoc Delegation",
  discovery: "Mission discovery · grilling",
  wayfinding: "Multi-session Wayfinding",
};

function variantFromLocation(): VariantKey {
  const candidate = new URLSearchParams(window.location.search).get("variant");
  return candidate === "B" || candidate === "C" ? candidate : "A";
}

function activeStep(journey: JourneyState): number {
  if (journey.workspace === "none") return 0;
  if (journey.mission === "none") return 1;
  if (journey.route === "none") return 2;
  if (journey.planning === "planning" || journey.planning === "not-started") return 3;
  if (journey.planning === "ready") return 4;
  if (journey.frontier !== "running") return 5;
  return 6;
}

function workspaceLabel(workspace: WorkspaceChoice): string {
  if (workspace === "existing") return "Alfredo · existing repository";
  if (workspace === "new") return "New repository";
  return "Not selected";
}

function missionLabel(mission: MissionChoice): string {
  if (mission === "resume") return "Resume · Reliable Alfredo modernization";
  if (mission === "new") return "Start new Mission";
  return "Not selected";
}

function routeLabel(route: FormationRoute): string {
  return route === "none" ? "Not recommended yet" : ROUTE_LABELS[route];
}

function StateReadout({ journey }: { readonly journey: JourneyState }) {
  return (
    <dl className="journey-state-readout" aria-label="Current prototype state">
      <div>
        <dt>Starting Location</dt>
        <dd>~/Projects</dd>
      </div>
      <div>
        <dt>Coding Workspace</dt>
        <dd>{workspaceLabel(journey.workspace)}</dd>
      </div>
      <div>
        <dt>Mission</dt>
        <dd>{missionLabel(journey.mission)}</dd>
      </div>
      <div>
        <dt>Formation Route</dt>
        <dd>
          {routeLabel(journey.route)}
          {journey.routeOverridden ? " · overridden" : ""}
        </dd>
      </div>
      <div>
        <dt>Planning</dt>
        <dd>{journey.planning.replace("-", " ")}</dd>
      </div>
      <div>
        <dt>Safe frontier</dt>
        <dd>{journey.frontier}</dd>
      </div>
    </dl>
  );
}

function RouteChoices({
  journey,
  actions,
}: Pick<VariantProps, "journey" | "actions">) {
  return (
    <div className="journey-route-choices">
      {(Object.keys(ROUTE_LABELS) as Array<Exclude<FormationRoute, "none">>).map(
        (route) => (
          <button
            className={journey.route === route ? "is-selected" : ""}
            key={route}
            onClick={() => actions.selectRoute(route)}
            type="button"
          >
            <span>{ROUTE_LABELS[route]}</span>
            <small>
              {route === "ad-hoc" && "One narrow, fully specified request"}
              {route === "discovery" && "A bounded route is visible but needs decisions"}
              {route === "wayfinding" && "Several sessions are needed to clear the fog"}
            </small>
            {route === RECOMMENDED_ROUTE && <em>Recommended</em>}
          </button>
        ),
      )}
    </div>
  );
}

function PublicationActions({ journey, actions }: VariantProps) {
  if (journey.route === "none") return null;

  if (journey.planning === "planning" || journey.planning === "not-started") {
    return (
      <button className="journey-primary-action" onClick={actions.finishPlanning} type="button">
        Mark the route clear
      </button>
    );
  }
  if (journey.planning === "ready") {
    return (
      <button className="journey-primary-action" onClick={actions.publishPlan} type="button">
        Publish PRD + Issue Graph
      </button>
    );
  }
  if (journey.frontier !== "running") {
    return (
      <button className="journey-primary-action" onClick={actions.startFrontier} type="button">
        Approve safe frontier start
      </button>
    );
  }
  return <strong className="journey-positive-outcome">Safe frontier is running</strong>;
}

function PrototypeChrome({
  eyebrow,
  title,
  summary,
  children,
}: {
  readonly eyebrow: string;
  readonly title: string;
  readonly summary: string;
  readonly children: ReactNode;
}) {
  return (
    <div className="journey-prototype">
      <header className="journey-prototype__topbar">
        <div className="journey-prototype__brand">
          <span aria-hidden="true" />
          <strong>ALFREDO</strong>
          <small>throwaway journey prototype</small>
        </div>
        <div className="journey-prototype__variant-intro">
          <span>{eyebrow}</span>
          <strong>{title}</strong>
        </div>
        <p>{summary}</p>
      </header>
      {children}
    </div>
  );
}

type InspectableSessionId = "wayfinder" | "workspace-agent" | "formation-agent";

interface InspectableSession {
  readonly id: InspectableSessionId;
  readonly workTitle: string;
  readonly agentName: string;
  readonly role: string;
  readonly model: string;
  readonly status: "working" | "queued" | "complete";
  readonly task: string;
  readonly activity: string;
  readonly latestUpdate: string;
  readonly activityTrail: readonly string[];
}

type AgentOutputLineKind = "model" | "tool" | "result" | "state";

interface AgentOutputLine {
  readonly kind: AgentOutputLineKind;
  readonly content: string;
}

type ExecutionStatusState =
  | "active"
  | "working"
  | "decision"
  | "queued"
  | "complete"
  | "blocked";

function ExecutionStatus({
  className = "",
  label,
  state,
}: {
  readonly className?: string;
  readonly label: string;
  readonly state: ExecutionStatusState;
}) {
  return (
    <span
      className={`journey-execution-status ${className}`.trim()}
      data-state={state}
    >
      <i aria-hidden="true" />
      {label}
    </span>
  );
}

function sessionStatusLabel(status: InspectableSession["status"]): string {
  if (status === "working") return "Working";
  if (status === "complete") return "Complete";
  return "Queued";
}

type ConversationCapability =
  | "workspace"
  | "wayfinder"
  | "orchestrator"
  | "mission";

interface ConversationReply {
  readonly capability: ConversationCapability;
  readonly content: string;
  readonly event?: string;
}

type PromptCompletionKind = "command" | "mention";

interface PromptCompletion {
  readonly value: string;
  readonly detail: string;
  readonly kind: PromptCompletionKind;
}

interface PromptCompletionQuery {
  readonly kind: PromptCompletionKind;
  readonly start: number;
  readonly value: string;
}

const COMMAND_COMPLETIONS: readonly PromptCompletion[] = [
  { value: "/help", detail: "Show Alfredo console commands", kind: "command" },
  { value: "/status", detail: "Summarize the current Mission state", kind: "command" },
  { value: "/agents", detail: "Show active and queued agent sessions", kind: "command" },
  { value: "/skills", detail: "Show tools and skills Alfredo can use", kind: "command" },
  { value: "/workspace", detail: "Inspect the active Coding Workspace", kind: "command" },
  { value: "/wayfinder", detail: "Inspect the active Wayfinding route", kind: "command" },
];

const MENTION_COMPLETIONS: readonly PromptCompletion[] = [
  { value: "@workspace", detail: "Coding Workspace context", kind: "mention" },
  { value: "@wayfinder", detail: "Mission planning capability", kind: "mention" },
  { value: "@orchestrator", detail: "Mission authority and execution", kind: "mention" },
  { value: "@codex", detail: "Frontier Architect session", kind: "mention" },
  { value: "@gemma-4-12b", detail: "Local Agent · workspace contract", kind: "mention" },
  {
    value: "@qwen-2.5-coder",
    detail: "Local Agent · Mission formation",
    kind: "mention",
  },
];

const KNOWN_CONSOLE_COMMANDS = new Set(
  COMMAND_COMPLETIONS.map((completion) => completion.value),
);

function consoleCommand(message: string): string | null {
  const command = message.trim().split(/\s+/, 1)[0]?.toLowerCase();
  return command && KNOWN_CONSOLE_COMMANDS.has(command) ? command : null;
}

function consoleCommandReply(command: string): ConversationReply {
  if (command === "/help") {
    return {
      capability: "mission",
      content:
        "Console commands: /status, /agents, /skills, /workspace, and /wayfinder. Use @ to address a capability or a visible agent directly; normal language remains the primary control surface.",
      event: "Console help displayed",
    };
  }
  if (command === "/status") {
    return {
      capability: "orchestrator",
      content:
        "Mission Work on the right is the canonical live state: planning is active, one Wayfinding agent is working, and a planning decision is pending.",
      event: "Mission status read from the Orchestrator",
    };
  }
  if (command === "/agents") {
    return {
      capability: "orchestrator",
      content:
        "Codex is working on Mission Wayfinding. The workspace-contract and conversational-formation Local Agents are waiting behind the current planning decision. Select any started session in Mission Work to inspect its output.",
      event: "Agent session roster displayed",
    };
  }
  if (command === "/skills") {
    return {
      capability: "workspace",
      content:
        "This Mission currently uses the Coding Workspace, Wayfinding, Orchestrator, GitHub issue tracker, and governed Local Agent runners. I’ll surface the relevant tool or skill label on each response.",
      event: "Active Alfredo capabilities displayed",
    };
  }
  if (command === "/workspace") {
    return {
      capability: "workspace",
      content:
        "The active Coding Workspace is the Alfredo repository. It supplies repository context, local tools, and the execution boundary for this Mission.",
      event: "Coding Workspace context displayed",
    };
  }
  return {
    capability: "wayfinder",
    content:
      "Wayfinding is active inside this conversation. I’m resolving the interaction decisions one at a time and recording accepted answers into the shared Mission map.",
    event: "Wayfinding route displayed",
  };
}

function promptCompletionQuery(draft: string): PromptCompletionQuery | null {
  const commandMatch = draft.match(/^\/[^\s]*$/);
  if (commandMatch) {
    return { kind: "command", start: 0, value: commandMatch[0] };
  }

  const mentionMatch = draft.match(/(?:^|\s)(@[^\s]*)$/);
  const mention = mentionMatch?.[1];
  if (!mention) return null;
  return {
    kind: "mention",
    start: draft.length - mention.length,
    value: mention,
  };
}

function completionMatches(
  query: PromptCompletionQuery | null,
): readonly PromptCompletion[] {
  if (!query) return [];
  const candidates =
    query.kind === "command" ? COMMAND_COMPLETIONS : MENTION_COMPLETIONS;
  const matches = candidates.filter((candidate) =>
    candidate.value.toLowerCase().startsWith(query.value.toLowerCase()),
  );
  if (
    matches.length === 1 &&
    matches[0].value.toLowerCase() === query.value.toLowerCase()
  ) {
    return [];
  }
  return matches;
}

function conversationReply(index: number, message: string): ConversationReply {
  const command = consoleCommand(message);
  if (command) return consoleCommandReply(command);

  const replies = [
    {
      capability: "workspace",
      content:
        "I found the Alfredo repository and established it as the Coding Workspace. It already has a modernization Mission, so I will not create a duplicate without discussing it with you.",
    },
    {
      capability: "wayfinder",
      content:
        "I resumed that Mission. This goal crosses interaction design, orchestration, performance, and migration, so I recommend Wayfinding. I’ll run it here in our conversation and ask one focused question at a time. First: when several agents are working, what must you understand without opening their raw output?",
      event: "Wayfinding session started · Codex · Frontier Architect",
    },
    {
      capability: "wayfinder",
      content:
        "That makes the supervision requirement clear: work stays primary, every started agent is visible beneath its work, and its live session can be inspected directly. The route is now clear enough to publish as a Product Requirements Document and Issue Graph. Continue in your own words; I’ll infer whether you want changes or publication.",
      event: "Wayfinding map updated · interaction decision captured",
    },
    {
      capability: "orchestrator",
      content:
        "Published. The Issue Graph has two safe frontier items and one blocked migration item. Nothing has started yet. Tell me naturally whether to begin, narrow the frontier, or inspect the plan first.",
      event: "PRD and Issue Graph published · 2 ready · 1 blocked",
    },
    {
      capability: "orchestrator",
      content:
        "The safe frontier is running. Two Local Agents have started. Their exact task, model, current activity, and latest update are visible in Mission Work; select either session to inspect it while we keep talking here.",
      event: "2 Local Agent sessions acknowledged and running",
    },
  ] as const;

  return (
    replies[index] ?? {
      capability: "mission",
      content:
        "I’m keeping the conversation attached to this Mission. You can redirect the work, ask about an agent, or continue shaping the next decision without leaving the Agent Console.",
    }
  );
}

function inspectableSessions(journey: JourneyState): readonly InspectableSession[] {
  const sessions: InspectableSession[] = [];
  if (journey.route === "wayfinding") {
    sessions.push({
      id: "wayfinder",
      workTitle: "Shape the modernization Mission",
      agentName: "Codex",
      role: "Frontier Architect",
      model: "Frontier Model",
      status: journey.planning === "published" ? "complete" : "working",
      task: "Resolve the Coding Workspace-to-Mission interaction and clear the route to a publishable plan.",
      activity:
        journey.planning === "published"
          ? "Resolution captured; monitoring the published Issue Graph."
          : "Discussing the Mission Execution Tree with the Mission Commander.",
      latestUpdate:
        journey.planning === "published"
          ? "Published the interaction decision into the Mission plan."
          : "Recorded that Mission formation remains conversational and agent sessions must be directly inspectable.",
      activityTrail: [
        "Loaded the active Mission and prior decisions",
        "Recommended Wayfinding with an explicit rationale",
        journey.planning === "published"
          ? "Closed the interaction question"
          : "Waiting for the next conversational answer",
      ],
    });
  }
  if (journey.planning === "published") {
    sessions.push(
      {
        id: "workspace-agent",
        workTitle: "Workspace selection contract",
        agentName: "Gemma 4 12B",
        role: "Local Agent",
        model: "gemma4:12b · Ollama",
        status: journey.frontier === "running" ? "working" : "queued",
        task: "Define and verify the Starting Location-to-Coding Workspace contract.",
        activity:
          journey.frontier === "running"
            ? "Reading launcher and workspace-boundary code."
            : "Waiting for safe-frontier dispatch.",
        latestUpdate:
          journey.frontier === "running"
            ? "Located the launcher boundary and is comparing existing-workspace recovery paths."
            : "Task packet validated; no process has started.",
        activityTrail: [
          "Task packet accepted",
          journey.frontier === "running"
            ? "Workspace scan in progress"
            : "Queued behind Mission Commander start",
        ],
      },
      {
        id: "formation-agent",
        workTitle: "Conversational Mission formation",
        agentName: "Qwen 2.5 Coder 14B",
        role: "Local Agent",
        model: "qwen2.5-coder:14b · Ollama",
        status: journey.frontier === "running" ? "working" : "queued",
        task: "Prototype conversational grilling and Wayfinding inside Agent Console.",
        activity:
          journey.frontier === "running"
            ? "Tracing controller conversation and Mission planning seams."
            : "Waiting for safe-frontier dispatch.",
        latestUpdate:
          journey.frontier === "running"
            ? "Preparing the first conversation-to-plan state transition."
            : "Task packet validated; no process has started.",
        activityTrail: [
          "Task packet accepted",
          journey.frontier === "running"
            ? "Controller-flow analysis in progress"
            : "Queued behind Mission Commander start",
        ],
      },
    );
  }
  return sessions;
}

const sessionOutputLines: Record<
  InspectableSessionId,
  readonly AgentOutputLine[]
> = {
  wayfinder: [
    {
      kind: "model",
      content:
        "I’m mapping the workspace-to-Mission path against the current interaction decision.",
    },
    {
      kind: "tool",
      content: "Reading issue #44 and the Mission Execution Tree prototype.",
    },
    {
      kind: "result",
      content:
        "Conversation-first formation and directly inspectable agent sessions match the selected route.",
    },
    {
      kind: "model",
      content:
        "Next I’m checking that each running agent exposes its task, activity, and decision state.",
    },
  ],
  "workspace-agent": [
    {
      kind: "model",
      content:
        "I’ll trace how a Starting Location becomes an acknowledged Coding Workspace.",
    },
    {
      kind: "tool",
      content:
        '$ rg -n "workspace|starting_location" mission-control albert_mvp',
    },
    {
      kind: "result",
      content:
        "The launcher and Orchestrator both participate; the handoff needs one canonical authority boundary.",
    },
    {
      kind: "model",
      content:
        "I’m comparing restart recovery paths before proposing the workspace contract.",
    },
  ],
  "formation-agent": [
    {
      kind: "model",
      content:
        "I’m tracing how a natural conversation becomes an explicit Mission Formation Route.",
    },
    {
      kind: "tool",
      content:
        '$ rg -n "conversation|wayfind|mission draft" mission-control albert_mvp',
    },
    {
      kind: "result",
      content:
        "The conversation can remain primary while the Orchestrator records the governed route and accepted plan.",
    },
    {
      kind: "model",
      content:
        "Next I’m checking where a Mission Commander decision must pause automatic work.",
    },
  ],
};

function AgentOutputStream({
  session,
}: {
  readonly session: InspectableSession;
}) {
  const outputLines =
    session.status === "queued"
      ? ([
          {
            kind: "state",
            content:
              "No model output yet. The task packet is queued and no Local Agent process is running.",
          },
        ] satisfies readonly AgentOutputLine[])
      : sessionOutputLines[session.id];
  const [visibleLineCount, setVisibleLineCount] = useState(1);

  useEffect(() => {
    if (session.status !== "working") {
      setVisibleLineCount(outputLines.length);
      return;
    }

    setVisibleLineCount(1);
    const intervalId = window.setInterval(() => {
      setVisibleLineCount((currentCount) => {
        if (currentCount >= outputLines.length) {
          window.clearInterval(intervalId);
          return currentCount;
        }
        return currentCount + 1;
      });
    }, 650);

    return () => window.clearInterval(intervalId);
  }, [outputLines.length, session.id, session.status]);

  const streamState =
    session.status === "working"
      ? "Live"
      : session.status === "complete"
        ? "Snapshot"
        : "Queued";

  return (
    <section
      aria-label={`${session.agentName} visible output`}
      className="journey-agent-output"
    >
      <header>
        <div>
          <strong>Agent output</strong>
          <span>Connected only while this inspector is open</span>
        </div>
        <span
          className="journey-agent-output__connection"
          data-state={session.status}
        >
          <i aria-hidden="true" />
          {streamState}
        </span>
      </header>
      <ol aria-live="polite">
        {outputLines.slice(0, visibleLineCount).map((line, index) => (
          <li data-kind={line.kind} key={`${line.kind}-${index}`}>
            <span>{line.kind}</span>
            <p>{line.content}</p>
          </li>
        ))}
      </ol>
    </section>
  );
}

function GuidedConversationVariant({ journey, actions }: VariantProps) {
  const [draft, setDraft] = useState("");
  const [promptHistory, setPromptHistory] = useState<readonly string[]>([]);
  const [historyIndex, setHistoryIndex] = useState<number | null>(null);
  const [selectedCompletionIndex, setSelectedCompletionIndex] = useState(0);
  const [dismissedCompletion, setDismissedCompletion] = useState<string | null>(
    null,
  );
  const [selectedSessionId, setSelectedSessionId] =
    useState<InspectableSessionId | null>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const draftBeforeHistoryRef = useRef("");
  const transcriptRef = useRef<HTMLDivElement>(null);
  const completionQuery = promptCompletionQuery(draft);
  const completionSignature = completionQuery
    ? `${completionQuery.kind}:${completionQuery.start}:${completionQuery.value}`
    : null;
  const promptCompletions =
    completionSignature === dismissedCompletion
      ? []
      : completionMatches(completionQuery);
  const sessions = inspectableSessions(journey);
  const selectedSession = selectedSessionId
    ? sessions.find((session) => session.id === selectedSessionId) ?? null
    : null;
  const workingSessionCount = sessions.filter(
    (session) => session.status === "working",
  ).length;
  const queuedSessionCount = sessions.filter(
    (session) => session.status === "queued",
  ).length;
  const planningStatus: {
    readonly label: string;
    readonly state: ExecutionStatusState;
  } =
    journey.planning === "published"
      ? { label: "Complete", state: "complete" }
      : { label: "Decision needed", state: "decision" };
  const safeFrontierStatus: {
    readonly label: string;
    readonly state: ExecutionStatusState;
  } =
    journey.planning !== "published"
      ? { label: "Waiting", state: "queued" }
      : journey.frontier === "running"
        ? { label: "Working", state: "working" }
        : { label: "Start decision", state: "decision" };
  const pendingDecision =
    journey.planning !== "published"
      ? "Planning decision pending"
      : journey.frontier !== "running"
        ? "Safe frontier decision pending"
        : "No decisions pending";
  const step = activeStep(journey);
  const placeholder =
    step === 3
      ? "Answer Alfredo’s Wayfinding question…"
      : step === 4
        ? "Continue the plan or ask Alfredo to publish…"
        : step === 5
          ? "Tell Alfredo what to do with the safe frontier…"
          : "Ask about an agent, redirect the work, or continue the Mission…";

  useEffect(() => {
    const transcript = transcriptRef.current;
    if (!transcript) return;
    transcript.scrollTop = transcript.scrollHeight;
  }, [journey.conversationMessages.length]);

  useEffect(() => {
    setSelectedCompletionIndex(0);
  }, [completionSignature]);

  const submitConversation = (event?: FormEvent) => {
    event?.preventDefault();
    const message = draft.trim();
    if (!message) return;
    actions.sendConversationMessage(message);
    setPromptHistory((currentHistory) => [...currentHistory, message]);
    setHistoryIndex(null);
    draftBeforeHistoryRef.current = "";
    setDismissedCompletion(null);
    setDraft("");
  };

  const applyCompletion = (completion: PromptCompletion) => {
    if (!completionQuery) return;
    const replacedUntil = completionQuery.start + completionQuery.value.length;
    const nextCursor = completionQuery.start + completion.value.length + 1;
    const nextDraft = `${draft.slice(0, completionQuery.start)}${completion.value} ${draft.slice(replacedUntil)}`;
    setDraft(nextDraft);
    setHistoryIndex(null);
    setDismissedCompletion(null);
    window.requestAnimationFrame(() => {
      composerRef.current?.focus();
      composerRef.current?.setSelectionRange(nextCursor, nextCursor);
    });
  };

  const navigatePromptHistory = (direction: -1 | 1) => {
    if (promptHistory.length === 0) return;

    if (direction === -1) {
      const nextIndex =
        historyIndex === null
          ? promptHistory.length - 1
          : Math.max(0, historyIndex - 1);
      if (historyIndex === null) draftBeforeHistoryRef.current = draft;
      setHistoryIndex(nextIndex);
      setDraft(promptHistory[nextIndex]);
      setDismissedCompletion(null);
      return;
    }

    if (historyIndex === null) return;
    const nextIndex = historyIndex + 1;
    if (nextIndex >= promptHistory.length) {
      setHistoryIndex(null);
      setDraft(draftBeforeHistoryRef.current);
    } else {
      setHistoryIndex(nextIndex);
      setDraft(promptHistory[nextIndex]);
    }
    setDismissedCompletion(null);
  };

  const handleComposerKeyDown = (
    event: ReactKeyboardEvent<HTMLTextAreaElement>,
  ) => {
    if (promptCompletions.length > 0) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const offset = event.key === "ArrowDown" ? 1 : -1;
        setSelectedCompletionIndex(
          (currentIndex) =>
            (currentIndex + offset + promptCompletions.length) %
            promptCompletions.length,
        );
        return;
      }
      if (
        event.key === "Tab" ||
        (event.key === "Enter" && !event.shiftKey)
      ) {
        event.preventDefault();
        applyCompletion(
          promptCompletions[selectedCompletionIndex] ?? promptCompletions[0],
        );
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setDismissedCompletion(completionSignature);
        return;
      }
    }

    if (
      !draft.includes("\n") &&
      (event.key === "ArrowUp" || event.key === "ArrowDown")
    ) {
      const canNavigate =
        promptHistory.length > 0 &&
        (event.key === "ArrowUp" || historyIndex !== null);
      if (canNavigate) {
        event.preventDefault();
        navigatePromptHistory(event.key === "ArrowUp" ? -1 : 1);
        return;
      }
    }

    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submitConversation();
    }
  };

  return (
    <PrototypeChrome
      eyebrow="Variant A · revised"
      title="Conversational workstation"
      summary="Free-form conversation shapes the Mission; an inspectable execution tree shows what every agent is doing."
    >
      <main className="journey-conversation-layout">
        <section className="journey-conversation" aria-label="Agent Console">
          <div
            aria-live="polite"
            className="journey-transcript"
            ref={transcriptRef}
          >
            <article className="journey-terminal-turn journey-terminal-turn--alfredo">
              <div className="journey-terminal-lead">
                <span aria-hidden="true" className="journey-terminal-mark">●</span>
                <strong>alfredo</strong>
                <span
                  aria-label="Current Alfredo activity: workspace"
                  className="journey-terminal-capability"
                >
                  workspace
                </span>
              </div>
              <p>
                You launched me from <code>~/Projects</code>. Tell me what you want to
                work on; I’ll establish the Coding Workspace, find relevant Missions,
                and discuss the safest route with you here.
              </p>
            </article>

            {journey.conversationMessages.map((message, index) => {
              const conversationalIndex = journey.conversationMessages
                .slice(0, index)
                .filter((priorMessage) => consoleCommand(priorMessage) === null)
                .length;
              const reply = conversationReply(conversationalIndex, message);
              return (
                <div className="journey-conversation-exchange" key={`${index}-${message}`}>
                  <article className="journey-terminal-turn journey-terminal-turn--human">
                    <div className="journey-terminal-lead">
                      <span aria-hidden="true" className="journey-terminal-mark">❯</span>
                      <strong>you</strong>
                    </div>
                    <p>{message}</p>
                  </article>
                  <article className="journey-terminal-turn journey-terminal-turn--alfredo">
                    <div className="journey-terminal-lead">
                      <span aria-hidden="true" className="journey-terminal-mark">●</span>
                      <strong>alfredo</strong>
                      <span
                        aria-label={`Current Alfredo activity: ${reply.capability}`}
                        className="journey-terminal-capability"
                      >
                        {reply.capability}
                      </span>
                    </div>
                    <p>{reply.content}</p>
                    {reply.event && (
                      <div className="journey-conversation-event">
                        <span aria-hidden="true">↳</span>
                        <span>{reply.event}</span>
                      </div>
                    )}
                  </article>
                </div>
              );
            })}
          </div>

          <form className="journey-conversation-composer" onSubmit={submitConversation}>
            <div className="journey-terminal-input">
              <span aria-hidden="true">you@alfredo:~$</span>
              <textarea
                aria-label="Message Alfredo"
                aria-activedescendant={
                  promptCompletions.length > 0
                    ? `journey-prompt-completion-${selectedCompletionIndex}`
                    : undefined
                }
                aria-autocomplete="list"
                aria-controls={
                  promptCompletions.length > 0
                    ? "journey-prompt-completions"
                    : undefined
                }
                aria-expanded={promptCompletions.length > 0}
                id="journey-conversation-draft"
                onChange={(event) => {
                  setDraft(event.target.value);
                  setHistoryIndex(null);
                  setDismissedCompletion(null);
                }}
                onKeyDown={handleComposerKeyDown}
                placeholder={placeholder}
                ref={composerRef}
                rows={2}
                value={draft}
              />
            </div>
            {promptCompletions.length > 0 && (
              <div
                aria-label={
                  completionQuery?.kind === "command"
                    ? "Alfredo commands"
                    : "Alfredo mentions"
                }
                className="journey-terminal-completions"
                id="journey-prompt-completions"
                role="listbox"
              >
                {promptCompletions.map((completion, index) => (
                  <button
                    aria-selected={index === selectedCompletionIndex}
                    id={`journey-prompt-completion-${index}`}
                    key={completion.value}
                    onClick={() => applyCompletion(completion)}
                    onMouseDown={(event) => event.preventDefault()}
                    role="option"
                    type="button"
                  >
                    <code>{completion.value}</code>
                    <span>{completion.detail}</span>
                  </button>
                ))}
              </div>
            )}
            <div className="journey-conversation-composer__actions">
              <span>↑↓ history · @ mention · / commands · Tab completes</span>
              <button disabled={!draft.trim()} type="submit">
                Send
              </button>
            </div>
          </form>
        </section>

        <aside
          className={`journey-conversation-state${selectedSession ? " is-inspecting" : ""}`}
          aria-label="Mission Work"
        >
          <header>
            <span>Mission Work</span>
            <strong>Mission Execution Tree</strong>
            <small>
              {selectedSession
                ? "Inspecting one agent session"
                : `${sessions.length} agent session${sessions.length === 1 ? "" : "s"} visible`}
            </small>
          </header>

          <section className="journey-mission-summary">
            <ExecutionStatus
              className="journey-mission-summary__status"
              label={journey.frontier === "running" ? "Frontier running" : "Planning active"}
              state={journey.frontier === "running" ? "working" : "active"}
            />
            <strong>Reliable Alfredo modernization</strong>
            <p>{routeLabel(journey.route)} · {journey.planning}</p>
            <div
              aria-label="Mission execution status"
              className="journey-execution-signals"
            >
              <ExecutionStatus
                label={`${workingSessionCount} agent${workingSessionCount === 1 ? "" : "s"} working`}
                state={workingSessionCount > 0 ? "working" : "queued"}
              />
              <ExecutionStatus
                label={pendingDecision}
                state={journey.frontier === "running" ? "complete" : "decision"}
              />
              <ExecutionStatus
                label={`${queuedSessionCount} agent${queuedSessionCount === 1 ? "" : "s"} queued`}
                state={queuedSessionCount > 0 ? "queued" : "complete"}
              />
            </div>
          </section>

          {!selectedSession && (
            <section className="journey-execution-tree" aria-label="Mission agent sessions">
              <article className="journey-work-branch">
                <header>
                  <span className="journey-work-branch__index">01</span>
                  <div>
                    <strong>Shape the modernization Mission</strong>
                    <small>Wayfinding · interaction decision</small>
                  </div>
                  <ExecutionStatus
                    label={planningStatus.label}
                    state={planningStatus.state}
                  />
                </header>
                {sessions
                  .filter((session) => session.id === "wayfinder")
                  .map((session) => (
                    <button
                      aria-label={`Inspect ${session.agentName}: ${session.task}`}
                      className="journey-agent-session"
                      key={session.id}
                      onClick={() => setSelectedSessionId(session.id)}
                      type="button"
                    >
                      <span className="journey-agent-session__avatar">C</span>
                      <span>
                        <strong>{session.agentName}</strong>
                        <small>{session.role} · {session.activity}</small>
                      </span>
                      <ExecutionStatus
                        label={sessionStatusLabel(session.status)}
                        state={session.status}
                      />
                    </button>
                  ))}
              </article>

              <article className="journey-work-branch">
                <header>
                  <span className="journey-work-branch__index">02</span>
                  <div>
                    <strong>Workspace selection contract</strong>
                    <small>Issue Slice · safe frontier</small>
                  </div>
                  <ExecutionStatus
                    label={safeFrontierStatus.label}
                    state={safeFrontierStatus.state}
                  />
                </header>
                {sessions
                  .filter((session) => session.id === "workspace-agent")
                  .map((session) => (
                    <button
                      aria-label={`Inspect ${session.agentName}: ${session.task}`}
                      className="journey-agent-session"
                      key={session.id}
                      onClick={() => setSelectedSessionId(session.id)}
                      type="button"
                    >
                      <span className="journey-agent-session__avatar">G</span>
                      <span>
                        <strong>{session.agentName}</strong>
                        <small>{session.role} · {session.activity}</small>
                      </span>
                      <ExecutionStatus
                        label={sessionStatusLabel(session.status)}
                        state={session.status}
                      />
                    </button>
                  ))}
                {journey.planning !== "published" && (
                  <p className="journey-no-session">No Local Agent session started yet.</p>
                )}
              </article>

              <article className="journey-work-branch">
                <header>
                  <span className="journey-work-branch__index">03</span>
                  <div>
                    <strong>Conversational Mission formation</strong>
                    <small>Issue Slice · safe frontier</small>
                  </div>
                  <ExecutionStatus
                    label={safeFrontierStatus.label}
                    state={safeFrontierStatus.state}
                  />
                </header>
                {sessions
                  .filter((session) => session.id === "formation-agent")
                  .map((session) => (
                    <button
                      aria-label={`Inspect ${session.agentName}: ${session.task}`}
                      className="journey-agent-session"
                      key={session.id}
                      onClick={() => setSelectedSessionId(session.id)}
                      type="button"
                    >
                      <span className="journey-agent-session__avatar">Q</span>
                      <span>
                        <strong>{session.agentName}</strong>
                        <small>{session.role} · {session.activity}</small>
                      </span>
                      <ExecutionStatus
                        label={sessionStatusLabel(session.status)}
                        state={session.status}
                      />
                    </button>
                  ))}
                {journey.planning !== "published" && (
                  <p className="journey-no-session">No Local Agent session started yet.</p>
                )}
              </article>

              <article className="journey-work-branch journey-work-branch--blocked">
                <header>
                  <span className="journey-work-branch__index">04</span>
                  <div>
                    <strong>Backend migration boundary</strong>
                    <small>Blocked by architecture verdict</small>
                  </div>
                  <ExecutionStatus label="Blocked" state="blocked" />
                </header>
              </article>
            </section>
          )}

          {selectedSession && (
            <section className="journey-session-inspector" aria-live="polite">
              <button
                className="journey-session-inspector__back"
                onClick={() => setSelectedSessionId(null)}
                type="button"
              >
                ← Mission Execution Tree
              </button>
              <header>
                <div>
                  <span>Selected agent session</span>
                  <h2>{selectedSession.agentName}</h2>
                  <p>{selectedSession.workTitle}</p>
                </div>
                <ExecutionStatus
                  label={sessionStatusLabel(selectedSession.status)}
                  state={selectedSession.status}
                />
              </header>
              <dl>
                <div>
                  <dt>Role</dt>
                  <dd>{selectedSession.role}</dd>
                </div>
                <div>
                  <dt>Model</dt>
                  <dd>{selectedSession.model}</dd>
                </div>
                <div>
                  <dt>Assigned task</dt>
                  <dd>{selectedSession.task}</dd>
                </div>
                <div>
                  <dt>Doing now</dt>
                  <dd>{selectedSession.activity}</dd>
                </div>
              </dl>
              <div className="journey-session-context">
                <div className="journey-session-latest">
                  <strong>Latest update</strong>
                  <p>{selectedSession.latestUpdate}</p>
                </div>
                <div className="journey-session-activity">
                  <strong>Session activity</strong>
                  <ol>
                    {selectedSession.activityTrail.map((activity) => (
                      <li key={activity}>{activity}</li>
                    ))}
                  </ol>
                </div>
              </div>
              <AgentOutputStream session={selectedSession} />
            </section>
          )}
        </aside>
      </main>
    </PrototypeChrome>
  );
}

function JourneyCard({
  index,
  active,
  complete,
  title,
  subtitle,
  children,
}: {
  readonly index: string;
  readonly active: boolean;
  readonly complete: boolean;
  readonly title: string;
  readonly subtitle: string;
  readonly children: ReactNode;
}) {
  return (
    <article
      className="journey-canvas-card"
      data-active={active}
      data-complete={complete}
    >
      <header>
        <span>{index}</span>
        <div>
          <strong>{title}</strong>
          <small>{subtitle}</small>
        </div>
        <em>{complete ? "Set" : active ? "Now" : "Later"}</em>
      </header>
      <div>{children}</div>
    </article>
  );
}

function JourneyCanvasVariant({ journey, actions }: VariantProps) {
  const step = activeStep(journey);
  return (
    <PrototypeChrome
      eyebrow="Variant B"
      title="Journey canvas"
      summary="The whole route is visible up front; the Mission Commander works left to right."
    >
      <main className="journey-canvas-layout">
        <header className="journey-canvas-heading">
          <div>
            <span>Formation canvas</span>
            <h1>From a directory to governed work</h1>
          </div>
          <p>
            Each gate answers one question. Nothing launches until the publication
            boundary is acknowledged.
          </p>
        </header>

        <section className="journey-canvas" aria-label="Mission formation journey canvas">
          <JourneyCard
            index="01"
            title="Establish"
            subtitle="Starting Location → Coding Workspace"
            active={step === 0}
            complete={journey.workspace !== "none"}
          >
            <p className="journey-canvas-value">{workspaceLabel(journey.workspace)}</p>
            {step === 0 ? (
              <div className="journey-card-actions">
                <button onClick={() => actions.selectWorkspace("existing")} type="button">
                  Select Alfredo
                </button>
                <button onClick={() => actions.selectWorkspace("new")} type="button">
                  Create new
                </button>
              </div>
            ) : (
              <small>Repository root confirmed; installation paths stay separate.</small>
            )}
          </JourneyCard>

          <JourneyCard
            index="02"
            title="Orient"
            subtitle="Resume without duplication"
            active={step === 1}
            complete={journey.mission !== "none"}
          >
            <p className="journey-canvas-value">{missionLabel(journey.mission)}</p>
            {step === 1 ? (
              <div className="journey-card-actions">
                {journey.workspace === "existing" && (
                  <button onClick={() => actions.selectMission("resume")} type="button">
                    Resume known Mission
                  </button>
                )}
                <button onClick={() => actions.selectMission("new")} type="button">
                  Start separate Mission
                </button>
              </div>
            ) : (
              <small>The Mission owns Shared Context; the Workspace Session stays continuous.</small>
            )}
          </JourneyCard>

          <JourneyCard
            index="03"
            title="Route"
            subtitle="Explain, recommend, override"
            active={step === 2}
            complete={journey.route !== "none"}
          >
            <p className="journey-canvas-value">{routeLabel(journey.route)}</p>
            {step === 2 ? (
              <RouteChoices journey={journey} actions={actions} />
            ) : (
              <small>
                {journey.routeOverridden
                  ? "The Mission Commander override is explicit."
                  : "Alfredo's recommendation is explicit."}
              </small>
            )}
          </JourneyCard>

          <JourneyCard
            index="04"
            title="Plan"
            subtitle="Turn decisions into a publication"
            active={step === 3 || step === 4}
            complete={journey.planning === "published"}
          >
            <div className="journey-artifact-stack">
              <span>Destination + scope</span>
              <span>Decisions + unresolved fog</span>
              <span>PRD + Issue Graph</span>
            </div>
            <PublicationActions journey={journey} actions={actions} />
          </JourneyCard>

          <JourneyCard
            index="05"
            title="Dispatch"
            subtitle="Only the safe frontier"
            active={step === 5}
            complete={journey.frontier === "running"}
          >
            <div className="journey-frontier-pulse">
              <span data-state={journey.frontier}>Ready · Workspace contract</span>
              <span data-state={journey.frontier}>Ready · Formation route</span>
              <span>Blocked · Backend migration</span>
            </div>
            {journey.planning === "published" && (
              <PublicationActions journey={journey} actions={actions} />
            )}
          </JourneyCard>
        </section>

        <footer className="journey-canvas-footer">
          <StateReadout journey={journey} />
        </footer>
      </main>
    </PrototypeChrome>
  );
}

function TreeNode({
  depth = 0,
  state,
  title,
  detail,
}: {
  readonly depth?: number;
  readonly state: "waiting" | "active" | "ready" | "running" | "blocked";
  readonly title: string;
  readonly detail: string;
}) {
  return (
    <div
      className="journey-tree-node"
      data-depth={depth}
      data-state={state}
      style={{ "--tree-depth": depth } as React.CSSProperties}
    >
      <span aria-hidden="true" />
      <div>
        <strong>{title}</strong>
        <small>{detail}</small>
      </div>
      <em>{state}</em>
    </div>
  );
}

function ExecutionTreeVariant({ journey, actions }: VariantProps) {
  const step = activeStep(journey);
  return (
    <PrototypeChrome
      eyebrow="Variant C"
      title="Execution tree"
      summary="Work is the primary object from the first screen; decisions fill in the tree in place."
    >
      <main className="journey-tree-layout">
        <aside className="journey-tree-sidebar">
          <header>
            <span>Mission Execution Tree</span>
            <strong>
              {journey.mission === "none"
                ? "Mission not established"
                : "Reliable Alfredo modernization"}
            </strong>
          </header>
          <div className="journey-tree">
            <TreeNode
              state={journey.workspace === "none" ? "active" : "ready"}
              title="Coding Workspace"
              detail={workspaceLabel(journey.workspace)}
            />
            <TreeNode
              depth={1}
              state={journey.mission === "none" ? "waiting" : "ready"}
              title="Mission"
              detail={missionLabel(journey.mission)}
            />
            <TreeNode
              depth={2}
              state={journey.route === "none" ? "waiting" : "ready"}
              title="Formation Route"
              detail={routeLabel(journey.route)}
            />
            <TreeNode
              depth={2}
              state={journey.planning === "published" ? "ready" : "waiting"}
              title="Product Requirements Document"
              detail={
                journey.planning === "published"
                  ? "Published and linked"
                  : "Appears when planning is clear"
              }
            />
            <TreeNode
              depth={2}
              state={journey.planning === "published" ? "ready" : "waiting"}
              title="Issue Graph"
              detail={
                journey.planning === "published"
                  ? "3 ready · 2 blocked"
                  : "No accepted Issue Slices yet"
              }
            />
            <TreeNode
              depth={3}
              state={journey.frontier === "running" ? "running" : "waiting"}
              title="Workspace selection contract"
              detail="Ready frontier · local-worker-01"
            />
            <TreeNode
              depth={3}
              state={journey.frontier === "running" ? "running" : "waiting"}
              title="Mission Formation Route"
              detail="Ready frontier · local-worker-02"
            />
            <TreeNode
              depth={3}
              state="blocked"
              title="Backend migration"
              detail="Blocked by architecture verdict"
            />
          </div>
        </aside>

        <section className="journey-tree-inspector">
          <header>
            <div>
              <span>Next governed decision</span>
              <h1>
                {step === 0 && "Choose a Coding Workspace"}
                {step === 1 && "Resume or start a Mission"}
                {step === 2 && "Confirm the Formation Route"}
                {(step === 3 || step === 4) && "Complete the planning boundary"}
                {step === 5 && "Approve the safe frontier"}
                {step === 6 && "Supervise running work"}
              </h1>
            </div>
            <em>Nothing runs without acknowledgement</em>
          </header>

          {step === 0 && (
            <div className="journey-tree-decision">
              <p>
                Alfredo started in <code>~/Projects</code>. Establish a repository root
                before Missions or Issue Slices can be loaded.
              </p>
              <div className="journey-large-actions">
                <button onClick={() => actions.selectWorkspace("existing")} type="button">
                  <span>Open Alfredo</span>
                  <small>Known repository · one resumable Mission</small>
                </button>
                <button onClick={() => actions.selectWorkspace("new")} type="button">
                  <span>Create Coding Workspace</span>
                  <small>New repository · no accepted Mission state</small>
                </button>
              </div>
            </div>
          )}

          {step === 1 && (
            <div className="journey-tree-decision">
              <p>
                This Coding Workspace already has bounded Mission state. Continuing the
                Workspace Session must not silently create a duplicate.
              </p>
              <div className="journey-large-actions">
                {journey.workspace === "existing" && (
                  <button onClick={() => actions.selectMission("resume")} type="button">
                    <span>Resume reliable modernization</span>
                    <small>12 decisions · 3 frontier items · last active today</small>
                  </button>
                )}
                <button onClick={() => actions.selectMission("new")} type="button">
                  <span>Start New Mission</span>
                  <small>Separate Shared Context and Issue Graph</small>
                </button>
              </div>
            </div>
          )}

          {step === 2 && (
            <div className="journey-tree-decision">
              <div className="journey-recommendation">
                <span>Alfredo recommends</span>
                <strong>Multi-session Wayfinding</strong>
                <p>
                  Five decision areas, unresolved dependencies, and a migration rollback
                  boundary make a one-shot task unsafe.
                </p>
              </div>
              <RouteChoices journey={journey} actions={actions} />
            </div>
          )}

          {(step === 3 || step === 4) && (
            <div className="journey-tree-decision">
              <div className="journey-publication-review">
                <div>
                  <span>Planning evidence</span>
                  <strong>Route is {step === 3 ? "still open" : "clear"}</strong>
                </div>
                <ul>
                  <li>Destination and explicit scope boundary</li>
                  <li>Decision index linked to resolution evidence</li>
                  <li>Ordered Issue Slices with native blockers</li>
                  <li>Rollback and acceptance seams remain explicit</li>
                </ul>
              </div>
              <PublicationActions journey={journey} actions={actions} />
            </div>
          )}

          {step >= 5 && (
            <div className="journey-tree-decision">
              <div className="journey-frontier-review">
                <span>Safe frontier</span>
                <strong>2 of 5 Issue Slices are launchable</strong>
                <p>
                  Alfredo will dispatch only approved, unblocked Issue Slices to eligible
                  Local Agents. Blocked work stays visible in the tree.
                </p>
              </div>
              <PublicationActions journey={journey} actions={actions} />
            </div>
          )}

          <footer>
            <StateReadout journey={journey} />
          </footer>
        </section>
      </main>
    </PrototypeChrome>
  );
}

function PrototypeSwitcher({
  current,
  onChange,
  onReset,
}: {
  readonly current: VariantKey;
  readonly onChange: (variant: VariantKey) => void;
  readonly onReset: () => void;
}) {
  const currentIndex = VARIANTS.findIndex((variant) => variant.key === current);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (
        target?.matches("input, textarea, select, [contenteditable='true']") ||
        (event.key !== "ArrowLeft" && event.key !== "ArrowRight")
      ) {
        return;
      }
      event.preventDefault();
      const offset = event.key === "ArrowLeft" ? -1 : 1;
      const nextIndex = (currentIndex + offset + VARIANTS.length) % VARIANTS.length;
      onChange(VARIANTS[nextIndex].key);
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [currentIndex, onChange]);

  const move = (offset: number) => {
    const nextIndex = (currentIndex + offset + VARIANTS.length) % VARIANTS.length;
    onChange(VARIANTS[nextIndex].key);
  };

  return (
    <div className="journey-prototype-switcher" role="group" aria-label="Prototype variant">
      <button aria-label="Previous prototype variant" onClick={() => move(-1)} type="button">
        ←
      </button>
      <strong>
        {VARIANTS[currentIndex].key} — {VARIANTS[currentIndex].name}
      </strong>
      <button aria-label="Next prototype variant" onClick={() => move(1)} type="button">
        →
      </button>
      <button className="journey-prototype-switcher__reset" onClick={onReset} type="button">
        Reset conversation
      </button>
    </div>
  );
}

export function WorkspaceMissionJourneyPrototype() {
  const [variant, setVariant] = useState<VariantKey>(variantFromLocation);
  const [journey, setJourney] = useState<JourneyState>(CONVERSATIONAL_DEMO_JOURNEY);

  const actions = useMemo<JourneyActions>(
    () => ({
      selectWorkspace: (workspace) =>
        setJourney({
          ...INITIAL_JOURNEY,
          workspace,
        }),
      selectMission: (mission) =>
        setJourney((current) => ({
          ...current,
          mission,
          route: "none",
          routeOverridden: false,
          planning: "not-started",
          frontier: "waiting",
        })),
      selectRoute: (route) =>
        setJourney((current) => ({
          ...current,
          route,
          routeOverridden: route !== RECOMMENDED_ROUTE,
          planning: "planning",
          frontier: "waiting",
        })),
      finishPlanning: () =>
        setJourney((current) => ({
          ...current,
          planning: "ready",
          frontier: "waiting",
        })),
      publishPlan: () =>
        setJourney((current) => ({
          ...current,
          planning: "published",
          frontier: "ready",
        })),
      startFrontier: () =>
        setJourney((current) => ({
          ...current,
          frontier: "running",
        })),
      sendConversationMessage: (message) =>
        setJourney((current) => {
          const step = activeStep(current);
          const next = {
            ...current,
            conversationMessages: [...current.conversationMessages, message],
          };
          if (consoleCommand(message)) return next;
          if (step === 0) return { ...next, workspace: "existing" };
          if (step === 1) return { ...next, mission: "resume" };
          if (step === 2) {
            return {
              ...next,
              route: "wayfinding",
              routeOverridden: false,
              planning: "planning",
            };
          }
          if (step === 3) return { ...next, planning: "ready" };
          if (step === 4) {
            return {
              ...next,
              planning: "published",
              frontier: "ready",
            };
          }
          if (step === 5) return { ...next, frontier: "running" };
          return next;
        }),
      reset: () => setJourney(CONVERSATIONAL_DEMO_JOURNEY),
    }),
    [],
  );

  const chooseVariant = (nextVariant: VariantKey) => {
    const url = new URL(window.location.href);
    url.searchParams.set("variant", nextVariant);
    window.history.replaceState({}, "", url);
    setVariant(nextVariant);
  };

  return (
    <>
      {variant === "A" && <GuidedConversationVariant journey={journey} actions={actions} />}
      {variant === "B" && <JourneyCanvasVariant journey={journey} actions={actions} />}
      {variant === "C" && <ExecutionTreeVariant journey={journey} actions={actions} />}
      <PrototypeSwitcher current={variant} onChange={chooseVariant} onReset={actions.reset} />
    </>
  );
}
