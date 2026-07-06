/// <reference types="node" />
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { App } from "./App";
import type {
  AgentConsoleMessageRequest,
  MissionDraftProjection,
  ReviewWorkspaceProjection,
  WorkspaceActionResult,
  ActivityJournalFilters,
  ActivityJournalProjection,
  WorkspaceMissionSwitchRequest,
  WorkspaceQueueProjection,
  WorkspaceScopeRequest,
  WorkspaceSnapshot,
  WorkingContextCurationRequest,
  WorkingContextProjection,
} from "./contracts";
import type { WorkspaceClient } from "./workspace-client";

const snapshot: WorkspaceSnapshot = {
      schema_version: 1,
      revision: 4,
      workspace_session: {
        id: "workspace-command-deck",
        workspace_path: "/workspace/albert",
        status: "ready",
      },
      active_mission: {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
      },
      conversation_scope: {
        kind: "issue-slice",
        target_id: "ISS-01",
        label: "Restore workspace session",
      },
      operations_view: "mission-board",
      mission_board: {
        prd_title: "Command Deck Mission",
        issue_count: 3,
        ordered_issue_ids: ["ISS-01", "ISS-02", "ISS-03"],
        ready_issue_ids: ["ISS-01"],
        approved_issue_ids: [],
      },
};

const client: WorkspaceClient = {
  loadSnapshot: async () => ({ kind: "ready", snapshot }),
};

const stylesSource = readFileSync(`${process.cwd()}/src/styles.css`, "utf8");

beforeEach(() => {
  window.localStorage.clear();
});

function cssVariable(name: string): string {
  const match = stylesSource.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`));
  if (!match) throw new Error(`Missing CSS variable --${name}`);
  return match[1];
}

function cssRootColor(property: string): string {
  return cssRuleColor(":root", property);
}

function cssRuleColor(selector: string, property: string): string {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const blocks = stylesSource.matchAll(new RegExp(`${escapedSelector}\\s*\\{([^}]*)\\}`, "g"));
  for (const block of blocks) {
    const propertyMatch = block[1].match(new RegExp(`${property}:\\s*(#[0-9a-fA-F]{6})`));
    if (propertyMatch) return propertyMatch[1];
  }
  throw new Error(`Missing CSS color ${property} in ${selector}`);
}

function contrastRatio(foreground: string, background: string): number {
  const [lighter, darker] = [relativeLuminance(foreground), relativeLuminance(background)].sort(
    (left, right) => right - left,
  );
  return (lighter + 0.05) / (darker + 0.05);
}

function relativeLuminance(hex: string): number {
  const value = Number.parseInt(hex.slice(1), 16);
  const [red, green, blue] = [
    (value >> 16) & 255,
    (value >> 8) & 255,
    value & 255,
  ].map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.03928
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

async function openCommandAudit() {
  fireEvent.click(screen.getByRole("button", { name: "Open command audit" }));
  return await screen.findByRole("region", { name: "Shell Terminal" });
}

function closeCommandAudit() {
  fireEvent.click(screen.getByRole("button", { name: "Close command audit", expanded: true }));
}

test("opens to a console-first workstation with persistent Mission Work beside it", async () => {
  const workstationSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-01-1",
            issue_id: "ISS-01",
            assigned_agent: "qwen-coder-local",
            status: "running",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
        ],
        attention: [
          {
            attention_id: "queue-ISS-02",
            mission_id: "command-deck",
            kind: "delegation-approval",
            label: "ISS-02 delegation approval required",
            queue_link: "workspace-queue#queue-ISS-02",
          },
        ],
      },
    ],
  };
  const history: AgentConsoleMessageRequest[] = [];
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: workstationSnapshot }),
        loadConsoleHistory: async () => ({
          kind: "history",
          history: {
            schema_version: 1,
            messages: [
              {
                message_id: "console-000001",
                sequence: 1,
                role: "user",
                content: "Implement the next Alfredo workstation slice.",
                scope: snapshot.conversation_scope,
                outcome: "proposed",
                source: "mission-commander",
              },
              {
                message_id: "console-000002",
                sequence: 2,
                role: "assistant",
                content: "I will keep the prompt transcript durable and route execution through the Orchestrator.",
                scope: snapshot.conversation_scope,
                outcome: "model-commentary",
                source: "frontier-model",
              },
            ],
          },
        }),
        appendConsoleMessage: async (message) => {
          history.push(message);
          return {
            kind: "message",
            message: {
              message_id: "console-000003",
              sequence: 3,
              role: message.role,
              content: message.content,
              scope: snapshot.conversation_scope,
              outcome: message.outcome,
              source: message.source,
            },
          };
        },
      }}
    />,
  );

  expect(await screen.findByRole("main", { name: "Prompt Workstation" })).toBeVisible();
  expect(screen.getByText(/Agent Console/)).toBeVisible();
  expect(stylesSource).toMatch(
    /\.deck-grid\s*\{[^}]*grid-template-columns:\s*minmax\(520px,\s*1\.7fr\)\s+minmax\(320px,\s*0\.75fr\)/s,
  );
  const transcript = screen.getByRole("region", { name: "Prompt Transcript" });
  expect(within(transcript).getByText("Implement the next Alfredo workstation slice.")).toBeVisible();
  expect(within(transcript).getByText(/durable and route execution/)).toBeVisible();
  expect(within(transcript).getByText("Workstation action pending: ISS-02 delegation approval required.")).toBeVisible();
  expect(within(transcript).getByText("Workstation outcome: ISS-01 is running on qwen-coder-local.")).toBeVisible();
  expect(screen.getByRole("complementary", { name: "Mission Work" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Mission Work" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Active Workstations" })).toBeVisible();
  expect(screen.queryByRole("tablist", { name: "Agent Workstation views" })).not.toBeInTheDocument();
  expect(screen.queryByRole("tab", { name: "Workstations" })).not.toBeInTheDocument();
  expect(screen.queryByRole("tab", { name: "Shell Terminal" })).not.toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Shell Terminal" })).not.toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Command history" })).not.toBeInTheDocument();
  expect(screen.queryByRole("textbox", { name: "Requested paths" })).not.toBeInTheDocument();
  expect(screen.queryByRole("combobox", { name: "Access level" })).not.toBeInTheDocument();
  expect(screen.queryByRole("textbox", { name: "Grant path" })).not.toBeInTheDocument();
  const cards = screen.getByRole("region", { name: "Workstation Cards" });
  expect(cards).toBeVisible();
  expect(screen.getByText("session-ISS-01-1")).toBeVisible();
  expect(screen.getByText("qwen3.6:27b")).toBeVisible();
  expect(within(cards).getByText("ISS-02 delegation approval required")).toBeVisible();
  const runningCard = within(cards).getByRole("article", { name: "qwen-coder-local workstation card" });
  expect(within(runningCard).getByText("Issue Slice")).toBeVisible();
  expect(within(runningCard).getAllByText("ISS-01").length).toBeGreaterThan(0);

  const statusLine = screen.getByLabelText("Prompt status line");
  expect(within(statusLine).getByText("Connection Connected")).toBeVisible();
  expect(within(statusLine).getByText("Conversation Scope Restore workspace session")).toBeVisible();
  expect(within(statusLine).getByText("Workspace /workspace/albert")).toBeVisible();
  expect(within(statusLine).getByText("Execution Waiting approval")).toBeVisible();
  expect(screen.getByRole("textbox", { name: "Message Alfredo" })).toBeVisible();

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Continue from the current issue." },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  await waitFor(() => expect(history).toHaveLength(1));
  expect(history[0]).toMatchObject({
    role: "user",
    content: "Continue from the current issue.",
    source: "mission-commander",
    scope_kind: "issue-slice",
  });
});

test("keeps review-ready workstation evidence affordances visible beside the Agent Console", async () => {
  const reviewReadySnapshot: WorkspaceSnapshot = {
    ...snapshot,
    mission_board: {
      ...snapshot.mission_board,
      issue_slices: [
        {
          issue_id: "ISS-04",
          title: "Mission Work pane active cards",
          lifecycle: "Approved",
          progress: "Evidence package ready for review",
          launch_eligible: false,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Create Mission Work active workstation cards.",
            acceptance_criteria: ["Review-ready cards expose evidence and review controls."],
            evidence_requirements: ["Frontend projection test passes."],
            source_path:
              ".scratch/alfredo-console-first-workstation-redesign/issues/04-mission-work-pane-active-workstation-cards.md",
          },
          sessions: [
            {
              session_id: "session-ISS-04-1",
              assigned_agent: "layout-subagent",
              role: "subagent",
              provider: "ollama",
              model: "qwen2.5-coder:14b",
              status: "evidence-ready",
              stale: false,
              disconnected: false,
              operation_status: "completed",
              failure: "",
            },
          ],
          provenance: {
            role: "subagent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
          },
          model_assignment: {
            agent_id: "layout-subagent",
            role: "subagent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
            availability: "available",
            availability_reason: "",
            operation_status: "completed",
            failure: "",
          },
          evidence: {
            state: "complete",
            changed_files: ["mission-control/src/App.tsx"],
            commands_run: ["npm test -- --run App.test.tsx"],
            test_results: "App tests passed.",
            risks: "None recorded.",
            artifact_links: ["app-local://evidence/session-ISS-04-1"],
          },
          working_context_sources: [],
        },
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-04-1",
            issue_id: "ISS-04",
            assigned_agent: "layout-subagent",
            status: "evidence-ready",
            role: "subagent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
          },
        ],
        attention: [],
      },
    ],
  };

  render(<App client={{ loadSnapshot: async () => ({ kind: "ready", snapshot: reviewReadySnapshot }) }} />);

  expect(await screen.findByRole("main", { name: "Prompt Workstation" })).toBeVisible();
  const cards = screen.getByRole("region", { name: "Workstation Cards" });
  const card = within(cards).getByRole("article", { name: "layout-subagent workstation card" });
  expect(within(card).getByText("Issue Slice")).toBeVisible();
  expect(within(card).getAllByText("ISS-04").length).toBeGreaterThan(0);
  expect(within(card).getByText("review-ready")).toBeVisible();
  expect(within(card).getAllByText("npm test -- --run App.test.tsx").length).toBeGreaterThan(0);

  fireEvent.click(within(card).getByRole("button", { name: "Expand layout-subagent" }));

  expect(within(card).getByRole("region", { name: "layout-subagent operational detail" })).toBeVisible();
  expect(within(card).getByText("Evidence Packages")).toBeVisible();
  expect(within(card).getByRole("link", { name: "Evidence Package session-ISS-04-1" })).toHaveAttribute(
    "href",
    "app-local://evidence/session-ISS-04-1",
  );
  expect(within(card).getByText("Accept evidence")).toBeVisible();
  expect(within(card).getByText("Request repair")).toBeVisible();
  expect(within(card).getByText("Reason required")).toBeVisible();
  expect(within(card).getAllByText("Use Review Workspace governed controls").length).toBeGreaterThan(0);
});

test("restores workstation card state and side-pane selection after desktop refresh", async () => {
  const continuitySnapshot: WorkspaceSnapshot = {
    ...snapshot,
    mission_board: {
      ...snapshot.mission_board,
      issue_slices: [
        {
          issue_id: "ISS-01",
          title: "Restore workspace session",
          lifecycle: "Approved",
          progress: "Evidence package ready for review",
          launch_eligible: false,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Restore the workspace session.",
            acceptance_criteria: ["Continuity restores card state."],
            evidence_requirements: ["Frontend refresh test."],
            source_path: ".agent/issues/27-persist-alfredo-workstation-continuity.md",
          },
          sessions: [
            {
              session_id: "session-ISS-01-1",
              assigned_agent: "qwen-coder-local",
              role: "local-agent",
              provider: "ollama",
              model: "qwen3.6:27b",
              status: "evidence-ready",
              stale: false,
              disconnected: false,
              operation_status: "completed",
              failure: "",
            },
          ],
          provenance: {
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
          model_assignment: {
            agent_id: "qwen-coder-local",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "completed",
            failure: "",
          },
          evidence: {
            state: "accepted",
            changed_files: ["mission-control/src/App.tsx"],
            commands_run: ["npm test -- App.test.tsx"],
            test_results: "App continuity tests passed.",
            risks: "None recorded.",
            artifact_links: ["app-local://evidence/session-ISS-01-1"],
          },
          working_context_sources: [],
        },
        {
          issue_id: "ISS-02",
          title: "Keep selected issue visible",
          lifecycle: "Approved",
          progress: "Issue selection restored from local continuity.",
          launch_eligible: false,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Restore the selected Issue Slice.",
            acceptance_criteria: ["Desktop refresh returns to the selected issue."],
            evidence_requirements: ["Frontend refresh test."],
            source_path: ".agent/issues/27-persist-alfredo-workstation-continuity.md",
          },
          sessions: [],
          provenance: {
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
          model_assignment: {
            agent_id: "qwen-coder-local",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "not-started",
            failure: "",
          },
          evidence: {
            state: "missing",
            changed_files: [],
            commands_run: [],
            test_results: "",
            risks: "",
            artifact_links: [],
          },
          working_context_sources: [],
        },
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-01-1",
            issue_id: "ISS-01",
            assigned_agent: "qwen-coder-local",
            status: "evidence-ready",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
        ],
        attention: [],
      },
    ],
  };
  const continuityClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot: continuitySnapshot }),
    loadLaunchContext: async () => ({
      kind: "launch-context",
      context: {
        selected_agent: "qwen3.6-27b",
        selected_model: "qwen3.6:27b",
        selected_workspace: "/workspace/albert",
        runtime_root: "/runtime/alfredo",
        recent_workspaces: ["/workspace/albert"],
      },
    }),
  };

  const first = render(<App client={continuityClient} />);
  const cards = await screen.findByRole("region", { name: "Workstation Cards" });
  fireEvent.change(within(cards).getByRole("searchbox", { name: "Filter workstation cards" }), {
    target: { value: "qwen" },
  });
  fireEvent.change(within(cards).getByRole("combobox", { name: "Sort workstation cards" }), {
    target: { value: "name" },
  });
  fireEvent.click(within(cards).getByRole("button", { name: "Expand qwen-coder-local" }));
  fireEvent.click(within(cards).getByRole("button", { name: "Pin qwen-coder-local" }));
  fireEvent.click(screen.getByRole("button", { name: "Inspect ISS-02" }));
  fireEvent.click(within(cards).getByRole("button", { name: "Select session session-ISS-01-1" }));
  fireEvent.click(within(cards).getByRole("button", { name: "Open diff mission-control/src/App.tsx" }));
  await openCommandAudit();

  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Close command audit", expanded: true })).toBeVisible(),
  );
  first.unmount();

  render(<App client={continuityClient} />);
  const restoredCards = await screen.findByRole("region", { name: "Workstation Cards" });

  await waitFor(() =>
    expect(within(restoredCards).getByRole("searchbox", { name: "Filter workstation cards" })).toHaveValue(
      "qwen",
    ),
  );
  expect(within(restoredCards).getByRole("combobox", { name: "Sort workstation cards" })).toHaveValue(
    "name",
  );
  expect(within(restoredCards).getByRole("button", { name: "Unpin qwen-coder-local" })).toBeVisible();
  expect(within(restoredCards).getByRole("region", { name: "qwen-coder-local operational detail" })).toBeVisible();
  expect(within(restoredCards).getByText("Selected session session-ISS-01-1")).toBeVisible();
  expect(within(restoredCards).getByText("Diff opened locally: mission-control/src/App.tsx")).toBeVisible();
  expect(screen.getByRole("button", { name: "Close command audit", expanded: true })).toBeVisible();
  closeCommandAudit();
  expect(screen.getByRole("region", { name: "Issue Slice Inspector" })).toHaveTextContent(
    "Keep selected issue visible",
  );
  expect(screen.getByText("Runtime /runtime/alfredo")).toBeVisible();
});

test("routes a waiting workstation card decision through typed queue acknowledgement", async () => {
  const queueItem = {
    item_id: "delegation-command-deck-ISS-02",
    mission_id: "command-deck",
    item_type: "ad-hoc-delegation" as const,
    status: "pending" as const,
    source: "agent-console",
    requested_action: "Approve ISS-02 delegation",
    affected_boundary: "launch-boundary",
    consequence: "Approval launches a local agent session.",
    issue_id: "ADHOC-000001",
    proposed_changes: {},
  };
  const before: WorkspaceSnapshot = {
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [],
        attention: [
          {
            attention_id: "delegation-command-deck-ISS-02",
            mission_id: "command-deck",
            kind: "delegation-approval",
            label: "ISS-02 delegation approval required",
            queue_link: "workspace-queue#delegation-command-deck-ISS-02",
          },
        ],
      },
    ],
  };
  const after: WorkspaceSnapshot = {
    ...before,
    revision: 5,
    missions: before.missions?.map((mission) => ({
      ...mission,
      attention: [],
      sessions: [
        {
          session_id: "session-ADHOC-000001-1",
          issue_id: "ADHOC-000001",
          assigned_agent: "qwen-coder-local",
          status: "launched",
        },
      ],
    })),
  };
  const queueProjection: WorkspaceQueueProjection = {
    schema_version: 1,
    revision: 2,
    items: [queueItem],
    groups: [
      {
        group_id: "ad-hoc-delegation:command-deck",
        item_type: "ad-hoc-delegation",
        mission_id: "command-deck",
        item_count: 1,
        items: [queueItem],
      },
    ],
  };
  const decisions: unknown[] = [];
  let snapshotLoads = 0;
  const actionClient: WorkspaceClient = {
    loadSnapshot: async () => {
      snapshotLoads += 1;
      return { kind: "ready", snapshot: snapshotLoads === 1 ? before : after };
    },
    loadWorkspaceQueue: async () => ({
      kind: "workspace-queue",
      projection:
        decisions.length === 0
          ? queueProjection
          : { ...queueProjection, revision: 3, items: [], groups: [] },
    }),
    submitWorkspaceQueueDecision: async (request) => {
      decisions.push(request);
      await new Promise((resolve) => setTimeout(resolve, 0));
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 3,
          item_id: request.item_id,
          item_status: "approved",
          effect_summary: "Approved delegation-command-deck-ISS-02; launched ADHOC-000001.",
        },
      };
    },
  };

  render(<App client={actionClient} />);

  const cards = await screen.findByRole("region", { name: "Workstation Cards" });
  expect(
    await within(cards).findByRole("button", {
      name: "Approve delegation-command-deck-ISS-02",
    }),
  ).toBeEnabled();
  expect(within(cards).getByRole("button", { name: "Reject delegation-command-deck-ISS-02" })).toBeDisabled();
  expect(within(cards).getByRole("button", { name: "Defer delegation-command-deck-ISS-02" })).toBeDisabled();

  fireEvent.click(within(cards).getByRole("button", { name: "Approve delegation-command-deck-ISS-02" }));

  expect(
    await within(cards).findByRole("status", {
      name: "ISS-02 delegation approval required workstation action state",
    }),
  ).toHaveTextContent("pending");
  expect(screen.getByText("Orchestrator validating workstation action.")).toBeVisible();

  await waitFor(() =>
    expect(decisions).toEqual([
      {
        correlation_id: "queue-approve-delegation-command-deck-ISS-02-2",
        action_type: "workspace-queue-decision",
        actor: "mission-commander",
        expected_revision: 2,
        target: {
          kind: "workspace-queue-item",
          id: "delegation-command-deck-ISS-02",
        },
        item_id: "delegation-command-deck-ISS-02",
        decision: "approve",
        reason: "",
      },
    ]),
  );
  expect(await screen.findByText(/Orchestrator accepted workstation action/)).toBeVisible();
  expect(screen.getByText("session-ADHOC-000001-1")).toBeVisible();
});

test("launches an issue from a workstation card through typed Orchestrator action", async () => {
  const before: WorkspaceSnapshot = {
    ...snapshot,
    revision: 4,
    mission_board: {
      ...snapshot.mission_board,
      issue_slices: [
        {
          issue_id: "ISS-01",
          title: "Restore workspace session",
          lifecycle: "Ready",
          progress: "Launch eligible",
          launch_eligible: true,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Restore the workspace session.",
            acceptance_criteria: ["Canonical snapshot is visible."],
            evidence_requirements: [],
            source_path: ".agent/issues/01-restore.md",
          },
          sessions: [],
          provenance: {
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
          model_assignment: {
            agent_id: "qwen-coder-local",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "idle",
            failure: "",
          },
          evidence: {
            state: "not-started",
            changed_files: [],
            commands_run: [],
            test_results: "No evidence package recorded.",
            risks: "",
            artifact_links: [],
          },
          working_context_sources: [],
        },
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [],
        attention: [],
      },
    ],
  };
  const after: WorkspaceSnapshot = {
    ...before,
    revision: 5,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-01-1",
            issue_id: "ISS-01",
            assigned_agent: "qwen-coder-local",
            status: "launched",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
        ],
        attention: [],
      },
    ],
  };
  const actions: unknown[] = [];
  let snapshotLoads = 0;
  render(
    <App
      client={{
        loadSnapshot: async () => {
          snapshotLoads += 1;
          return { kind: "ready", snapshot: snapshotLoads === 1 ? before : after };
        },
        submitWorkstationAction: async (request) => {
          actions.push(request);
          return {
            kind: "acknowledged",
            acknowledgement: {
              correlation_id: request.correlation_id,
              outcome: "acknowledged",
              revision: 5,
              action_type: "issue-launch",
              issue_id: "ISS-01",
              session_id: "session-ISS-01-1",
              effect_summary: "Orchestrator launched ISS-01 as session-ISS-01-1.",
            },
          };
        },
      }}
    />,
  );

  const cards = await screen.findByRole("region", { name: "Workstation Cards" });
  fireEvent.click(await within(cards).findByRole("button", { name: "Launch ISS-01" }));

  await waitFor(() =>
    expect(actions).toEqual([
      {
        correlation_id: "workstation-issue-launch-ISS-01-4",
        action_type: "issue-launch",
        actor: "mission-commander",
        expected_revision: 4,
        target: { kind: "issue-slice", id: "ISS-01" },
        issue_id: "ISS-01",
        session_id: undefined,
        agent_id: undefined,
        reason: undefined,
        allowed_paths: [],
        command_policy: {},
      },
    ]),
  );
  expect(screen.getByText("Orchestrator validating workstation action.")).toBeVisible();
  expect(await screen.findByText(/Orchestrator accepted workstation action/)).toBeVisible();
  expect(screen.getByText("session-ISS-01-1")).toBeVisible();
});

test("changes model assignment from a workstation card with required agent and reason", async () => {
  const assignmentSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 6,
    mission_board: {
      ...snapshot.mission_board,
      issue_slices: [
        {
          issue_id: "ISS-01",
          title: "Restore workspace session",
          lifecycle: "Ready",
          progress: "Launch eligible",
          launch_eligible: true,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Restore the workspace session.",
            acceptance_criteria: ["Canonical snapshot is visible."],
            evidence_requirements: [],
            source_path: ".agent/issues/01-restore.md",
          },
          sessions: [],
          provenance: {
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
          model_assignment: {
            agent_id: "qwen-coder-local",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "idle",
            failure: "",
          },
          evidence: {
            state: "not-started",
            changed_files: [],
            commands_run: [],
            test_results: "No evidence package recorded.",
            risks: "",
            artifact_links: [],
          },
          working_context_sources: [],
        },
      ],
    },
  };
  const actions: unknown[] = [];
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: assignmentSnapshot }),
        submitWorkstationAction: async (request) => {
          actions.push(request);
          return {
            kind: "acknowledged",
            acknowledgement: {
              correlation_id: request.correlation_id,
              outcome: "acknowledged",
              revision: 7,
              action_type: "model-assignment-change",
              issue_id: "ISS-01",
              session_id: "",
              effect_summary: "Mission Commander assigned ISS-01 to qwen3.6-27b.",
            },
          };
        },
      }}
    />,
  );

  const cards = await screen.findByRole("region", { name: "Workstation Cards" });
  const button = await within(cards).findByRole("button", {
    name: "Change model assignment ISS-01",
  });
  expect(button).toBeDisabled();
  fireEvent.change(within(cards).getByRole("textbox", { name: "Workstation action agent ISS-01" }), {
    target: { value: "qwen3.6-27b" },
  });
  fireEvent.change(within(cards).getByRole("textbox", { name: "Workstation action reason ISS-01" }), {
    target: { value: "Use the stronger local model." },
  });
  expect(button).toBeEnabled();
  fireEvent.click(button);

  await waitFor(() =>
    expect(actions).toEqual([
      {
        correlation_id: "workstation-model-assignment-change-ISS-01-6",
        action_type: "model-assignment-change",
        actor: "mission-commander",
        expected_revision: 6,
        target: { kind: "issue-slice", id: "ISS-01" },
        issue_id: "ISS-01",
        session_id: undefined,
        agent_id: "qwen3.6-27b",
        reason: "Use the stronger local model.",
        allowed_paths: [],
        command_policy: {},
      },
    ]),
  );
  expect(await screen.findByText(/Orchestrator accepted workstation action/)).toBeVisible();
});

test("keeps expanded workstation navigation local to the side pane", async () => {
  const appendConsoleMessage = vi.fn();
  const workstationSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 8,
    mission_board: {
      ...snapshot.mission_board,
      issue_slices: [
        {
          issue_id: "ISS-01",
          title: "Restore workspace session",
          lifecycle: "Approved",
          progress: "Runner summarized operational detail",
          launch_eligible: false,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Restore the workspace session.",
            acceptance_criteria: ["Session detail remains inspectable."],
            evidence_requirements: ["Evidence package links are available."],
            source_path: ".agent/issues/ISS-01.md",
          },
          sessions: [
            {
              session_id: "session-ISS-01-1",
              assigned_agent: "qwen-coder-local",
              role: "local-agent",
              provider: "ollama",
              model: "qwen3.6:27b",
              status: "evidence-ready",
              stale: false,
              disconnected: false,
              operation_status: "completed",
              failure: "",
            },
          ],
          provenance: {
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
          model_assignment: {
            agent_id: "qwen-coder-local",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "completed",
            failure: "",
          },
          evidence: {
            state: "ready",
            changed_files: ["mission-control/src/App.tsx"],
            commands_run: ["npm test -- App.test.tsx"],
            test_results: "App tests passed.",
            risks: "Diff should be reviewed before acceptance.",
            artifact_links: ["app-local://evidence/session-ISS-01-1"],
          },
          working_context_sources: [],
        },
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-01-1",
            issue_id: "ISS-01",
            assigned_agent: "qwen-coder-local",
            status: "evidence-ready",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
        ],
        attention: [],
      },
    ],
  };
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: workstationSnapshot }),
        loadConsoleHistory: async () => ({
          kind: "history",
          history: {
            schema_version: 1,
            messages: [
              {
                message_id: "console-000001",
                sequence: 1,
                role: "user",
                content: "Keep this transcript clean.",
                scope: snapshot.conversation_scope,
                outcome: "proposed",
                source: "mission-commander",
              },
            ],
          },
        }),
        appendConsoleMessage,
      }}
    />,
  );

  const transcript = await screen.findByRole("region", { name: "Prompt Transcript" });
  const cards = screen.getByRole("region", { name: "Workstation Cards" });
  fireEvent.change(screen.getByRole("searchbox", { name: "Filter workstation cards" }), {
    target: { value: "qwen" },
  });
  fireEvent.change(screen.getByRole("combobox", { name: "Sort workstation cards" }), {
    target: { value: "name" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Pin qwen-coder-local" }));
  fireEvent.click(screen.getByRole("button", { name: "Expand qwen-coder-local" }));

  expect(within(cards).getByText("Tool Activity")).toBeVisible();
  expect(within(cards).getAllByText("npm test -- App.test.tsx").length).toBeGreaterThan(0);
  expect(within(cards).getByRole("link", { name: "Evidence Package session-ISS-01-1" })).toHaveAttribute(
    "href",
    "app-local://evidence/session-ISS-01-1",
  );
  expect(within(cards).getByText("Diff should be reviewed before acceptance.")).toBeVisible();

  fireEvent.click(within(cards).getByRole("button", { name: "Select session session-ISS-01-1" }));
  expect(within(cards).getByText("Selected session session-ISS-01-1")).toBeVisible();

  fireEvent.click(within(cards).getByRole("button", { name: "Open diff mission-control/src/App.tsx" }));
  expect(within(cards).getByRole("status", { name: "Selected workstation diff" })).toHaveTextContent(
    "mission-control/src/App.tsx",
  );
  fireEvent.click(screen.getByRole("button", { name: "Collapse qwen-coder-local" }));

  expect(appendConsoleMessage).not.toHaveBeenCalled();
  expect(within(transcript).getByText("Keep this transcript clean.")).toBeVisible();
  expect(within(transcript).queryByText(/Selected session session-ISS-01-1/)).not.toBeInTheDocument();
  expect(within(transcript).queryByText(/mission-control\/src\/App.tsx/)).not.toBeInTheDocument();
});

test("opens command audit without mixing prompt and terminal drafts", async () => {
  render(<App client={client} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Keep this console draft" },
  });
  await openCommandAudit();

  expect(screen.getByRole("region", { name: "Workstation Cards" })).toBeVisible();
  expect(screen.getByRole("region", { name: "Shell Terminal" })).toBeVisible();

  fireEvent.change(screen.getByRole("textbox", { name: "Command" }), {
    target: { value: "python3 -m unittest --help" },
  });
  closeCommandAudit();

  expect(screen.getByRole("textbox", { name: "Message Alfredo" })).toHaveValue(
    "Keep this console draft",
  );
  expect(screen.queryByRole("region", { name: "Shell Terminal" })).not.toBeInTheDocument();

  await openCommandAudit();
  expect(screen.getByRole("textbox", { name: "Command" })).toHaveValue(
    "python3 -m unittest --help",
  );
});

test("shows running snapshot work as active execution near the prompt", async () => {
  const runningSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-01-running",
            issue_id: "ISS-01",
            assigned_agent: "qwen-coder-local",
            status: "running",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
        ],
        attention: [],
      },
    ],
  };

  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: runningSnapshot }),
      }}
    />,
  );

  const statusLine = await screen.findByLabelText("Prompt status line");
  expect(within(statusLine).getByText("Execution Session running")).toBeVisible();
});

test("shows selected controller and model near the prompt composer", async () => {
  const loadLaunchContext = vi.fn(async () => ({
    kind: "launch-context" as const,
    context: {
      selected_agent: "qwen3.6-27b",
      selected_model: "qwen3.6:27b",
      selected_workspace: "/workspace/albert",
      runtime_root: "/home/mission/.alfredo/runtime",
      recent_workspaces: ["/workspace/albert"],
    },
  }));

  render(<App client={{ ...client, loadLaunchContext }} />);

  await screen.findByRole("heading", { name: "Command Deck Mission" });
  const launchContext = await screen.findByLabelText("Prompt status line");
  expect(within(launchContext).getByText("Controller qwen3.6-27b")).toBeVisible();
  expect(within(launchContext).getByText("Model qwen3.6:27b")).toBeVisible();
  expect(within(launchContext).getByText("1 recent workspaces")).toBeVisible();
});

test("renders observed terminal commands inline without reconstructing terminal bytes by default", async () => {
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: 2,
      commands: [
        {
          command_id: "terminal-command-000001",
          correlation_id: "terminal-ui-1",
          command: "python3 -m unittest --help",
          classification: "auto-allowed" as const,
          status: "completed" as const,
          exit_code: 0,
          working_directory: "/workspace/albert",
          requested_paths: [],
          access_level: "read" as const,
          requester: "mission-commander",
          approver: "",
          decider: "",
          reason: "",
        },
      ],
      grants: [],
    },
  }));
  render(<App client={{ ...client, loadShellTerminal }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await waitFor(() => expect(loadShellTerminal).toHaveBeenCalledTimes(1));

  const transcript = screen.getByRole("region", { name: "Prompt Transcript" });
  expect(within(transcript).getByText("python3 -m unittest --help")).toBeVisible();
  expect(within(transcript).getByText(/auto-allowed \/ Auto-allowed by command policy/)).toBeVisible();
  expect(within(transcript).getByText("/workspace/albert")).toBeVisible();
  expect(within(transcript).getByText("Completed with exit 0 and no output.")).toBeVisible();
  expect(within(transcript).getByRole("button", { name: "Inspect full output for terminal-command-000001" })).toBeDisabled();
  expect(screen.queryByRole("region", { name: "Command history" })).not.toBeInTheDocument();
  expect(screen.queryByRole("textbox", { name: "Requested paths" })).not.toBeInTheDocument();

  await openCommandAudit();

  const terminal = await screen.findByRole("region", { name: "Shell Terminal" });
  expect(loadShellTerminal).toHaveBeenCalledTimes(2);
  expect(within(terminal).getByText("python3 -m unittest --help")).toBeVisible();
  expect(within(terminal).getByText("auto-allowed / completed")).toBeVisible();
  expect(within(terminal).queryByLabelText("Command output")).not.toBeInTheDocument();
});

test("submits auto-allowed terminal command and keeps output local to the session", async () => {
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: { schema_version: 1 as const, revision: 1, commands: [], grants: [] },
  }));
  const submitShellTerminalCommand = vi.fn(async () => ({
    kind: "command-result" as const,
    result: {
      command_id: "terminal-command-000002",
      correlation_id: "terminal-ui-2",
      classification: "auto-allowed" as const,
      status: "completed" as const,
      exit_code: 0,
      stdout: "usage: python3 -m unittest\n",
      stderr: "",
    },
  }));
  render(
    <App client={{ ...client, loadShellTerminal, submitShellTerminalCommand }} />,
  );
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await waitFor(() => expect(loadShellTerminal).toHaveBeenCalledTimes(1));
  await openCommandAudit();
  await waitFor(() => expect(loadShellTerminal).toHaveBeenCalledTimes(2));

  fireEvent.change(screen.getByRole("textbox", { name: "Command" }), {
    target: { value: "python3 -m unittest --help" },
  });
  fireEvent.change(screen.getByRole("textbox", { name: "Requested paths" }), {
    target: { value: "/workspace/albert/tests\n/workspace/albert/docs" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Run command" }));

  expect(submitShellTerminalCommand).toHaveBeenCalledWith({
    correlation_id: expect.stringMatching(/^terminal-command-/),
    command: "python3 -m unittest --help",
    working_directory: "/workspace/albert",
    requested_paths: ["/workspace/albert/tests", "/workspace/albert/docs"],
    requester: "mission-commander",
    access_level: "read",
  });
  expect(await screen.findByLabelText("Command output")).toHaveTextContent(
    "usage: python3 -m unittest",
  );
  expect(screen.getByRole("textbox", { name: "Command" })).toHaveValue("");
  expect(loadShellTerminal).toHaveBeenCalledTimes(3);

  closeCommandAudit();
  const transcript = screen.getByRole("region", { name: "Prompt Transcript" });
  expect(within(transcript).getByText("python3 -m unittest --help")).toBeVisible();
  expect(within(transcript).getByText(/Captured 1 stdout line/)).toBeVisible();
  expect(screen.queryByText("usage: python3 -m unittest")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Inspect full output for terminal-command-000002" }));
  expect(screen.getByLabelText("Full command output for terminal-command-000002")).toHaveTextContent(
    "usage: python3 -m unittest",
  );
});

function pendingTerminalCommand(classification: "human-required" | "frontier-approvable") {
  return {
    command_id: `terminal-${classification}`,
    correlation_id: `correlation-${classification}`,
    command: "git push origin main",
    classification,
    status: "pending-approval" as const,
    exit_code: null,
    working_directory: "/workspace/albert",
    requested_paths: [],
    access_level: "write" as const,
    requester: "mission-commander",
    approver: classification === "human-required" ? "mission-commander" : "frontier-model",
    decider: "",
    reason: "",
  };
}

test("approves a human-required terminal command as Mission Commander", async () => {
  const command = pendingTerminalCommand("human-required");
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: { schema_version: 1 as const, revision: 1, commands: [command], grants: [] },
  }));
  const decideShellTerminalCommand = vi.fn(async () => ({
    kind: "command-result" as const,
    result: {
      command_id: command.command_id,
      correlation_id: command.correlation_id,
      classification: command.classification,
      status: "completed" as const,
      exit_code: 0,
      stdout: "pushed\n",
      stderr: "",
    },
  }));
  render(<App client={{ ...client, loadShellTerminal, decideShellTerminalCommand }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await openCommandAudit();
  fireEvent.click(await screen.findByRole("button", { name: `Approve ${command.command_id}` }));

  expect(decideShellTerminalCommand).toHaveBeenCalledWith({
    command_id: command.command_id,
    decision: "approve",
    actor: "mission-commander",
    reason: "",
  });
  expect(await screen.findByLabelText("Command output")).toHaveTextContent("pushed");
  expect(loadShellTerminal).toHaveBeenCalledTimes(3);
});

test("requires a reason before denying a pending terminal command", async () => {
  const command = pendingTerminalCommand("human-required");
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: { schema_version: 1 as const, revision: 1, commands: [command], grants: [] },
  }));
  const decideShellTerminalCommand = vi.fn(async () => ({
    kind: "command-result" as const,
    result: {
      command_id: command.command_id,
      correlation_id: command.correlation_id,
      classification: command.classification,
      status: "denied" as const,
      exit_code: null,
      stdout: "",
      stderr: "",
    },
  }));
  render(<App client={{ ...client, loadShellTerminal, decideShellTerminalCommand }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await openCommandAudit();
  const deny = await screen.findByRole("button", { name: `Deny ${command.command_id}` });
  expect(deny).toBeDisabled();
  fireEvent.change(screen.getByRole("textbox", { name: `Denial reason ${command.command_id}` }), {
    target: { value: "Unsafe destination" },
  });
  fireEvent.click(deny);

  expect(decideShellTerminalCommand).toHaveBeenCalledWith({
    command_id: command.command_id,
    decision: "deny",
    actor: "mission-commander",
    reason: "Unsafe destination",
  });
  const terminal = screen.getByRole("region", { name: "Shell Terminal" });
  expect(await within(terminal).findByText("Command denied.")).toBeVisible();
  expect(screen.queryByLabelText("Command output")).not.toBeInTheDocument();
});

test("shows the Frontier approval boundary without false Mission Commander approval", async () => {
  const command = pendingTerminalCommand("frontier-approvable");
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: { schema_version: 1 as const, revision: 1, commands: [command], grants: [] },
  }));
  render(<App client={{ ...client, loadShellTerminal }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await openCommandAudit();

  expect(await screen.findByText("Awaiting Frontier Model approval")).toBeVisible();
  expect(screen.queryByRole("button", { name: `Approve ${command.command_id}` })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: `Deny ${command.command_id}` })).toBeDisabled();
});

test("handles command approval and contextual path grant requests inline in the Agent Console", async () => {
  const command = pendingTerminalCommand("human-required");
  const grant = {
    grant_id: "path-grant-000001",
    correlation_id: "path-grant-inline-1",
    path: "/external/docs",
    access_level: "write" as const,
    duration_seconds: 900,
    granted_by: "mission-commander" as const,
    granted_at: "2026-07-01T12:00:00Z",
    expires_at: "2099-07-01T12:15:00Z",
  };
  let grantCreated = false;
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: grantCreated ? 2 : 1,
      commands: [command],
      grants: grantCreated ? [grant] : [],
    },
  }));
  const decideShellTerminalCommand = vi.fn(async () => ({
    kind: "command-result" as const,
    result: {
      command_id: command.command_id,
      correlation_id: command.correlation_id,
      classification: command.classification,
      status: "completed" as const,
      exit_code: 0,
      stdout: "pushed\n",
      stderr: "",
    },
  }));
  const submitShellTerminalCommand = vi.fn(async () => ({
    kind: "command-rejected" as const,
    code: "invalid-action",
    message:
      "Shell Terminal working directory is outside the workspace and has no active write Additional Path Grant.",
  }));
  const createAdditionalPathGrant = vi.fn(async () => {
    grantCreated = true;
    return { kind: "path-grant" as const, grant };
  });
  render(
    <App
      client={{
        ...client,
        loadShellTerminal,
        decideShellTerminalCommand,
        submitShellTerminalCommand,
        createAdditionalPathGrant,
      }}
    />,
  );
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await waitFor(() => expect(loadShellTerminal).toHaveBeenCalledTimes(1));

  const transcript = screen.getByRole("region", { name: "Prompt Transcript" });
  const approvalPrompt = await within(transcript).findByRole("group", {
    name: `Approval prompt for ${command.command_id}`,
  });
  expect(within(approvalPrompt).getByText("git push origin main")).toBeVisible();
  expect(within(approvalPrompt).getByText("Access / write")).toBeVisible();
  fireEvent.click(within(approvalPrompt).getByRole("button", { name: `Approve ${command.command_id} inline` }));
  expect(decideShellTerminalCommand).toHaveBeenCalledWith({
    command_id: command.command_id,
    decision: "approve",
    actor: "mission-commander",
    reason: "",
  });
  expect(await within(transcript).findByText(/Orchestrator accepted workstation action: Command completed/)).toBeVisible();

  await openCommandAudit();
  fireEvent.change(screen.getByRole("textbox", { name: "Command" }), {
    target: { value: "touch report.md" },
  });
  fireEvent.change(screen.getByRole("textbox", { name: "Working directory" }), {
    target: { value: "/external/docs" },
  });
  fireEvent.change(screen.getByRole("textbox", { name: "Requested paths" }), {
    target: { value: "/external/docs/report.md" },
  });
  fireEvent.change(screen.getByRole("combobox", { name: "Access level" }), {
    target: { value: "write" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Run command" }));
  closeCommandAudit();

  const grantPrompt = await within(transcript).findByRole("group", {
    name: "Additional Path Grant request for /external/docs",
  });
  expect(within(grantPrompt).getByText("Path / /external/docs")).toBeVisible();
  expect(within(grantPrompt).getByText("Access / write")).toBeVisible();
  expect(within(grantPrompt).getByText("Duration / 900 seconds")).toBeVisible();
  expect(within(grantPrompt).getByText("Affected action / touch report.md")).toBeVisible();
  expect(within(grantPrompt).getByText(/no active write Additional Path Grant/)).toBeVisible();
  expect(screen.queryByRole("textbox", { name: "Grant path" })).not.toBeInTheDocument();

  fireEvent.click(within(grantPrompt).getByRole("button", { name: "Grant write access for /external/docs" }));
  expect(createAdditionalPathGrant).toHaveBeenCalledWith({
    correlation_id: expect.stringMatching(/^path-grant-/),
    expected_revision: 1,
    path: "/external/docs",
    access_level: "write",
    duration_seconds: 900,
    requester: "mission-commander",
  });
  expect(await within(transcript).findByText(/Orchestrator accepted workstation action: Created path-grant-000001/)).toBeVisible();
});

test("shows denied contextual path grants as Agent Console outcomes", async () => {
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: { schema_version: 1 as const, revision: 1, commands: [], grants: [] },
  }));
  const submitShellTerminalCommand = vi.fn(async () => ({
    kind: "command-rejected" as const,
    code: "invalid-action",
    message:
      "Shell Terminal requested path is outside the workspace and has no active read Additional Path Grant: /external/notes.md",
  }));
  render(<App client={{ ...client, loadShellTerminal, submitShellTerminalCommand }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await openCommandAudit();
  fireEvent.change(screen.getByRole("textbox", { name: "Command" }), {
    target: { value: "cat /external/notes.md" },
  });
  fireEvent.change(screen.getByRole("textbox", { name: "Requested paths" }), {
    target: { value: "/external/notes.md" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Run command" }));
  closeCommandAudit();

  const transcript = screen.getByRole("region", { name: "Prompt Transcript" });
  const grantPrompt = await within(transcript).findByRole("group", {
    name: "Additional Path Grant request for /external/notes.md",
  });
  fireEvent.click(within(grantPrompt).getByRole("button", { name: "Deny grant request for /external/notes.md" }));

  expect(await within(transcript).findByText(/Mission Commander denied Additional Path Grant for \/external\/notes.md/)).toBeVisible();
  expect(await within(transcript).findByText(/Orchestrator left command blocked/)).toBeVisible();
  expect(within(grantPrompt).getByText("Mission Commander denied this grant request.")).toBeVisible();
});

test("keeps Additional Path Grants as contextual history instead of a standing form", async () => {
  const grant = {
    grant_id: "path-grant-000001",
    correlation_id: "path-grant-ui-1",
    path: "/external/docs",
    access_level: "write" as const,
    duration_seconds: 900,
    granted_by: "mission-commander" as const,
    granted_at: "2026-07-01T12:00:00Z",
    expires_at: "2099-07-01T12:15:00Z",
  };
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: 2,
      commands: [],
      grants: [grant],
    },
  }));
  render(<App client={{ ...client, loadShellTerminal }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await waitFor(() => expect(loadShellTerminal).toHaveBeenCalledTimes(1));
  await openCommandAudit();
  await waitFor(() => expect(loadShellTerminal).toHaveBeenCalledTimes(2));

  expect(screen.queryByRole("textbox", { name: "Grant path" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Create Additional Path Grant" })).not.toBeInTheDocument();
  expect(screen.getByText(/Additional authority is requested inline/)).toBeVisible();
  expect(await screen.findByText("/external/docs")).toBeVisible();
  expect(screen.getByText("write / 900 seconds")).toBeVisible();
});

test("shows expired grants as immutable history without self-expansion controls", async () => {
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: 2,
      commands: [],
      grants: [{
        grant_id: "path-grant-expired",
        correlation_id: "path-grant-ui-expired",
        path: "/external/archive",
        access_level: "read" as const,
        duration_seconds: 60,
        granted_by: "mission-commander" as const,
        granted_at: "2020-01-01T00:00:00Z",
        expires_at: "2020-01-01T00:01:00Z",
      }],
    },
  }));
  render(<App client={{ ...client, loadShellTerminal }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await openCommandAudit();

  expect(await screen.findByText("Expired")).toBeVisible();
  expect(screen.queryByRole("button", { name: /Edit|Renew|Broaden/i })).not.toBeInTheDocument();
  expect(screen.queryByText(/agent requester|skill requester/i)).not.toBeInTheDocument();
});

test("keeps terminal inputs and shows actionable path rejection without false success", async () => {
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: { schema_version: 1 as const, revision: 1, commands: [], grants: [] },
  }));
  const submitShellTerminalCommand = vi.fn(async () => ({
    kind: "command-rejected" as const,
    code: "invalid-action",
    message:
      "Shell Terminal working directory is outside the workspace and has no active write Additional Path Grant.",
  }));
  render(<App client={{ ...client, loadShellTerminal, submitShellTerminalCommand }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await openCommandAudit();
  fireEvent.change(screen.getByRole("textbox", { name: "Command" }), {
    target: { value: "touch report.txt" },
  });
  fireEvent.change(screen.getByRole("textbox", { name: "Working directory" }), {
    target: { value: "/external/docs" },
  });
  fireEvent.change(screen.getByRole("textbox", { name: "Requested paths" }), {
    target: { value: "/external/docs/report.txt" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Run command" }));

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Shell Terminal working directory is outside the workspace and has no active write Additional Path Grant.",
  );
  expect(screen.getByRole("textbox", { name: "Command" })).toHaveValue("touch report.txt");
  expect(screen.getByRole("textbox", { name: "Working directory" })).toHaveValue("/external/docs");
  expect(screen.getByRole("textbox", { name: "Requested paths" })).toHaveValue(
    "/external/docs/report.txt",
  );
  expect(screen.queryByText(/Command completed/i)).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Command output")).not.toBeInTheDocument();
});

test("keeps command audit reachable at constrained width", async () => {
  const originalWidth = window.innerWidth;
  try {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 700 });
    fireEvent(window, new Event("resize"));
    render(<App client={client} />);
    await screen.findByRole("heading", { name: "Command Deck Mission" });
    fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
      target: { value: "Preserve constrained draft" },
    });
    expect(screen.queryByRole("tablist", { name: "Agent Workstation views" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open command audit" })).toBeVisible();
    expect(stylesSource).toMatch(
      /@media \(max-width: 820px\)[\s\S]*\.prompt-workspace \{ min-height: 58vh;[\s\S]*\.agent-workstations \{ min-height: 34vh;/,
    );
    await openCommandAudit();
    expect(screen.getByRole("region", { name: "Shell Terminal" })).toBeVisible();

    closeCommandAudit();
    expect(screen.getByRole("textbox", { name: "Message Alfredo" })).toHaveValue(
      "Preserve constrained draft",
    );
    expect(screen.queryByRole("region", { name: "Shell Terminal" })).not.toBeInTheDocument();
  } finally {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: originalWidth });
    fireEvent(window, new Event("resize"));
  }
});

test("launches from loading into the canonical Command Deck snapshot", async () => {
  render(<App client={client} />);

  expect(screen.getByRole("status")).toHaveTextContent("Connecting to Alfredo");
  expect(await screen.findByRole("heading", { name: "Command Deck Mission" })).toBeVisible();
  expect(screen.getByRole("region", { name: "Prompt Transcript" })).toBeVisible();
  expect(screen.getByRole("complementary", { name: "Mission Work" })).toBeVisible();
  expect(screen.getByText("Restore workspace session", { selector: ".scope-card strong" })).toBeVisible();
  expect(screen.getByText("Workspace Session workspace-command-deck")).toBeVisible();
  expect(screen.getAllByText("ISS-01")).not.toHaveLength(0);
});

test("exposes named landmarks and labelled controls with command audit open", async () => {
  render(<App client={client} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  expect(screen.getByRole("main", { name: "Prompt Workstation" })).toBeVisible();
  expect(screen.getByRole("complementary", { name: "Mission Work" })).toBeVisible();
  for (const control of document.querySelectorAll("button, input, select, textarea, a[href]")) {
    expect(control).toHaveAccessibleName();
  }

  await openCommandAudit();
  expect(screen.getByRole("region", { name: "Shell Terminal" })).toBeVisible();
  expect(screen.getByRole("alert")).toHaveTextContent("Shell Terminal transport is unavailable");
  for (const control of document.querySelectorAll("button, input, select, textarea, a[href]")) {
    expect(control).toHaveAccessibleName();
  }
});

test("exposes workstation cards as keyboard-reachable accessible summaries", async () => {
  const workstationSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 12,
    mission_board: {
      ...snapshot.mission_board,
      issue_slices: [
        {
          issue_id: "ISS-01",
          title: "Validate Alfredo accessibility",
          lifecycle: "Approved",
          progress: "Agent is streaming responsive fixes.",
          launch_eligible: false,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Validate workstation accessibility.",
            acceptance_criteria: ["Card summaries are understandable."],
            evidence_requirements: ["Accessibility contract test."],
            source_path: ".agent/issues/28-validate-alfredo-accessibility-and-responsive-use.md",
          },
          sessions: [
            {
              session_id: "session-ISS-01-1",
              assigned_agent: "qwen-coder-local",
              role: "local-agent",
              provider: "ollama",
              model: "qwen3.6:27b",
              status: "running",
              stale: false,
              disconnected: false,
              operation_status: "streaming",
              failure: "",
            },
          ],
          provenance: {
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
          model_assignment: {
            agent_id: "qwen-coder-local",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "running",
            failure: "",
          },
          evidence: {
            state: "missing",
            changed_files: ["mission-control/src/App.tsx"],
            commands_run: ["npm test -- App.test.tsx"],
            test_results: "Accessibility test is red.",
            risks: "Low-vision readability requires human validation.",
            artifact_links: ["app-local://evidence/session-ISS-01-1"],
          },
          working_context_sources: [],
        },
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-01-1",
            issue_id: "ISS-01",
            assigned_agent: "qwen-coder-local",
            status: "running",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
        ],
        attention: [],
      },
    ],
  };
  const originalWidth = window.innerWidth;
  try {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 390 });
    fireEvent(window, new Event("resize"));
    render(<App client={{ loadSnapshot: async () => ({ kind: "ready", snapshot: workstationSnapshot }) }} />);

    expect(await screen.findByRole("region", { name: "Prompt Composer" })).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Message Alfredo" })).toBeVisible();
    expect(screen.getByRole("status", { name: "Execution status" })).toHaveTextContent(
      "Execution Session running",
    );

    const cards = screen.getByRole("region", { name: "Workstation Cards" });
    const card = within(cards).getByRole("article", { name: "qwen-coder-local workstation card" });
    card.focus();
    expect(card).toHaveFocus();
    expect(card).toHaveAccessibleDescription(
      /running\. Validate Alfredo accessibility\. Next action: Agent is streaming responsive fixes\./,
    );
    expect(within(card).getByText("running")).toHaveAccessibleDescription(
      "Running work is active. Monitor progress and preserve the prompt workflow.",
    );

    const expand = within(card).getByRole("button", { name: "Expand qwen-coder-local" });
    expect(expand).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(expand);
    expect(expand).toHaveAttribute("aria-expanded", "true");
    expect(
      within(card).getByRole("button", { name: "Open diff mission-control/src/App.tsx" }),
    ).toBeVisible();

    const cancel = within(card).getByRole("button", { name: "Cancel session session-ISS-01-1" });
    expect(cancel).toBeDisabled();
    expect(cancel).toHaveAccessibleDescription(
      "Enter a reason to enable Cancel session for session-ISS-01-1.",
    );
  } finally {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: originalWidth });
    fireEvent(window, new Event("resize"));
  }
});

test("keeps critical workstation decisions reachable at 520px", async () => {
  const queueItem = {
    item_id: "delegation-command-deck-ISS-02",
    mission_id: "command-deck",
    item_type: "ad-hoc-delegation" as const,
    status: "pending" as const,
    source: "agent-console",
    requested_action: "Approve ISS-02 delegation",
    affected_boundary: "launch-boundary",
    consequence: "Approval launches a local agent session.",
    issue_id: "ADHOC-000001",
    proposed_changes: {},
  };
  const constrainedSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 12,
    operations_view: "review-workspace",
    mission_board: {
      ...snapshot.mission_board,
      issue_slices: [
        {
          issue_id: "ISS-01",
          title: "Review responsive workstation evidence",
          lifecycle: "Ready",
          progress: "Evidence package ready for review.",
          launch_eligible: false,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Review workstation accessibility evidence.",
            acceptance_criteria: ["Review actions remain reachable."],
            evidence_requirements: ["Review Workspace decision controls."],
            source_path: ".agent/issues/28-validate-alfredo-accessibility-and-responsive-use.md",
          },
          sessions: [
            {
              session_id: "session-ISS-01-1",
              assigned_agent: "review-agent",
              role: "local-agent",
              provider: "ollama",
              model: "qwen3.6:27b",
              status: "evidence-ready",
              stale: false,
              disconnected: false,
              operation_status: "completed",
              failure: "",
            },
          ],
          provenance: {
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
          model_assignment: {
            agent_id: "review-agent",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "completed",
            failure: "",
          },
          evidence: {
            state: "ready",
            changed_files: ["mission-control/src/App.tsx"],
            commands_run: ["npm test -- App.test.tsx"],
            test_results: "Accessibility contract passed.",
            risks: "Human reviewer still needs to inspect zoom and low-vision readability.",
            artifact_links: ["app-local://evidence/session-ISS-01-1"],
          },
          working_context_sources: [],
        },
        {
          issue_id: "ISS-03",
          title: "Recover stale workstation state",
          lifecycle: "Approved",
          progress: "Retry requires a fresh reason.",
          launch_eligible: false,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Recover stale state.",
            acceptance_criteria: ["Stale failures are understandable."],
            evidence_requirements: [],
            source_path: ".agent/issues/28-validate-alfredo-accessibility-and-responsive-use.md",
          },
          sessions: [
            {
              session_id: "session-ISS-03-1",
              assigned_agent: "repair-agent",
              role: "local-agent",
              provider: "ollama",
              model: "qwen3.6:27b",
              status: "failed",
              stale: true,
              disconnected: false,
              operation_status: "failed",
              failure: "Stale state: workspace revision changed; refresh before retry.",
            },
          ],
          provenance: {
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
          model_assignment: {
            agent_id: "repair-agent",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "failed",
            failure: "Stale state: workspace revision changed; refresh before retry.",
          },
          evidence: {
            state: "missing",
            changed_files: [],
            commands_run: [],
            test_results: "",
            risks: "Stale snapshot needs recovery.",
            artifact_links: [],
          },
          working_context_sources: [],
        },
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-01-1",
            issue_id: "ISS-01",
            assigned_agent: "review-agent",
            status: "evidence-ready",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
          {
            session_id: "session-ISS-03-1",
            issue_id: "ISS-03",
            assigned_agent: "repair-agent",
            status: "failed",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
        ],
        attention: [
          {
            attention_id: "delegation-command-deck-ISS-02",
            mission_id: "command-deck",
            kind: "delegation-approval",
            label: "ISS-02 delegation approval required",
            queue_link: "workspace-queue#delegation-command-deck-ISS-02",
          },
        ],
      },
    ],
  };
  const reviewProjection: ReviewWorkspaceProjection = {
    schema_version: 1,
    revision: 12,
    mission_id: "command-deck",
    items: [
      {
        mission_id: "command-deck",
        issue_id: "ISS-01",
        issue_title: "Review responsive workstation evidence",
        session_id: "session-ISS-01-1",
        assigned_agent: "review-agent",
        status: "evidence-ready",
        lifecycle: "Ready",
        evidence_complete: true,
        missing_evidence: [],
        can_accept: true,
        evidence: {
          changed_files: ["mission-control/src/App.tsx"],
          diff_summary: "Added workstation accessibility hardening.",
          commands_run: ["npm test -- App.test.tsx"],
          test_results: "Accessibility contract passed.",
          risks: "Human reviewer still needs to inspect zoom and low-vision readability.",
          proposed_context_updates: "",
          artifact_links: ["app-local://evidence/session-ISS-01-1"],
        },
        visibility_limitations: [],
      },
    ],
  };
  const originalWidth = window.innerWidth;
  try {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 520 });
    fireEvent(window, new Event("resize"));
    render(
      <App
        client={{
          loadSnapshot: async () => ({ kind: "ready", snapshot: constrainedSnapshot }),
          loadWorkspaceQueue: async () => ({
            kind: "workspace-queue",
            projection: {
              schema_version: 1,
              revision: 12,
              items: [queueItem],
              groups: [
                {
                  group_id: "ad-hoc-delegation:command-deck",
                  item_type: "ad-hoc-delegation",
                  mission_id: "command-deck",
                  item_count: 1,
                  items: [queueItem],
                },
              ],
            },
          }),
          loadReviewWorkspace: async () => ({ kind: "review-workspace", projection: reviewProjection }),
        }}
      />,
    );

    const prompt = await screen.findByRole("main", { name: "Prompt Workstation" });
    const cards = await screen.findByRole("region", { name: "Workstation Cards" });
    expect(prompt.compareDocumentPosition(cards) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
    expect(screen.getByRole("region", { name: "Prompt Composer" })).toBeVisible();
    expect(within(cards).getByRole("button", { name: "Approve delegation-command-deck-ISS-02" })).toBeEnabled();

    const failedCard = within(cards).getByRole("article", { name: "repair-agent workstation card" });
    expect(failedCard).toHaveAccessibleDescription(/Stale state: workspace revision changed; refresh before retry\./);
    expect(within(failedCard).getByText("failed", { selector: ".status" })).toHaveAccessibleDescription(
      "Failed work needs review, repair, retry, or human escalation.",
    );

    const reviewWorkspace = await screen.findByRole("region", { name: "Review Workspace" });
    expect(within(reviewWorkspace).getByRole("button", { name: "Accept session-ISS-01-1" })).toBeEnabled();
    expect(within(reviewWorkspace).getByRole("button", { name: "Request repair session-ISS-01-1" })).toBeDisabled();
    expect(within(reviewWorkspace).getByRole("button", { name: "Escalate session-ISS-01-1" })).toBeEnabled();
  } finally {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: originalWidth });
    fireEvent(window, new Event("resize"));
  }
});

test("audits workstation contrast tokens and reduced-motion CSS", () => {
  const pairs = [
    ["ink/base", cssVariable("ink"), cssRootColor("background")],
    ["muted/card", cssVariable("muted"), cssRuleColor(".workstation-card", "background")],
    ["body/card", cssRuleColor(".workstation-card p", "color"), cssRuleColor(".workstation-card", "background")],
    ["cyan/control", cssVariable("cyan"), "#0a0f0d"],
    ["lime/control", cssVariable("lime"), "#0a0f0d"],
    ["warning/pending", cssRuleColor(".workstation-card__action-help", "color"), cssRuleColor(".workstation-pending", "background")],
    ["danger/action", "#ffb4ad", cssRuleColor(".action--danger", "background")],
    ["focus/base", cssVariable("focus"), "#050807"],
  ] as const;

  for (const [label, foreground, background] of pairs) {
    expect(contrastRatio(foreground, background), label).toBeGreaterThanOrEqual(4.5);
  }
  expect(stylesSource).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
  expect(stylesSource).toMatch(/animation:\s*none\s*!important/);
  expect(stylesSource).toMatch(/transition:\s*none\s*!important/);
  expect(stylesSource).toMatch(/@media \(max-width: 520px\)/);
});

test("restores the acknowledged Operations Workspace view", async () => {
  const restoredClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "ready",
      snapshot: { ...snapshot, operations_view: "review-workspace" },
    }),
  };

  render(<App client={restoredClient} />);

  expect(await screen.findByRole("heading", { name: "Review Workspace" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Review" })).toHaveAttribute("aria-current", "page");
});

test("review workspace lists evidence packages and blocks incomplete acceptance", async () => {
  const projection: ReviewWorkspaceProjection = {
    schema_version: 1,
    revision: 4,
    mission_id: "command-deck",
    items: [
      {
        mission_id: "command-deck",
        issue_id: "ISS-01",
        issue_title: "Restore workspace session",
        session_id: "session-ISS-01-1",
        assigned_agent: "qwen-coder-local",
        status: "evidence-ready",
        lifecycle: "Ready",
        evidence_complete: true,
        missing_evidence: [],
        can_accept: true,
        evidence: {
          changed_files: ["src/App.tsx", ".env"],
          diff_summary: "Added Review Workspace.",
          commands_run: ["npm test"],
          test_results: "Tests passed.",
          risks: "None.",
          proposed_context_updates: "Document Review Workspace.",
          artifact_links: ["app-local://evidence/session-ISS-01-1"],
        },
        visibility_limitations: [
          {
            path: ".env",
            classification: "Blocked",
            consequence: "Frontier Reviewer cannot inspect this path; human review may be required.",
          },
        ],
      },
      {
        mission_id: "command-deck",
        issue_id: "ISS-02",
        issue_title: "Synchronize live state",
        session_id: "session-ISS-02-1",
        assigned_agent: "qwen-coder-local",
        status: "launched",
        lifecycle: "Approved",
        evidence_complete: false,
        missing_evidence: ["changed_files", "diff_summary", "commands_run"],
        can_accept: false,
        evidence: {
          changed_files: [],
          diff_summary: "",
          commands_run: [],
          test_results: "",
          risks: "",
          proposed_context_updates: "",
          artifact_links: [],
        },
        visibility_limitations: [],
      },
    ],
  };
  const reviewClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "ready",
      snapshot: { ...snapshot, operations_view: "review-workspace" },
    }),
    loadReviewWorkspace: async () => ({ kind: "review-workspace", projection }),
  };

  render(<App client={reviewClient} />);

  const workspace = await screen.findByRole("region", { name: "Review Workspace" });
  expect(within(workspace).getByText("session-ISS-01-1")).toBeVisible();
  expect(within(workspace).getByText("Added Review Workspace.")).toBeVisible();
  expect(within(workspace).getByText("npm test")).toBeVisible();
  expect(within(workspace).getByText("Tests passed.")).toBeVisible();
  expect(within(workspace).getByText("Document Review Workspace.")).toBeVisible();
  expect(within(workspace).getByText(".env")).toBeVisible();
  expect(within(workspace).getByText("Blocked")).toBeVisible();
  expect(within(workspace).getByText("changed_files, diff_summary, commands_run")).toBeVisible();
  expect(within(workspace).getByRole("button", { name: "Accept session-ISS-01-1" })).toBeEnabled();
  expect(within(workspace).getByRole("button", { name: "Accept session-ISS-02-1" })).toBeDisabled();
});

test("activity view displays searchable journal entries with filters and links", async () => {
  const initialJournal: ActivityJournalProjection = {
    schema_version: 1,
    revision: 2,
    entries: [
      {
        entry_id: "activity-000001",
        sequence: 1,
        recorded_at: "2026-06-26T10:00:00Z",
        actor: "mission-commander",
        action_type: "operations-view-selected",
        summary: "Mission Commander selected Operations Workspace view Activity.",
        affected_entities: [
          {
            entity_type: "mission",
            entity_id: "command-deck",
            label: "Command Deck Mission",
            href: "app-local://missions/command-deck",
          },
        ],
        evidence_links: [],
        correlation_id: "view-activity-1",
      },
      {
        entry_id: "activity-000002",
        sequence: 2,
        recorded_at: "2026-06-26T10:15:00Z",
        actor: "mission-commander",
        action_type: "review-decision",
        summary: "Mission Commander recorded Review Workspace decision Approved.",
        affected_entities: [
          {
            entity_type: "issue-slice",
            entity_id: "ISS-01",
            label: "Restore workspace session",
            href: "app-local://missions/command-deck/issues/ISS-01",
          },
          {
            entity_type: "evidence-package",
            entity_id: "session-ISS-01-1",
            label: "Evidence Package session-ISS-01-1",
            href: "app-local://evidence/session-ISS-01-1",
          },
        ],
        evidence_links: ["app-local://evidence/session-ISS-01-1"],
        correlation_id: "review-activity-1",
      },
    ],
  };
  const filteredJournal: ActivityJournalProjection = {
    ...initialJournal,
    entries: [initialJournal.entries[1]],
  };
  const filterRequests: ActivityJournalFilters[] = [];
  const activityClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "ready",
      snapshot: { ...snapshot, operations_view: "activity" },
    }),
    loadActivityJournal: async (filters = {}) => {
      filterRequests.push(filters);
      return {
        kind: "activity-journal",
        projection: filterRequests.length === 1 ? initialJournal : filteredJournal,
      };
    },
  };

  render(<App client={activityClient} />);

  const activity = await screen.findByRole("region", { name: "Activity Journal" });
  expect(await within(activity).findByText("Mission Commander selected Operations Workspace view Activity.")).toBeVisible();
  expect(within(activity).getByText("Mission Commander recorded Review Workspace decision Approved.")).toBeVisible();
  expect(within(activity).getByText("issue-slice / ISS-01")).toBeVisible();
  expect(within(activity).getByRole("link", { name: "Evidence Package session-ISS-01-1" })).toHaveAttribute(
    "href",
    "app-local://evidence/session-ISS-01-1",
  );

  fireEvent.change(within(activity).getByRole("searchbox", { name: "Search Activity" }), {
    target: { value: "evidence" },
  });
  fireEvent.change(within(activity).getByLabelText("Activity action type"), {
    target: { value: "review-decision" },
  });
  fireEvent.change(within(activity).getByLabelText("Activity actor"), {
    target: { value: "mission-commander" },
  });
  fireEvent.change(within(activity).getByLabelText("Activity Mission"), {
    target: { value: "command-deck" },
  });
  fireEvent.click(within(activity).getByRole("button", { name: "Apply Activity filters" }));

  await waitFor(() => expect(filterRequests).toHaveLength(2));
  expect(filterRequests[1]).toMatchObject({
    search: "evidence",
    action_type: "review-decision",
    actor: "mission-commander",
    mission_id: "command-deck",
  });
  expect(within(activity).queryByText("Mission Commander selected Operations Workspace view Activity.")).not.toBeInTheDocument();
  expect(within(activity).getByText("Mission Commander recorded Review Workspace decision Approved.")).toBeVisible();
});

test("workspace queue lists grouped governance items and acknowledges decisions", async () => {
  const item = {
    item_id: "issue-change-command-deck-ISS-01-000001",
    mission_id: "command-deck",
    item_type: "issue-change-proposal" as const,
    status: "pending" as const,
    source: "issue-slice-inspector",
    requested_action: "Change accepted Issue Slice contract",
    affected_boundary: "acceptance_criteria",
    consequence: "Approval will reopen ISS-01 for re-review.",
    issue_id: "ISS-01",
    proposed_changes: {
      acceptance_criteria: ["Queue proposals preserve accepted state."],
    },
  };
  const projection: WorkspaceQueueProjection = {
    schema_version: 1,
    revision: 2,
    items: [item],
    groups: [
      {
        group_id: "issue-change-proposal:command-deck",
        item_type: "issue-change-proposal",
        mission_id: "command-deck",
        item_count: 1,
        items: [item],
      },
    ],
  };
  let queueLoads = 0;
  const decisions: unknown[] = [];
  const queueClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "ready",
      snapshot: { ...snapshot, operations_view: "workspace-queue" },
    }),
    loadWorkspaceQueue: async () => {
      queueLoads += 1;
      return {
        kind: "workspace-queue",
        projection: queueLoads === 1 ? projection : { ...projection, revision: 3, items: [], groups: [] },
      };
    },
    submitWorkspaceQueueDecision: async (request) => {
      decisions.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 3,
          item_id: request.item_id,
          item_status: "approved",
          effect_summary: "Applied proposal; ISS-01 is reopened for re-review.",
        },
      };
    },
  };

  render(<App client={queueClient} />);

  const queue = await screen.findByRole("region", { name: "Workspace Queue" });
  expect(within(queue).getByText("issue-change-proposal / command-deck")).toBeVisible();
  expect(within(queue).getByText("issue-slice-inspector")).toBeVisible();
  expect(within(queue).getByText("Change accepted Issue Slice contract")).toBeVisible();
  expect(within(queue).getByText("acceptance_criteria")).toBeVisible();
  expect(within(queue).getByText("Approval will reopen ISS-01 for re-review.")).toBeVisible();
  expect(within(queue).getByText("Queue proposals preserve accepted state.")).toBeVisible();

  fireEvent.click(within(queue).getByRole("button", { name: "Approve issue-change-command-deck-ISS-01-000001" }));

  expect(await screen.findByRole("status", { name: "Workspace Queue decision status" })).toHaveTextContent("Acknowledged");
  expect(screen.getByRole("status", { name: "Workspace Queue decision status" })).toHaveFocus();
  expect(decisions).toEqual([
    {
      correlation_id: "queue-approve-issue-change-command-deck-ISS-01-000001-2",
      action_type: "workspace-queue-decision",
      actor: "mission-commander",
      expected_revision: 2,
      target: {
        kind: "workspace-queue-item",
        id: "issue-change-command-deck-ISS-01-000001",
      },
      item_id: "issue-change-command-deck-ISS-01-000001",
      decision: "approve",
      reason: "",
    },
  ]);
  expect(await within(queue).findByText("No governance items pending")).toBeVisible();
});

test("workspace queue presents Mission Draft scope before confirmation", async () => {
  const draftProjection: MissionDraftProjection = {
    schema_version: 1,
    revision: 2,
    drafts: [
      {
        draft_id: "mission-draft-command-deck-000001",
        mission_id: "command-deck",
        status: "draft",
        proposed_goal: "Create a focused Command Deck follow-up mission.",
        included_ad_hoc_work: [
          {
            work_id: "ADHOC-000001",
            source: "agent-console",
            status: "pending",
            acceptance_criteria: ["Represent selected ad hoc work."],
            allowed_paths: ["docs/mission-draft.md"],
            originating_message_id: "console-000001",
          },
        ],
        excluded_ad_hoc_work_ids: ["ADHOC-000002"],
        new_work_items: ["Add confirmation handling."],
        dependencies: ["Issue 10 remains authoritative."],
        unresolved_decisions: ["Choose final UI placement."],
      },
    ],
  };
  const queueClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "ready",
      snapshot: { ...snapshot, operations_view: "workspace-queue" },
    }),
    loadWorkspaceQueue: async () => ({
      kind: "workspace-queue",
      projection: {
        schema_version: 1,
        revision: 1,
        items: [],
        groups: [],
      },
    }),
    loadMissionDrafts: async () => ({
      kind: "mission-drafts",
      projection: draftProjection,
    }),
  };

  render(<App client={queueClient} />);

  const queue = await screen.findByRole("region", { name: "Workspace Queue" });
  const drafts = await within(queue).findByRole("region", { name: "Mission Drafts" });

  expect(within(drafts).getByText("Create a focused Command Deck follow-up mission.")).toBeVisible();
  expect(within(drafts).getByText("ADHOC-000001")).toBeVisible();
  expect(within(drafts).getByText("Represent selected ad hoc work.")).toBeVisible();
  expect(within(drafts).getByText("Excluded: ADHOC-000002")).toBeVisible();
  expect(within(drafts).getByText("Add confirmation handling.")).toBeVisible();
  expect(within(drafts).getByText("Issue 10 remains authoritative.")).toBeVisible();
  expect(within(drafts).getByText("Choose final UI placement.")).toBeVisible();
});

test("workspace queue creates a Mission Draft from selected Ad Hoc Delegations", async () => {
  const draftCreates: unknown[] = [];
  let draftLoads = 0;
  const selected = {
    item_id: "ad-hoc-delegation-command-deck-000001",
    mission_id: "command-deck",
    item_type: "ad-hoc-delegation" as const,
    status: "pending" as const,
    source: "agent-console",
    requested_action: "Promote useful ad hoc work.",
    affected_boundary: "mission-draft",
    consequence: "Can be included in a draft without accepting mission state.",
    issue_id: "ADHOC-000001",
    proposed_changes: {
      acceptance_criteria: ["Represent selected ad hoc work."],
      allowed_paths: ["docs/selected.md"],
    },
  };
  const excluded = {
    ...selected,
    item_id: "ad-hoc-delegation-command-deck-000002",
    requested_action: "Keep unrelated ad hoc work separate.",
    issue_id: "ADHOC-000002",
    proposed_changes: {
      acceptance_criteria: ["Keep unrelated work outside the draft."],
      allowed_paths: ["docs/excluded.md"],
    },
  };
  const projection: WorkspaceQueueProjection = {
    schema_version: 1,
    revision: 6,
    items: [selected, excluded],
    groups: [
      {
        group_id: "ad-hoc-delegation:command-deck",
        item_type: "ad-hoc-delegation",
        mission_id: "command-deck",
        item_count: 2,
        items: [selected, excluded],
      },
    ],
  };
  const emptyDrafts: MissionDraftProjection = {
    schema_version: 1,
    revision: 1,
    drafts: [],
  };
  const createdDrafts: MissionDraftProjection = {
    schema_version: 1,
    revision: 2,
    drafts: [
      {
        draft_id: "mission-draft-command-deck-000001",
        mission_id: "command-deck",
        status: "draft",
        proposed_goal: "Create a focused follow-up mission.",
        included_ad_hoc_work: [
          {
            work_id: "ADHOC-000001",
            source: "agent-console",
            status: "pending",
            acceptance_criteria: ["Represent selected ad hoc work."],
            allowed_paths: ["docs/selected.md"],
            originating_message_id: "console-000001",
          },
        ],
        excluded_ad_hoc_work_ids: ["ADHOC-000002"],
        new_work_items: ["Add explicit confirmation handling."],
        dependencies: ["Issue 10 approvals remain authoritative."],
        unresolved_decisions: ["Choose final queue grouping."],
      },
    ],
  };
  const queueClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "ready",
      snapshot: { ...snapshot, revision: 4, operations_view: "workspace-queue" },
    }),
    loadWorkspaceQueue: async () => ({
      kind: "workspace-queue",
      projection,
    }),
    loadMissionDrafts: async () => {
      draftLoads += 1;
      return {
        kind: "mission-drafts",
        projection: draftLoads === 1 ? emptyDrafts : createdDrafts,
      };
    },
    submitMissionDraftCreate: async (request) => {
      draftCreates.push(request);
      expect(draftLoads).toBe(1);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 2,
          draft_id: "mission-draft-command-deck-000001",
          draft_status: "draft",
          effect_summary: "Mission Draft created; accepted Mission state is unchanged.",
          accepted_issue_id: "",
        },
      };
    },
  };

  render(<App client={queueClient} />);

  const queue = await screen.findByRole("region", { name: "Workspace Queue" });
  fireEvent.click(await within(queue).findByLabelText("Include ADHOC-000001"));
  fireEvent.click(within(queue).getByLabelText("Exclude ADHOC-000002"));
  fireEvent.change(within(queue).getByLabelText("Mission Draft proposed goal"), {
    target: { value: "Create a focused follow-up mission." },
  });
  fireEvent.change(within(queue).getByLabelText("Mission Draft new work"), {
    target: { value: "Add explicit confirmation handling." },
  });
  fireEvent.change(within(queue).getByLabelText("Mission Draft dependencies"), {
    target: { value: "Issue 10 approvals remain authoritative." },
  });
  fireEvent.change(within(queue).getByLabelText("Mission Draft unresolved decisions"), {
    target: { value: "Choose final queue grouping." },
  });
  fireEvent.click(within(queue).getByRole("button", { name: "Create Mission Draft" }));

  await waitFor(() => expect(draftCreates).toHaveLength(1));
  expect(draftCreates[0]).toEqual({
    correlation_id: "mission-draft-create-4",
    expected_revision: 4,
    proposed_goal: "Create a focused follow-up mission.",
    selected_ad_hoc_ids: ["ADHOC-000001"],
    excluded_ad_hoc_ids: ["ADHOC-000002"],
    new_work_items: ["Add explicit confirmation handling."],
    dependencies: ["Issue 10 approvals remain authoritative."],
    unresolved_decisions: ["Choose final queue grouping."],
    mission_id: "command-deck",
  });
  expect(await screen.findByRole("status", { name: "Mission Draft decision status" })).toHaveTextContent("Acknowledged");
  expect(await within(queue).findByText("mission-draft-command-deck-000001")).toBeVisible();
});

test("workspace queue confirms a Mission Draft only through acknowledgement", async () => {
  const requests: unknown[] = [];
  let draftLoads = 0;
  const draftProjection: MissionDraftProjection = {
    schema_version: 1,
    revision: 2,
    drafts: [
      {
        draft_id: "mission-draft-command-deck-000001",
        mission_id: "command-deck",
        status: "draft",
        proposed_goal: "Create a focused Command Deck follow-up mission.",
        included_ad_hoc_work: [],
        excluded_ad_hoc_work_ids: [],
        new_work_items: ["Add confirmation handling."],
        dependencies: [],
        unresolved_decisions: [],
      },
    ],
  };
  const queueClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "ready",
      snapshot: { ...snapshot, operations_view: "workspace-queue" },
    }),
    loadWorkspaceQueue: async () => ({
      kind: "workspace-queue",
      projection: {
        schema_version: 1,
        revision: 1,
        items: [],
        groups: [],
      },
    }),
    loadMissionDrafts: async () => {
      draftLoads += 1;
      return {
        kind: "mission-drafts",
        projection:
          draftLoads === 1
            ? draftProjection
            : {
                ...draftProjection,
                revision: 3,
                drafts: [{ ...draftProjection.drafts[0], status: "confirmed" }],
              },
      };
    },
    submitMissionDraftDecision: async (request) => {
      requests.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 3,
          draft_id: request.draft_id,
          draft_status: "confirmed",
          effect_summary: "Mission Draft confirmed as accepted Issue Slice ISS-02.",
          accepted_issue_id: "ISS-02",
        },
      };
    },
  };

  render(<App client={queueClient} />);

  const queue = await screen.findByRole("region", { name: "Workspace Queue" });
  const drafts = await within(queue).findByRole("region", { name: "Mission Drafts" });
  fireEvent.change(within(drafts).getByLabelText("Mission Draft decision reason"), {
    target: { value: "Mission Commander confirmed this scope." },
  });
  fireEvent.click(within(drafts).getByRole("button", { name: "Confirm mission-draft-command-deck-000001" }));

  expect(await screen.findByRole("status", { name: "Mission Draft decision status" })).toHaveTextContent("Acknowledged");
  expect(await screen.findByText(/Workstation action: Mission Commander requested Confirm Mission Draft mission-draft-command-deck-000001/)).toBeVisible();
  expect(await screen.findByText(/Orchestrator accepted workstation action: Mission Draft confirmed as accepted Issue Slice ISS-02/)).toBeVisible();
  expect(requests).toEqual([
    {
      correlation_id: "mission-draft-confirm-mission-draft-command-deck-000001-2",
      expected_revision: 2,
      draft_id: "mission-draft-command-deck-000001",
      decision: "confirm",
      reason: "Mission Commander confirmed this scope.",
    },
  ]);
  expect(await within(drafts).findByText("confirmed / command-deck")).toBeVisible();
});

test("workspace queue proposes ad hoc delegation from the latest console message", async () => {
  const proposals: unknown[] = [];
  let queueLoads = 0;
  const queueClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "ready",
      snapshot: {
        ...snapshot,
        operations_view: "workspace-queue",
        conversation_scope: {
          kind: "working-directory",
          target_id: "/workspace/albert",
          label: "albert",
        },
      },
    }),
    loadConsoleHistory: async () => ({
      kind: "history",
      history: {
        schema_version: 1,
        messages: [
          {
            message_id: "console-000001",
            sequence: 1,
            role: "user",
            content: "Have a local agent refresh smoke-test notes.",
            scope: {
              kind: "working-directory",
              target_id: "/workspace/albert",
              label: "albert",
            },
            outcome: "proposed",
            source: "mission-commander",
          },
        ],
      },
    }),
    loadWorkspaceQueue: async () => {
      queueLoads += 1;
      return {
        kind: "workspace-queue",
        projection: {
          schema_version: 1,
          revision: 1 + queueLoads,
          items: [],
          groups: [],
        },
      };
    },
    submitAdHocDelegationProposal: async (request) => {
      proposals.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 3,
          item_id: "ad-hoc-delegation-command-deck-000001",
          item_status: "pending",
          effect_summary: "Ad Hoc Delegation ADHOC-000001 is pending approval.",
        },
      };
    },
  };

  render(<App client={queueClient} />);

  const queue = await screen.findByRole("region", { name: "Workspace Queue" });
  fireEvent.change(within(queue).getByLabelText("Ad Hoc Delegation acceptance criteria"), {
    target: { value: "Smoke-test notes mention the focused unit command." },
  });
  fireEvent.change(within(queue).getByLabelText("Ad Hoc Delegation allowed paths"), {
    target: { value: "docs/smoke-tests.md" },
  });
  fireEvent.change(within(queue).getByLabelText("Ad Hoc Delegation command policy"), {
    target: { value: "python3 -m unittest tests.test_workspace_snapshot=auto-allowed" },
  });
  fireEvent.change(within(queue).getByLabelText("Ad Hoc Delegation proposed agent"), {
    target: { value: "qwen-coder-local-1" },
  });
  fireEvent.click(within(queue).getByRole("button", { name: "Propose Ad Hoc Delegation" }));

  await waitFor(() => expect(proposals).toHaveLength(1));
  expect(proposals[0]).toEqual({
    correlation_id: "ad-hoc-delegation-console-000001-4",
    expected_revision: 4,
    source: "agent-console",
    scope_kind: "working-directory",
    scope_target: "/workspace/albert",
    scope_label: "albert",
    acceptance_criteria: ["Smoke-test notes mention the focused unit command."],
    allowed_paths: ["docs/smoke-tests.md"],
    command_policy: {
      "python3 -m unittest tests.test_workspace_snapshot": "auto-allowed",
    },
    proposed_agent: "qwen-coder-local-1",
    originating_message_id: "console-000001",
  });
  expect(await screen.findByRole("status", { name: "Workspace Queue decision status" })).toHaveTextContent("Acknowledged");
});

test("review workspace applies accepted evidence only after acknowledgement", async () => {
  const projection: ReviewWorkspaceProjection = {
    schema_version: 1,
    revision: 4,
    mission_id: "command-deck",
    items: [
      {
        mission_id: "command-deck",
        issue_id: "ISS-01",
        issue_title: "Restore workspace session",
        session_id: "session-ISS-01-1",
        assigned_agent: "qwen-coder-local",
        status: "evidence-ready",
        lifecycle: "Ready",
        evidence_complete: true,
        missing_evidence: [],
        can_accept: true,
        evidence: {
          changed_files: ["src/App.tsx"],
          diff_summary: "Added Review Workspace.",
          commands_run: ["npm test"],
          test_results: "Tests passed.",
          risks: "None.",
          proposed_context_updates: "Document Review Workspace.",
          artifact_links: [],
        },
        visibility_limitations: [],
      },
    ],
  };
  let resolveDecision!: (value: Awaited<ReturnType<NonNullable<WorkspaceClient["submitReviewDecision"]>>>) => void;
  const decisionPromise = new Promise<Awaited<ReturnType<NonNullable<WorkspaceClient["submitReviewDecision"]>>>>(
    (resolve) => {
      resolveDecision = resolve;
    },
  );
  const requests: unknown[] = [];
  let snapshotLoads = 0;
  let reviewLoads = 0;
  const reviewClient: WorkspaceClient = {
    loadSnapshot: async () => {
      snapshotLoads += 1;
      return {
        kind: "ready",
        snapshot: {
          ...snapshot,
          revision: snapshotLoads === 1 ? 4 : 5,
          operations_view: "review-workspace",
        },
      };
    },
    loadReviewWorkspace: async () => {
      reviewLoads += 1;
      return {
        kind: "review-workspace",
        projection:
          reviewLoads === 1
            ? projection
            : { ...projection, revision: 5, items: [] },
      };
    },
    submitReviewDecision: async (request) => {
      requests.push(request);
      return decisionPromise;
    },
  };

  render(<App client={reviewClient} />);
  const workspace = await screen.findByRole("region", { name: "Review Workspace" });

  fireEvent.click(within(workspace).getByRole("button", { name: "Accept session-ISS-01-1" }));

  expect(await screen.findByRole("status", { name: "Review decision status" })).toHaveTextContent("Pending");
  expect(screen.getByText(/Workstation action: Mission Commander requested Accept evidence for session-ISS-01-1/)).toBeVisible();
  expect(screen.getByText("Orchestrator validating workstation action.")).toBeVisible();
  expect(screen.getByText("session-ISS-01-1")).toBeVisible();
  expect(requests).toEqual([
    {
      correlation_id: "review-accept-session-ISS-01-1-4",
      expected_revision: 4,
      session_id: "session-ISS-01-1",
      decision: "accept",
      reason: "",
    },
  ]);

  await act(async () => {
    resolveDecision({
      kind: "acknowledged",
      acknowledgement: {
        correlation_id: "review-accept-session-ISS-01-1-4",
        outcome: "acknowledged",
        revision: 5,
        issue_id: "ISS-01",
        session_id: "session-ISS-01-1",
        review_outcome: "Approved",
        next_action: "prepare-pr",
        issue_lifecycle: "Complete",
        effect_summary: "Issue Slice becomes Complete and PR-ready; it is not marked merged.",
      },
    });
  });

  expect(await screen.findByText("No evidence awaiting review")).toBeVisible();
  expect(await screen.findByText(/Orchestrator accepted workstation action: Issue Slice becomes Complete and PR-ready/)).toBeVisible();
  expect(screen.getByRole("status", { name: "Review decision status" })).toHaveTextContent(
    "Issue Slice becomes Complete and PR-ready; it is not marked merged.",
  );
  expect(screen.getByRole("status", { name: "Review decision status" })).toHaveFocus();
});

test("review workspace requires a repair reason and exposes the next action", async () => {
  const projection: ReviewWorkspaceProjection = {
    schema_version: 1,
    revision: 4,
    mission_id: "command-deck",
    items: [
      {
        mission_id: "command-deck",
        issue_id: "ISS-01",
        issue_title: "Restore workspace session",
        session_id: "session-ISS-01-1",
        assigned_agent: "qwen-coder-local",
        status: "evidence-ready",
        lifecycle: "Ready",
        evidence_complete: true,
        missing_evidence: [],
        can_accept: true,
        evidence: {
          changed_files: ["src/App.tsx"],
          diff_summary: "Added Review Workspace.",
          commands_run: ["npm test"],
          test_results: "Tests passed.",
          risks: "Needs clearer copy.",
          proposed_context_updates: "Document repair flow.",
          artifact_links: [],
        },
        visibility_limitations: [],
      },
    ],
  };
  const requests: unknown[] = [];
  const reviewClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "ready",
      snapshot: { ...snapshot, operations_view: "review-workspace" },
    }),
    loadReviewWorkspace: async () => ({ kind: "review-workspace", projection }),
    submitReviewDecision: async (request) => {
      requests.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 5,
          issue_id: "ISS-01",
          session_id: request.session_id,
          review_outcome: "Needs repair",
          next_action: "same-local-agent-repair",
          issue_lifecycle: "Needs repair",
          effect_summary: "Issue Slice needs repair; next action is same-local-agent-repair.",
        },
      };
    },
  };

  render(<App client={reviewClient} />);
  const workspace = await screen.findByRole("region", { name: "Review Workspace" });
  const repair = within(workspace).getByRole("button", { name: "Request repair session-ISS-01-1" });

  expect(repair).toBeDisabled();
  fireEvent.change(within(workspace).getByLabelText("Review reason session-ISS-01-1"), {
    target: { value: "Acceptance copy is missing." },
  });
  expect(repair).toBeEnabled();
  fireEvent.click(repair);

  expect(await screen.findByRole("status", { name: "Review decision status" })).toHaveTextContent(
    "same-local-agent-repair",
  );
  expect(requests).toEqual([
    {
      correlation_id: "review-repair-session-ISS-01-1-4",
      expected_revision: 4,
      session_id: "session-ISS-01-1",
      decision: "repair",
      reason: "Acceptance copy is missing.",
    },
  ]);
});

test("review workspace records explicit human escalation outcomes", async () => {
  const projection: ReviewWorkspaceProjection = {
    schema_version: 1,
    revision: 4,
    mission_id: "command-deck",
    items: [
      {
        mission_id: "command-deck",
        issue_id: "ISS-01",
        issue_title: "Restore workspace session",
        session_id: "session-ISS-01-1",
        assigned_agent: "qwen-coder-local",
        status: "evidence-ready",
        lifecycle: "Ready",
        evidence_complete: true,
        missing_evidence: [],
        can_accept: true,
        evidence: {
          changed_files: ["src/App.tsx"],
          diff_summary: "Added Review Workspace.",
          commands_run: ["npm test"],
          test_results: "Tests passed.",
          risks: "Sensitive file requires operator judgment.",
          proposed_context_updates: "Document human review flow.",
          artifact_links: [],
        },
        visibility_limitations: [],
      },
    ],
  };
  const requests: unknown[] = [];
  const reviewClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "ready",
      snapshot: { ...snapshot, operations_view: "review-workspace" },
    }),
    loadReviewWorkspace: async () => ({ kind: "review-workspace", projection }),
    submitReviewDecision: async (request) => {
      requests.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 5,
          issue_id: "ISS-01",
          session_id: request.session_id,
          review_outcome: "Needs human review",
          next_action: "user-review",
          issue_lifecycle: "Needs human review",
          effect_summary: "Issue Slice records needs-human-review and waits for human review.",
        },
      };
    },
  };

  render(<App client={reviewClient} />);
  const workspace = await screen.findByRole("region", { name: "Review Workspace" });
  fireEvent.change(within(workspace).getByLabelText("Review reason session-ISS-01-1"), {
    target: { value: "Sensitive file needs human review." },
  });
  fireEvent.click(within(workspace).getByRole("button", { name: "Escalate session-ISS-01-1" }));

  expect(await screen.findByRole("status", { name: "Review decision status" })).toHaveTextContent(
    "needs-human-review",
  );
  expect(requests).toEqual([
    {
      correlation_id: "review-escalate-human-session-ISS-01-1-4",
      expected_revision: 4,
      session_id: "session-ISS-01-1",
      decision: "escalate-human",
      reason: "Sensitive file needs human review.",
    },
  ]);
});

test.each([
  [
    "stale",
    { kind: "stale" as const, code: "stale-action", message: "Workspace revision changed" },
    "Stale",
  ],
  [
    "backend rejection",
    {
      kind: "rejected" as const,
      code: "evidence-incomplete",
      message: "Evidence Package is missing: changed_files",
    },
    "Rejected",
  ],
])("review workspace shows %s decisions without changing evidence state", async (_name, result, label) => {
  const projection: ReviewWorkspaceProjection = {
    schema_version: 1,
    revision: 4,
    mission_id: "command-deck",
    items: [
      {
        mission_id: "command-deck",
        issue_id: "ISS-01",
        issue_title: "Restore workspace session",
        session_id: "session-ISS-01-1",
        assigned_agent: "qwen-coder-local",
        status: "evidence-ready",
        lifecycle: "Ready",
        evidence_complete: true,
        missing_evidence: [],
        can_accept: true,
        evidence: {
          changed_files: ["src/App.tsx"],
          diff_summary: "Added Review Workspace.",
          commands_run: ["npm test"],
          test_results: "Tests passed.",
          risks: "None.",
          proposed_context_updates: "Document Review Workspace.",
          artifact_links: [],
        },
        visibility_limitations: [],
      },
    ],
  };
  let loadSnapshotCalls = 0;
  const reviewClient: WorkspaceClient = {
    loadSnapshot: async () => {
      loadSnapshotCalls += 1;
      return {
        kind: "ready",
        snapshot: { ...snapshot, operations_view: "review-workspace" },
      };
    },
    loadReviewWorkspace: async () => ({ kind: "review-workspace", projection }),
    submitReviewDecision: async () => result,
  };

  render(<App client={reviewClient} />);
  const workspace = await screen.findByRole("region", { name: "Review Workspace" });
  fireEvent.click(within(workspace).getByRole("button", { name: "Accept session-ISS-01-1" }));

  expect(await screen.findByRole("status", { name: "Review decision status" })).toHaveTextContent(label);
  expect(screen.getByText("session-ISS-01-1")).toBeVisible();
  expect(screen.getByText("Added Review Workspace.")).toBeVisible();
  expect(loadSnapshotCalls).toBe(1);
});

test("renders an actionable empty workspace without inventing Issue Slices", async () => {
  const emptyClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "empty",
      snapshot: {
        ...snapshot,
        workspace_session: { ...snapshot.workspace_session, status: "empty" },
        active_mission: { ...snapshot.active_mission!, issue_count: 0 },
        conversation_scope: {
          kind: "working-directory",
          target_id: "/workspace/albert",
          label: "albert",
        },
        mission_board: {
          ...snapshot.mission_board,
          issue_count: 0,
          ordered_issue_ids: [],
          ready_issue_ids: [],
          approved_issue_ids: [],
        },
      },
    }),
  };

  render(<App client={emptyClient} />);

  expect(await screen.findByRole("heading", { name: "Workspace is ready" })).toBeVisible();
  expect(screen.queryByText("ISS-01")).not.toBeInTheDocument();
});

test("shows backend startup failure and retries without fabricated accepted state", async () => {
  let attempts = 0;
  const retryClient: WorkspaceClient = {
    loadSnapshot: async () => {
      attempts += 1;
      return attempts === 1
        ? { kind: "startup-failure", message: "Python backend did not start", recoverable: true }
        : { kind: "ready", snapshot };
    },
  };

  render(<App client={retryClient} />);

  expect(await screen.findByRole("alert")).toHaveTextContent("Python backend did not start");
  expect(screen.queryByRole("main", { name: "Prompt Workstation" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Retry connection" }));
  expect(await screen.findByRole("main", { name: "Prompt Workstation" })).toBeVisible();
});

test("shows persistence read failure without rendering accepted mission state", async () => {
  const failedClient: WorkspaceClient = {
    loadSnapshot: async () => ({
      kind: "persistence-read-failure",
      message: "Workspace preferences are corrupt",
      recoverable: true,
    }),
  };

  render(<App client={failedClient} />);

  expect(await screen.findByRole("alert")).toHaveTextContent("Workspace preferences are corrupt");
  expect(screen.queryByRole("heading", { name: "Command Deck Mission" })).not.toBeInTheDocument();
});

test("shows a pending semantic action and applies only its acknowledged event", async () => {
  let resolveAction!: (result: {
    kind: "acknowledged";
    acknowledgement: { correlation_id: string; outcome: "acknowledged"; revision: number };
  }) => void;
  const actionPromise = new Promise<{
    kind: "acknowledged";
    acknowledgement: { correlation_id: string; outcome: "acknowledged"; revision: number };
  }>((resolve) => {
    resolveAction = resolve;
  });
  const liveClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    submitAction: async () => actionPromise,
    loadUpdates: async () => ({
      kind: "updates",
      batch: {
        after_revision: 4,
        current_revision: 5,
        events: [
          {
            event_id: "workspace-5-activity",
            correlation_id: "operations-view-activity-4",
            revision: 5,
            kind: "workspace-preferences-updated",
            active_mission_id: "command-deck",
            conversation_scope: snapshot.conversation_scope,
            operations_view: "activity",
          },
        ],
      },
    }),
  };
  render(<App client={liveClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.click(screen.getByRole("button", { name: "Activity" }));

  expect(screen.getByRole("status", { name: "Action status" })).toHaveTextContent("Pending");
  expect(screen.getByRole("heading", { name: "Command Deck Mission" })).toBeVisible();

  await act(async () => {
    resolveAction({
      kind: "acknowledged",
      acknowledgement: {
        correlation_id: "operations-view-activity-4",
        outcome: "acknowledged",
        revision: 5,
      },
    });
  });

  expect(await screen.findByRole("heading", { name: "Activity" })).toBeVisible();
  expect(screen.getByRole("status", { name: "Action status" })).toHaveTextContent("Acknowledged");
});

test("shows offline state and reloads a fresh canonical snapshot on reconnect", async () => {
  let snapshotLoads = 0;
  let updateLoads = 0;
  const freshSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 9,
    operations_view: "activity",
  };
  const reconnectClient: WorkspaceClient = {
    loadSnapshot: async () => {
      snapshotLoads += 1;
      return { kind: "ready", snapshot: snapshotLoads === 1 ? snapshot : freshSnapshot };
    },
    loadUpdates: async (afterRevision) => {
      updateLoads += 1;
      return updateLoads === 1
        ? {
            kind: "sync-failure",
            code: "backend-startup-failure",
            message: "Orchestrator connection lost",
            recoverable: true,
          }
        : {
            kind: "updates",
            batch: { after_revision: afterRevision, current_revision: afterRevision, events: [] },
          };
    },
  };

  render(<App client={reconnectClient} syncIntervalMs={1} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  await waitFor(() =>
    expect(screen.getByRole("status", { name: "Connection status" })).toHaveTextContent("Offline"),
  );
  fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));

  expect(await screen.findByRole("heading", { name: "Activity" })).toBeVisible();
  expect(screen.getByRole("status", { name: "Connection status" })).toHaveTextContent(
    "Connected",
  );
  expect(snapshotLoads).toBe(2);
});

test("does not retarget Agent Console state when the Active Mission changes", async () => {
  let snapshotLoads = 0;
  let updateLoads = 0;
  const messageRequests: AgentConsoleMessageRequest[] = [];
  const switchedSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 9,
    active_mission: {
      id: "background-mission",
      title: "Background Mission",
      issue_count: 2,
    },
  };
  const missionSwitchClient: WorkspaceClient = {
    loadSnapshot: async () => {
      snapshotLoads += 1;
      return { kind: "ready", snapshot: snapshotLoads === 1 ? snapshot : switchedSnapshot };
    },
    loadConsoleHistory: async () => ({
      kind: "history",
      history: {
        schema_version: 1,
        messages: [
          {
            message_id: "console-000001",
            sequence: 1,
            role: "assistant",
            content: "Keep this conversation anchored",
            scope: snapshot.conversation_scope,
            outcome: "model-commentary",
            source: "frontier-model",
          },
        ],
      },
    }),
    loadUpdates: async (afterRevision) => {
      updateLoads += 1;
      return updateLoads === 1
        ? {
            kind: "sync-failure",
            code: "backend-startup-failure",
            message: "Orchestrator connection lost",
            recoverable: true,
          }
        : {
            kind: "updates",
            batch: { after_revision: afterRevision, current_revision: afterRevision, events: [] },
          };
    },
    appendConsoleMessage: async (request) => {
      messageRequests.push(request);
      return {
        kind: "message",
        message: {
          message_id: "console-000002",
          sequence: 2,
          role: "user",
          content: request.content,
          scope: snapshot.conversation_scope,
          outcome: "proposed",
          source: "mission-commander",
        },
      };
    },
  };

  render(<App client={missionSwitchClient} syncIntervalMs={1} />);
  expect(await screen.findByText("Keep this conversation anchored")).toBeVisible();
  const composer = screen.getByRole("textbox", { name: "Message Alfredo" });
  fireEvent.change(composer, { target: { value: "Continue on ISS-01" } });

  await waitFor(() =>
    expect(screen.getByRole("status", { name: "Connection status" })).toHaveTextContent("Offline"),
  );
  fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));

  expect(await screen.findByRole("heading", { name: "Background Mission" })).toBeVisible();
  expect(screen.getByText("Keep this conversation anchored")).toBeVisible();
  expect(composer).toHaveValue("Continue on ISS-01");
  expect(screen.getByText("Restore workspace session", { selector: ".scope-card strong" })).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
  await screen.findByText("Continue on ISS-01");
  expect(messageRequests[0]).toMatchObject({
    expected_revision: 9,
    scope_kind: "issue-slice",
    scope_target: "ISS-01",
    scope_label: "Restore workspace session",
  });
});

test("switches Active Mission from the compact selector while preserving console state and queue attention", async () => {
  const switchRequests: WorkspaceMissionSwitchRequest[] = [];
  const consoleMessage = {
    message_id: "console-000001",
    sequence: 1,
    role: "assistant" as const,
    content: "Keep this workspace conversation continuous",
    scope: snapshot.conversation_scope,
    outcome: "model-commentary" as const,
    source: "frontier-model",
  };
  const before: WorkspaceSnapshot = {
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-01-1",
            issue_id: "ISS-01",
            assigned_agent: "local-agent",
            status: "launched",
          },
        ],
        attention: [
          {
            attention_id: "delegation-command-deck-ISS-01",
            mission_id: "command-deck",
            kind: "delegation-approval",
            label: "ISS-01 delegation approval required",
            queue_link: "workspace-queue#delegation-command-deck-ISS-01",
          },
        ],
      },
      {
        id: "background-mission",
        title: "Background Mission",
        issue_count: 2,
        is_active: false,
        sessions: [],
        attention: [],
      },
    ],
  };
  const after: WorkspaceSnapshot = {
    ...before,
    revision: 5,
    active_mission: {
      id: "background-mission",
      title: "Background Mission",
      issue_count: 2,
    },
    mission_board: {
      prd_title: "Background Mission",
      issue_count: 2,
      ordered_issue_ids: ["BG-01", "BG-02"],
      ready_issue_ids: ["BG-01"],
      approved_issue_ids: [],
    },
    missions: before.missions?.map((mission) => ({
      ...mission,
      is_active: mission.id === "background-mission",
    })),
  };
  let loadCount = 0;
  const missionClient: WorkspaceClient = {
    loadSnapshot: async () => {
      loadCount += 1;
      return { kind: "ready", snapshot: loadCount === 1 ? before : after };
    },
    loadConsoleHistory: async () => ({
      kind: "history",
      history: { schema_version: 1, messages: [consoleMessage] },
    }),
    switchMission: async (request) => {
      switchRequests.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 5,
        },
      };
    },
  };

  render(<App client={missionClient} />);
  expect(await screen.findByText("Keep this workspace conversation continuous")).toBeVisible();

  fireEvent.change(screen.getByRole("combobox", { name: "Active Mission" }), {
    target: { value: "background-mission" },
  });

  expect(await screen.findByRole("heading", { name: "Background Mission" })).toBeVisible();
  expect(screen.getByText("Keep this workspace conversation continuous")).toBeVisible();
  expect(screen.getByText("Restore workspace session", { selector: ".scope-card strong" })).toBeVisible();
  expect(within(screen.getByLabelText("Mission Catalog")).getByText("ISS-01 delegation approval required")).toBeVisible();
  expect(screen.getByText("1 active session")).toBeVisible();
  expect(within(screen.getByRole("region", { name: "Issue Graph" })).getByText("BG-01")).toBeVisible();
  expect(switchRequests).toEqual([
    {
      correlation_id: "active-mission-background-mission-4",
      expected_revision: 4,
      active_mission_id: "background-mission",
    },
  ]);
});

test("inspects an Issue Slice from the graph without changing Conversation Scope", async () => {
  const boardSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    conversation_scope: {
      kind: "mission",
      target_id: "command-deck",
      label: "Command Deck Mission",
    },
    mission_board: {
      ...snapshot.mission_board,
      ordered_issue_ids: ["ISS-01", "ISS-02"],
      ready_issue_ids: ["ISS-01"],
      approved_issue_ids: ["ISS-01", "ISS-02"],
      issue_slices: [
        {
          issue_id: "ISS-01",
          title: "Restore workspace session",
          lifecycle: "Ready",
          progress: "1 of 2 Issue Slices launch eligible",
          launch_eligible: true,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Restore the canonical Workspace Session snapshot.",
            acceptance_criteria: ["Snapshot restores acknowledged Mission Board state."],
            evidence_requirements: ["Focused frontend interaction test."],
            source_path: ".agent/issues/01-open-and-restore-command-deck-workspace-session.md",
          },
          sessions: [],
          provenance: {
            role: "local-agent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
          },
          model_assignment: {
            agent_id: "qwen2.5-coder-14b",
            role: "local-agent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
            availability: "available",
            availability_reason: "",
            operation_status: "idle",
            failure: "",
          },
          evidence: {
            state: "not-started",
            changed_files: [],
            commands_run: [],
            test_results: "No evidence package recorded.",
            risks: "None recorded.",
            artifact_links: [],
          },
          working_context_sources: [
            {
              source_id: "shared-context:command-deck:ISS-01",
              kind: "shared-context",
              label: "Shared Context — Restore workspace session",
            },
          ],
        },
        {
          issue_id: "ISS-02",
          title: "Synchronize live state",
          lifecycle: "Approved",
          progress: "Waiting on ISS-01",
          launch_eligible: false,
          blockers: [
            {
              issue_id: "ISS-01",
              title: "Restore workspace session",
              lifecycle: "Ready",
              satisfied: false,
            },
          ],
          accepted_boundary: {
            what_to_build: "Synchronize ordered workspace events after startup.",
            acceptance_criteria: ["Stale updates never mutate accepted state."],
            evidence_requirements: ["Graph navigation interaction coverage."],
            source_path: ".agent/issues/02-synchronize-live-state-and-recover-from-disconnection.md",
          },
          sessions: [
            {
              session_id: "session-ISS-02-1",
              assigned_agent: "qwen-coder-local",
              role: "local-agent",
              provider: "ollama",
              model: "qwen2.5-coder:14b",
              status: "launched",
              stale: true,
              disconnected: true,
              operation_status: "streaming",
              failure: "",
            },
          ],
          provenance: {
            role: "local-agent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
          },
          model_assignment: {
            agent_id: "qwen2.5-coder-14b",
            role: "local-agent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
            availability: "unavailable",
            availability_reason: "Model is not installed locally.",
            operation_status: "streaming",
            failure: "",
          },
          evidence: {
            state: "missing",
            changed_files: [],
            commands_run: [],
            test_results: "No evidence package recorded.",
            risks: "Disconnected before evidence collection.",
            artifact_links: [],
          },
          working_context_sources: [
            {
              source_id: "issue:command-deck:ISS-02",
              kind: "unresolved-item",
              label: "ISS-02 — Synchronize live state",
            },
          ],
        },
      ],
    },
  };
  const inspectClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot: boardSnapshot }),
  };

  render(<App client={inspectClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.click(screen.getByRole("button", { name: "Inspect ISS-02" }));

  const inspector = screen.getByRole("region", { name: "Issue Slice Inspector" });
  expect(inspector).toHaveFocus();
  expect(within(inspector).getByRole("heading", { name: "ISS-02" })).toBeVisible();
  expect(within(inspector).getByText("Synchronize ordered workspace events after startup.")).toBeVisible();
  expect(within(inspector).getByText("Blocked by ISS-01")).toBeVisible();
  expect(within(inspector).getByText("Approved")).toBeVisible();
  expect(within(inspector).getByText("local-agent / ollama / qwen2.5-coder:14b")).toBeVisible();
  expect(within(inspector).getByText("unavailable")).toBeVisible();
  expect(within(inspector).getByText("Model is not installed locally.")).toBeVisible();
  expect(within(inspector).getByText("No evidence package recorded.")).toBeVisible();
  expect(within(inspector).getByText("ISS-02 — Synchronize live state")).toBeVisible();
  expect(screen.getByText("Command Deck Mission", { selector: ".scope-card strong" })).toBeVisible();

  fireEvent.click(within(inspector).getByRole("button", { name: "Session session-ISS-02-1" }));

  expect(within(inspector).getByText("stale")).toBeVisible();
  expect(within(inspector).getByText("disconnected")).toBeVisible();
  expect(within(inspector).getAllByText("streaming")).toHaveLength(2);
});

test("keeps Complete distinct from merged lifecycle terminology on graph inspection", async () => {
  const lifecycleSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    mission_board: {
      ...snapshot.mission_board,
      ordered_issue_ids: ["ISS-03", "ISS-04"],
      ready_issue_ids: [],
      approved_issue_ids: [],
      issue_slices: [
        {
          issue_id: "ISS-03",
          title: "Review accepted evidence",
          lifecycle: "Complete",
          progress: "Evidence accepted and PR-ready",
          launch_eligible: false,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Keep evidence-accepted work available for PR preparation.",
            acceptance_criteria: ["Complete does not imply merged."],
            evidence_requirements: ["Accepted Evidence Package."],
            source_path: ".agent/issues/03-complete.md",
          },
          sessions: [],
          provenance: { role: "frontier-reviewer", provider: "ollama", model: "qwen3.6:27b" },
          model_assignment: {
            agent_id: "qwen3.6-27b",
            role: "frontier-reviewer",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "idle",
            failure: "",
          },
          evidence: {
            state: "accepted",
            changed_files: ["src/App.tsx"],
            commands_run: ["npm test"],
            test_results: "Accepted by Frontier Reviewer.",
            risks: "None recorded.",
            artifact_links: ["app-local://evidence/ISS-03"],
          },
          working_context_sources: [],
        },
        {
          issue_id: "ISS-04",
          title: "Merged issue slice",
          lifecycle: "Merged",
          progress: "Merged",
          launch_eligible: false,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Show merged work separately from PR-ready work.",
            acceptance_criteria: ["Merged remains distinct from Complete."],
            evidence_requirements: ["Merged tracker status."],
            source_path: ".agent/issues/04-merged.md",
          },
          sessions: [],
          provenance: { role: "frontier-integrator", provider: "ollama", model: "qwen3.6:27b" },
          model_assignment: {
            agent_id: "qwen3.6-27b",
            role: "frontier-integrator",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "idle",
            failure: "",
          },
          evidence: {
            state: "accepted",
            changed_files: [],
            commands_run: [],
            test_results: "Already merged.",
            risks: "None recorded.",
            artifact_links: [],
          },
          working_context_sources: [],
        },
      ],
    },
  };

  render(<App client={{ loadSnapshot: async () => ({ kind: "ready", snapshot: lifecycleSnapshot }) }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.click(screen.getByRole("button", { name: "Inspect ISS-03" }));
  const inspector = screen.getByRole("region", { name: "Issue Slice Inspector" });
  expect(inspector).toHaveTextContent("Complete");
  expect(within(inspector).getByText("Evidence accepted and PR-ready")).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "Inspect ISS-04" }));
  const updatedInspector = screen.getByRole("region", { name: "Issue Slice Inspector" });
  expect(updatedInspector).toHaveTextContent("Merged");
  expect(within(updatedInspector).getByText("Show merged work separately from PR-ready work.")).toBeVisible();
});

test.each([
  [
    "stale",
    { kind: "stale", code: "stale-action", message: "Revision changed", current_revision: 5 },
    "Stale",
  ],
  [
    "rejected",
    { kind: "rejected", code: "invalid-action", message: "View is not allowed" },
    "Rejected",
  ],
] as const)("shows %s action outcome without changing accepted state", async (_name, result, label) => {
  const outcome: WorkspaceActionResult = result;
  const outcomeClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    submitAction: async () => outcome,
  };
  render(<App client={outcomeClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.click(screen.getByRole("button", { name: "Review" }));

  expect(await screen.findByRole("status", { name: "Action status" })).toHaveTextContent(label);
  expect(screen.getByRole("heading", { name: "Command Deck Mission" })).toBeVisible();
  expect(screen.queryByRole("heading", { name: "Review Workspace" })).not.toBeInTheDocument();
});

test("preserves Agent Console history, draft, and scope while navigating operations", async () => {
  const consoleClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadConsoleHistory: async () => ({
      kind: "history",
      history: {
        schema_version: 1,
        messages: [
          {
            message_id: "console-000001",
            sequence: 1,
            role: "assistant",
            content: "Persisted guidance",
            scope: snapshot.conversation_scope,
            outcome: "model-commentary",
            source: "frontier-model",
          },
        ],
      },
    }),
    submitAction: async (action) => ({
      kind: "acknowledged",
      acknowledgement: {
        correlation_id: action.correlation_id,
        outcome: "acknowledged",
        revision: 5,
      },
    }),
    loadUpdates: async () => ({
      kind: "updates",
      batch: {
        after_revision: 4,
        current_revision: 5,
        events: [
          {
            event_id: "workspace-5-review",
            correlation_id: "operations-view-review-workspace-4",
            revision: 5,
            kind: "workspace-preferences-updated",
            active_mission_id: "command-deck",
            conversation_scope: snapshot.conversation_scope,
            operations_view: "review-workspace",
          },
        ],
      },
    }),
  };
  render(<App client={consoleClient} />);

  expect(await screen.findByText("Persisted guidance")).toBeVisible();
  const composer = screen.getByRole("textbox", { name: "Message Alfredo" });
  fireEvent.change(composer, { target: { value: "Unfinished mission question" } });
  fireEvent.click(screen.getByRole("button", { name: "Review" }));

  expect(await screen.findByRole("heading", { name: "Review Workspace" })).toBeVisible();
  expect(screen.getByText("Persisted guidance")).toBeVisible();
  expect(composer).toHaveValue("Unfinished mission question");
  expect(screen.getByText("Restore workspace session", { selector: ".scope-card strong" })).toBeVisible();
});

test("deliberately changes Conversation Scope to the active Mission", async () => {
  const scopeRequests: WorkspaceScopeRequest[] = [];
  const scopeClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    changeScope: async (request) => {
      scopeRequests.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 5,
        },
      };
    },
    loadUpdates: async () => ({
      kind: "updates",
      batch: {
        after_revision: 4,
        current_revision: 5,
        events: [
          {
            event_id: "workspace-5-scope-mission",
            correlation_id: "conversation-scope-mission-command-deck-4",
            revision: 5,
            kind: "workspace-preferences-updated",
            active_mission_id: "command-deck",
            conversation_scope: {
              kind: "mission",
              target_id: "command-deck",
              label: "Command Deck Mission",
            },
            operations_view: "mission-board",
          },
        ],
      },
    }),
  };
  render(<App client={scopeClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("combobox", { name: "Conversation Scope" }), {
    target: { value: "mission:command-deck" },
  });
  expect(screen.getByText("Restore workspace session", { selector: ".scope-card strong" })).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Apply scope" }));

  expect(await screen.findByText("Command Deck Mission", { selector: ".scope-card strong" })).toBeVisible();
  expect(await screen.findByText(/Workstation action: Mission Commander requested Change Conversation Scope to Command Deck Mission/)).toBeVisible();
  expect(await screen.findByText(/Orchestrator accepted workstation action: Conversation Scope now targets Command Deck Mission/)).toBeVisible();
  expect(scopeRequests).toEqual([
    {
      correlation_id: "conversation-scope-mission-command-deck-4",
      expected_revision: 4,
      scope_kind: "mission",
      scope_target: "command-deck",
      scope_label: "Command Deck Mission",
    },
  ]);
});

test("shows bounded Working Context sources while retaining full Agent Console history", async () => {
  const messages = Array.from({ length: 8 }, (_, index) => ({
    message_id: `console-${String(index + 1).padStart(6, "0")}`,
    sequence: index + 1,
    role: "user" as const,
    content: `Full history message ${index + 1}`,
    scope: snapshot.conversation_scope,
    outcome: "proposed" as const,
    source: "mission-commander",
  }));
  const projection: WorkingContextProjection = {
    schema_version: 1,
    revision: 1,
    scope: snapshot.conversation_scope,
    content_character_count: 312,
    sources: [
      {
        source_id: "workspace-session:workspace-command-deck",
        kind: "workspace-session",
        label: "Workspace Session workspace-command-deck",
        content: "Workspace /workspace/albert",
        governed: true,
        eligible: false,
        disposition: "required",
      },
      {
        source_id: "shared-context:issue-slice:ISS-01",
        kind: "shared-context",
        label: "Shared Context — Restore workspace session",
        content: "Accepted mission truth",
        governed: true,
        eligible: false,
        disposition: "required",
      },
      {
        source_id: "issue:ISS-01",
        kind: "unresolved-item",
        label: "ISS-01 — Restore workspace session",
        content: "Still unresolved",
        governed: false,
        eligible: true,
        disposition: "included",
      },
      ...messages.slice(2).map((message) => ({
        source_id: `message:${message.message_id}`,
        kind: "recent-conversation" as const,
        label: `Agent Console message ${message.sequence}`,
        content: message.content,
        governed: false,
        eligible: true,
        disposition: "included" as const,
      })),
    ],
  };
  const contextClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadConsoleHistory: async () => ({
      kind: "history",
      history: { schema_version: 1, messages },
    }),
    loadWorkingContext: async () => ({ kind: "working-context", projection }),
  };

  render(<App client={contextClient} />);
  const inspector = await screen.findByRole("region", { name: "Context Inspector" });

  expect(screen.getByText("Full history message 1")).toBeVisible();
  expect(screen.getAllByText("Full history message 8")).toHaveLength(2);
  expect(within(inspector).queryByText("Full history message 1")).not.toBeInTheDocument();
  expect(within(inspector).getByText("Full history message 3")).toBeVisible();
  expect(within(inspector).getByText("6 recent-conversation sources")).toBeVisible();
  expect(within(inspector).getAllByText("Governed / required")).toHaveLength(2);
  expect(
    within(inspector).queryByRole("button", { name: /Shared Context.*exclude/i }),
  ).not.toBeInTheDocument();
});

test("updates eligible Working Context only after Orchestrator acknowledgement", async () => {
  let resolveCuration!: (result: {
    kind: "acknowledged";
    acknowledgement: { outcome: "acknowledged"; revision: number };
  }) => void;
  const curationPromise = new Promise<{
    kind: "acknowledged";
    acknowledgement: { outcome: "acknowledged"; revision: number };
  }>((resolve) => {
    resolveCuration = resolve;
  });
  const requests: WorkingContextCurationRequest[] = [];
  const source = {
    source_id: "issue:ISS-01",
    kind: "unresolved-item" as const,
    label: "ISS-01 — Restore workspace session",
    content: "Still unresolved",
    governed: false,
    eligible: true,
  };
  let loads = 0;
  const contextClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadWorkingContext: async () => {
      loads += 1;
      return {
        kind: "working-context",
        projection: {
          schema_version: 1,
          revision: loads === 1 ? 1 : 2,
          scope: snapshot.conversation_scope,
          content_character_count: loads === 1 ? 16 : 0,
          sources: [{ ...source, disposition: loads === 1 ? "included" : "excluded" }],
        },
      };
    },
    curateWorkingContext: async (request) => {
      requests.push(request);
      return curationPromise;
    },
  };

  render(<App client={contextClient} />);
  const inspector = await screen.findByRole("region", { name: "Context Inspector" });
  fireEvent.click(
    within(inspector).getByRole("button", {
      name: "Exclude ISS-01 — Restore workspace session",
    }),
  );

  expect(within(inspector).getByRole("status", { name: "Context curation status" })).toHaveTextContent(
    "Pending",
  );
  expect(within(inspector).getByText("included")).toBeVisible();

  await act(async () => {
    resolveCuration({
      kind: "acknowledged",
      acknowledgement: { outcome: "acknowledged", revision: 2 },
    });
  });

  expect(await within(inspector).findByText("excluded")).toBeVisible();
  expect(within(inspector).getByRole("status", { name: "Context curation status" })).toHaveTextContent(
    "Acknowledged",
  );
  expect(requests).toEqual([
    {
      source_id: "issue:ISS-01",
      disposition: "excluded",
      expected_context_revision: 1,
    },
  ]);
});

test("submits a message with the displayed acknowledged scope", async () => {
  const messageRequests: AgentConsoleMessageRequest[] = [];
  const messageClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    appendConsoleMessage: async (request) => {
      messageRequests.push(request);
      return {
        kind: "message",
        message: {
          message_id: "console-000001",
          sequence: 1,
          role: "user",
          content: request.content,
          scope: snapshot.conversation_scope,
          outcome: "proposed",
          source: "mission-commander",
        },
      };
    },
  };
  render(<App client={messageClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  const composer = screen.getByRole("textbox", { name: "Message Alfredo" });

  fireEvent.change(composer, { target: { value: "Explain the restore boundary" } });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  expect(await screen.findByText("Explain the restore boundary")).toBeVisible();
  expect(composer).toHaveValue("");
  expect(messageRequests).toEqual([
    {
      role: "user",
      content: "Explain the restore boundary",
      outcome: "proposed",
      source: "mission-commander",
      expected_revision: 4,
      scope_kind: "issue-slice",
      scope_target: "ISS-01",
      scope_label: "Restore workspace session",
    },
  ]);
});

test("renders every Agent Console outcome as a distinct sourced record", async () => {
  const outcomes = [
    "proposed",
    "pending",
    "acknowledged",
    "rejected",
    "model-commentary",
  ] as const;
  const outcomeClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadConsoleHistory: async () => ({
      kind: "history",
      history: {
        schema_version: 1,
        messages: outcomes.map((outcome, index) => ({
          message_id: `console-${String(index + 1).padStart(6, "0")}`,
          sequence: index + 1,
          role: outcome === "model-commentary" ? "assistant" : "system",
          content: `Record ${outcome}`,
          scope: snapshot.conversation_scope,
          outcome,
          source: outcome === "model-commentary" ? "frontier-model" : "orchestrator",
        })),
      },
    }),
  };

  const { container } = render(<App client={outcomeClient} />);
  await screen.findByText("Record model-commentary");

  for (const outcome of outcomes) {
    const record = container.querySelector(`[data-outcome="${outcome}"]`);
    expect(record).toHaveTextContent(`Record ${outcome}`);
    expect(record).toHaveTextContent(outcome);
  }
  expect(container.querySelector('[data-outcome="model-commentary"]')).not.toHaveAttribute(
    "data-outcome",
    "acknowledged",
  );
});

test.each([
  [
    "working-directory:/workspace/albert",
    {
      kind: "working-directory" as const,
      target_id: "/workspace/albert",
      label: "albert",
    },
  ],
  [
    "issue-slice:ISS-02",
    { kind: "issue-slice" as const, target_id: "ISS-02", label: "ISS-02" },
  ],
])("deliberately applies Conversation Scope target %s", async (selection, expectedScope) => {
  const scopeRequests: WorkspaceScopeRequest[] = [];
  const scopeClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    changeScope: async (request) => {
      scopeRequests.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 5,
        },
      };
    },
    loadUpdates: async () => ({
      kind: "updates",
      batch: {
        after_revision: 4,
        current_revision: 5,
        events: [
          {
            event_id: `workspace-5-${expectedScope.kind}`,
            correlation_id: `conversation-scope-${expectedScope.kind}-${expectedScope.target_id}-4`,
            revision: 5,
            kind: "workspace-preferences-updated",
            active_mission_id: "command-deck",
            conversation_scope: expectedScope,
            operations_view: "mission-board",
          },
        ],
      },
    }),
  };
  render(<App client={scopeClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("combobox", { name: "Conversation Scope" }), {
    target: { value: selection },
  });
  fireEvent.click(screen.getByRole("button", { name: "Apply scope" }));

  await waitFor(() =>
    expect(screen.getByText(expectedScope.target_id, { selector: ".scope-card code" })).toBeVisible(),
  );
  expect(scopeRequests).toEqual([
    {
      correlation_id: `conversation-scope-${expectedScope.kind}-${expectedScope.target_id}-4`,
      expected_revision: 4,
      scope_kind: expectedScope.kind,
      scope_target: expectedScope.target_id,
      scope_label: expectedScope.label,
    },
  ]);
});
