/// <reference types="node" />
import {
  act,
  createEvent,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { readFileSync } from "node:fs";
import { App, isExactAdHocDelegationBoundary } from "./App";
import { MissionExecutionTree } from "./MissionExecutionTree";
import type { MissionExecutionTreeProjection } from "./workstation-projection";
import type {
  AdHocDelegationProposalRequest,
  AdditionalPathGrantDenialRequest,
  AdditionalPathGrantDenialResult,
  AgentConsoleMessageRequest,
  MissionDraftProjection,
  ReviewDecisionRequest,
  ReviewWorkspaceProjection,
  SessionOutputEvent,
  SessionOutputSubscriptionRequest,
  SessionArtifactReadRequest,
  SessionArtifactReadResult,
  WorkspaceActionResult,
  ActivityJournalFilters,
  ActivityJournalProjection,
  WorkspaceMissionSwitchRequest,
  WorkspaceIssueSliceSummary,
  WorkspaceQueueItem,
  WorkspaceQueueProjection,
  WorkspaceScopeRequest,
  WorkspaceSnapshot,
  WorkingContextCurationRequest,
  WorkingContextProjection,
  WorkstationActionRequest,
} from "./contracts";
import type { SessionOutputSubscriptionState, WorkspaceClient } from "./workspace-client";

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
        issue_slices: [
          appIssueSlice({
            issue_id: "ISS-01",
            title: "Restore workspace session",
            work_type: "AFK",
            tracker_status: "ready-for-agent",
            lifecycle: "Ready",
            launch_eligible: true,
          }),
        ],
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

test("renders the typed Wayfinder Chart route and pending Shared Understanding Gate", async () => {
  const submitAdHocDelegationProposal = vi.fn();
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        appendConsoleMessage: async (request) => ({
          kind: "message",
          message: {
            message_id: "console-wayfinder-000001",
            sequence: 1,
            role: "user",
            content: request.content,
            scope: snapshot.conversation_scope,
            outcome: "proposed",
            source: "mission-commander",
          },
        }),
        generateConsoleResponse: async () => ({
          kind: "message",
          message: {
            message_id: "console-wayfinder-000002",
            sequence: 2,
            role: "assistant",
            content: "Wayfinder Chart mode is active.",
            scope: snapshot.conversation_scope,
            outcome: "acknowledged",
            source: "orchestrator",
            correlation_id: "wayfinder-entry:console-wayfinder-000001",
            action_phase: "wayfinder-chart-entered",
          },
          route: { intent: "discussion", task_request: "", acceptance_criteria: [] },
          wayfinder: {
            mode: "chart",
            gate: { status: "pending", opened_by: "", receipt_id: "" },
            flow: {
              flow_id: "wayfinder-console-wayfinder-000001",
              mode: "chart",
              originating_message_id: "console-wayfinder-000001",
              scope: snapshot.conversation_scope,
              reference: "",
            },
            continuing: false,
            turn_complete: true,
          },
        }),
        submitAdHocDelegationProposal,
      }}
    />,
  );

  await screen.findByRole("heading", { name: "Command Deck Mission" });
  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Start a new project for release planning." },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  expect(await screen.findByRole("status", { name: "Wayfinder route" })).toHaveTextContent(
    "Chart mode",
  );
  expect(screen.getByRole("status", { name: "Wayfinder route" })).toHaveTextContent(
    "Shared Understanding Gate pending",
  );
  expect(submitAdHocDelegationProposal).not.toHaveBeenCalled();
});

function appIssueSlice(
  overrides: Partial<WorkspaceIssueSliceSummary> & Pick<WorkspaceIssueSliceSummary, "issue_id" | "title">,
): WorkspaceIssueSliceSummary {
  return {
    issue_id: overrides.issue_id,
    title: overrides.title,
    work_type: overrides.work_type,
    tracker_status: overrides.tracker_status,
    lifecycle: overrides.lifecycle ?? "Approved",
    progress: overrides.progress ?? "Ready for assignment",
    launch_eligible: overrides.launch_eligible ?? false,
    blockers: overrides.blockers ?? [],
    accepted_boundary: overrides.accepted_boundary ?? {
      what_to_build: `Build ${overrides.title}.`,
      acceptance_criteria: [`${overrides.title} is accepted.`],
      evidence_requirements: ["App projection tests pass."],
      source_path: `.agent/issues/${overrides.issue_id}.md`,
    },
    sessions: overrides.sessions ?? [],
    provenance: overrides.provenance ?? {
      role: "local-agent",
      provider: "ollama",
      model: "qwen3.6:27b",
    },
    model_assignment: overrides.model_assignment ?? {
      agent_id: "",
      role: "local-agent",
      provider: "ollama",
      model: "qwen3.6:27b",
      availability: "available",
      availability_reason: "",
      operation_status: "",
      failure: "",
    },
    evidence: overrides.evidence ?? {
      state: "missing",
      changed_files: [],
      commands_run: [],
      test_results: "No evidence package recorded.",
      risks: "None recorded.",
      artifact_links: [],
    },
    working_context_sources: overrides.working_context_sources ?? [],
  };
}

function reviewReadyTreeSnapshot(
  sessionIds: readonly string[],
  revision = 13,
): WorkspaceSnapshot {
  return {
    ...snapshot,
    revision,
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-REVIEW"],
      ready_issue_ids: [],
      approved_issue_ids: ["ISS-REVIEW"],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-REVIEW",
          title: "Review tree evidence",
          lifecycle: "Approved",
          progress: "Evidence Package is ready for review",
          sessions: sessionIds.map((session_id) => ({
            session_id,
            assigned_agent: "review-agent",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            status: "evidence-ready",
            stale: false,
            disconnected: false,
            operation_status: "awaiting-review",
            failure: "",
          })),
          evidence: {
            state: "complete",
            changed_files: ["mission-control/src/App.tsx"],
            commands_run: ["npm test -- --run App.test.tsx"],
            test_results: "Focused App tests passed.",
            risks: "None recorded.",
            artifact_links: sessionIds.map((sessionId) => `app-local://evidence/${sessionId}`),
          },
        }),
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: sessionIds.map((session_id) => ({
          session_id,
          issue_id: "ISS-REVIEW",
          assigned_agent: "review-agent",
          status: "evidence-ready",
          role: "local-agent",
          provider: "ollama",
          model: "qwen3.6:27b",
        })),
        attention: [],
      },
    ],
  };
}

test("renders the canonical Mission Execution Tree and scopes detailed output to its exact inspector", async () => {
  let outputHandler: ((event: SessionOutputEvent) => void) | undefined;
  let outputStateHandler: ((state: SessionOutputSubscriptionState) => void) | undefined;
  const unsubscribe = vi.fn();
  const subscribeToSessionOutput = vi.fn(
    (
      request: SessionOutputSubscriptionRequest,
      onEvent: (event: SessionOutputEvent) => void,
      onState?: (state: SessionOutputSubscriptionState) => void,
    ) => {
      expect(request).toEqual({ mission_id: "command-deck", session_id: "session-ISS-TREE-1" });
      outputHandler = onEvent;
      outputStateHandler = onState;
      return unsubscribe;
    },
  );
  const treeSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 17,
    active_mission: { id: "command-deck", title: "Command Deck Mission", issue_count: 1 },
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-TREE"],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-TREE",
          title: "Ship the inspectable execution tree",
          lifecycle: "Running",
          progress: "Local Agent is executing",
          launch_eligible: false,
        }),
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
            session_id: "session-ISS-TREE-1",
            issue_id: "ISS-TREE",
            assigned_agent: "qwen-coder-local",
            status: "running",
            role: "implementer",
            provider: "ollama",
            model: "qwen3.6:27b",
            task_title: "Build the Mission Execution Tree",
            operation_status: "streaming",
            changed_files: ["mission-control/src/MissionExecutionTree.tsx"],
            commands_run: ["npm test -- --run MissionExecutionTree"],
            test_results: "Tree tests pass.",
            evidence_correlation_id: "evidence:command-deck:session-ISS-TREE-1",
            supervision_receipt_id: "supervision-receipt:tree-recovery",
            supervision_outcome: "recovered",
            automatic_recovery_count: 1,
          },
        ],
        attention: [],
      },
    ],
  };

  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: treeSnapshot }),
        subscribeToSessionOutput,
      }}
    />,
  );

  const tree = await screen.findByRole("region", { name: "Mission Execution Tree" });
  expect(within(tree).getByText("1 Issue Slices")).toBeVisible();
  expect(within(tree).getByText("1 Local Agent sessions")).toBeVisible();
  const issueNode = within(tree).getByRole("treeitem", {
    name: /Ship the inspectable execution tree/,
  });
  const sessionNode = within(tree).getByRole("treeitem", { name: /session-ISS-TREE-1/ });
  act(() => {
    sessionNode.focus();
    fireEvent.keyDown(sessionNode, { key: "ArrowUp" });
  });
  expect(document.activeElement).toBe(issueNode);
  act(() => {
    fireEvent.keyDown(issueNode, { key: "ArrowDown" });
  });
  expect(document.activeElement).toBe(sessionNode);
  act(() => {
    fireEvent.click(sessionNode);
  });

  const inspector = await screen.findByRole("region", {
    name: "session-ISS-TREE-1 execution inspector",
  });
  expect(inspector).toHaveFocus();
  expect(within(inspector).getByText("Task").parentElement).toHaveTextContent(
    "Build the Mission Execution Tree",
  );
  expect(within(inspector).getByText("Latest update").parentElement).toHaveTextContent(
    "Tree tests pass.",
  );
  expect(within(inspector).getByText("mission-control/src/MissionExecutionTree.tsx")).toBeVisible();
  expect(
    within(inspector).getByText(
      "Supervision receipt: supervision-receipt:tree-recovery · recovered",
    ),
  ).toBeVisible();
  expect(within(inspector).getByText("Automatic recovery: 1 of 1 used")).toBeVisible();
  expect(subscribeToSessionOutput).toHaveBeenCalledTimes(1);
  expect(outputHandler).toBeDefined();
  expect(within(inspector).getByText("Subscribing…")).toBeVisible();

  act(() => outputStateHandler?.({ kind: "subscribed" }));
  expect(within(inspector).getByText("Subscribed while this inspector is open")).toBeVisible();

  act(() => {
    outputHandler?.({
      schema_version: 1,
      mission_id: "command-deck",
      session_id: "session-ISS-TREE-1",
      sequence: 1,
      content: "pytest -q\n",
      phase: "streaming",
    });
    outputHandler?.({
      schema_version: 1,
      mission_id: "other-mission",
      session_id: "session-ISS-TREE-1",
      sequence: 2,
      content: "must not render",
      phase: "streaming",
    });
  });
  const output = within(inspector).getByLabelText("Detailed Local Agent output content");
  expect(output.textContent).toBe("pytest -q\n");
  expect(output).not.toHaveTextContent("must not render");

  act(() => outputStateHandler?.({
    kind: "failure",
    code: "session-output-unavailable",
    message: "The output reader temporarily lost the runner journal.",
    recoverable: true,
    retrying: false,
  }));
  expect(within(inspector).getByRole("alert")).toHaveTextContent(
    "The output reader temporarily lost the runner journal.",
  );
  expect(output.textContent).toBe("pytest -q\n");
  fireEvent.click(within(inspector).getByRole("button", { name: "Retry output" }));
  await waitFor(() => expect(subscribeToSessionOutput).toHaveBeenCalledTimes(2));
  act(() => outputHandler?.({
    schema_version: 1,
    mission_id: "command-deck",
    session_id: "session-ISS-TREE-1",
    sequence: 1,
    content: "pytest -q\n",
    phase: "streaming",
  }));
  expect(output.textContent).toBe("pytest -q\n");
  act(() => {
    for (let sequence = 2; sequence <= 257; sequence += 1) {
      outputHandler?.({
        schema_version: 1,
        mission_id: "command-deck",
        session_id: "session-ISS-TREE-1",
        sequence,
        content: `event ${sequence}${sequence === 257 ? "" : "\n"}`,
        phase: "streaming",
      });
    }
  });
  expect(output.textContent).toBe(
    ["pytest -q", ...Array.from({ length: 256 }, (_, index) => `event ${index + 2}`)].join("\n"),
  );

  act(() => {
    fireEvent.click(
      within(inspector).getByRole("button", { name: "Close Mission Execution Tree inspector" }),
    );
  });
  expect(unsubscribe).toHaveBeenCalledTimes(2);
  expect(
    screen.queryByRole("region", { name: "session-ISS-TREE-1 execution inspector" }),
  ).not.toBeInTheDocument();
  await waitFor(() => expect(sessionNode).toHaveFocus());
});

test("archives a completed Issue Slice only after an acknowledged Mission Work action", async () => {
  const archiveSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 18,
    active_mission: { id: "command-deck", title: "Command Deck Mission", issue_count: 1 },
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-ARCHIVE"],
      ready_issue_ids: [],
      approved_issue_ids: [],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-ARCHIVE",
          title: "Retain accepted execution history",
          lifecycle: "Complete",
          progress: "Evidence accepted and PR-ready",
          evidence: {
            state: "accepted",
            changed_files: ["src/history.ts"],
            commands_run: ["npm test -- archive"],
            test_results: "Accepted evidence is retained.",
            risks: "None recorded.",
            artifact_links: [],
          },
        }),
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [],
        attention: [],
      },
    ],
  };
  const submitWorkstationAction = vi.fn(async (request: WorkstationActionRequest) => ({
    kind: "acknowledged" as const,
    acknowledgement: {
      correlation_id: request.correlation_id,
      outcome: "acknowledged" as const,
      revision: 19,
      action_type: "issue-archive" as const,
      issue_id: "ISS-ARCHIVE",
      session_id: "",
      effect_summary:
        "Mission Commander archived ISS-ARCHIVE; its sessions, evidence, and Activity Journal history remain inspectable.",
    },
  }));

  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: archiveSnapshot }),
        submitWorkstationAction,
      }}
    />,
  );

  const inspector = await openExecutionInspector("ISS-ARCHIVE", "issue-slice");
  const archive = within(inspector).getByRole("button", { name: "Archive completed Issue Slice" });
  expect(archive).toHaveAccessibleDescription(
    /archives the completed ISS-ARCHIVE subtree from active Mission Work while retaining its identity, sessions, Evidence Packages, and Activity Journal history/,
  );
  expect(screen.queryByText(/Mission Commander archived ISS-ARCHIVE/)).not.toBeInTheDocument();

  fireEvent.click(archive);

  await waitFor(() =>
    expect(submitWorkstationAction).toHaveBeenCalledWith(
      expect.objectContaining({
        action_type: "issue-archive",
        target: { kind: "issue-slice", id: "ISS-ARCHIVE" },
        issue_id: "ISS-ARCHIVE",
        expected_revision: 18,
      }),
    ),
  );
  expect(await within(inspector).findByText(/Mission Commander archived ISS-ARCHIVE/)).toBeVisible();
});

test("requires exact-session confirmation before submitting a typed retirement discard", async () => {
  const sessionId = "session-ISS-RETIRE-1";
  const retirementSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 31,
    active_mission: { id: "command-deck", title: "Command Deck Mission", issue_count: 1 },
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-RETIRE"],
      ready_issue_ids: [],
      approved_issue_ids: [],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-RETIRE",
          title: "Retire retained worktree",
          lifecycle: "Complete",
          progress: "Retirement is blocked on retained worktree cleanup",
          sessions: [
            {
              session_id: sessionId,
              assigned_agent: "qwen-coder-local",
              role: "local-agent",
              provider: "ollama",
              model: "qwen3.6:27b",
              status: "reviewed",
              stale: false,
              disconnected: false,
              operation_status: "completed",
              failure: "",
            },
          ],
        }),
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
            session_id: sessionId,
            issue_id: "ISS-RETIRE",
            assigned_agent: "qwen-coder-local",
            status: "reviewed",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            session_revision: 9,
            retirement_phase: "retirement-blocked",
            retirement_blocked_reason: "Retained worktree still has open handles.",
            retirement_record: {
              manifest_sha256: "b".repeat(64),
              snapshot_bytes: 4096,
              pinned: false,
              payload_disposition: "retained",
            },
            retirement_actions: {
              retry: true,
              inspect: true,
              export: true,
              discard: true,
            },
          },
        ],
        attention: [],
      },
    ],
  };
  const submitWorkstationAction = vi.fn(async (request: WorkstationActionRequest) => ({
    kind: "acknowledged" as const,
    acknowledgement: {
      correlation_id: request.correlation_id,
      outcome: "acknowledged" as const,
      revision: 32,
      action_type: "retirement-discard" as const,
      issue_id: "ISS-RETIRE",
      session_id: sessionId,
      effect_summary: `Mission Commander discarded retained worktree ${sessionId}.`,
    },
  }));

  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: retirementSnapshot }),
        submitWorkstationAction,
      }}
    />,
  );

  const inspector = await openExecutionInspector(sessionId);
  expect(within(inspector).getByRole("heading", { name: "Retirement Record" })).toBeVisible();
  const discard = within(inspector).getByRole("button", { name: "Discard Retained Worktree" });
  expect(discard).toBeDisabled();

  fireEvent.change(
    within(inspector).getByRole("textbox", { name: "Discard Retained Worktree reason" }),
    { target: { value: "The retained payload was exported and reviewed." } },
  );
  fireEvent.change(
    within(inspector).getByRole("textbox", { name: "Discard Retained Worktree confirmation" }),
    { target: { value: "wrong-session" } },
  );
  expect(discard).toBeDisabled();
  fireEvent.change(
    within(inspector).getByRole("textbox", { name: "Discard Retained Worktree confirmation" }),
    { target: { value: sessionId } },
  );
  expect(discard).toBeEnabled();
  fireEvent.click(discard);

  await waitFor(() =>
    expect(submitWorkstationAction).toHaveBeenCalledWith(
      expect.objectContaining({
        action_type: "retirement-discard",
        actor: "mission-commander",
        expected_revision: 9,
        target: { kind: "agent-session", id: sessionId },
        mission_id: "command-deck",
        issue_id: "ISS-RETIRE",
        session_id: sessionId,
        reason: "The retained payload was exported and reviewed.",
        confirmation: sessionId,
      }),
    ),
  );
});

test("restores an archived Issue Slice only after an acknowledged Mission Work action", async () => {
  const archivedSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 21,
    active_mission: { id: "command-deck", title: "Command Deck Mission", issue_count: 1 },
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-ARCHIVE"],
      ready_issue_ids: [],
      approved_issue_ids: [],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-ARCHIVE",
          title: "Retained accepted execution history",
          lifecycle: "Complete",
          progress: "History retained outside active work",
        }),
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [],
        attention: [],
        archived_issue_ids: ["ISS-ARCHIVE"],
      },
    ],
  };
  const submitWorkstationAction = vi.fn(async (request: WorkstationActionRequest) => ({
    kind: "acknowledged" as const,
    acknowledgement: {
      correlation_id: request.correlation_id,
      outcome: "acknowledged" as const,
      revision: 22,
      action_type: "issue-restore" as const,
      issue_id: "ISS-ARCHIVE",
      session_id: "",
      effect_summary:
        "Mission Commander restored ISS-ARCHIVE to active Mission Work with its sessions, evidence, and Activity Journal history intact.",
    },
  }));

  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: archivedSnapshot }),
        submitWorkstationAction,
      }}
    />,
  );

  const inspector = await openExecutionInspector("ISS-ARCHIVE", "issue-slice");
  const restore = within(inspector).getByRole("button", { name: "Restore Issue Slice" });
  expect(restore).toHaveAccessibleDescription(
    /restores the retained ISS-ARCHIVE subtree to active Mission Work with its identity, sessions, Evidence Packages, and Activity Journal history intact/,
  );
  expect(screen.queryByText(/Mission Commander restored ISS-ARCHIVE/)).not.toBeInTheDocument();

  fireEvent.click(restore);

  await waitFor(() =>
    expect(submitWorkstationAction).toHaveBeenCalledWith(
      expect.objectContaining({
        action_type: "issue-restore",
        target: { kind: "issue-slice", id: "ISS-ARCHIVE" },
        issue_id: "ISS-ARCHIVE",
        expected_revision: 21,
      }),
    ),
  );
  expect(await within(inspector).findByText(/Mission Commander restored ISS-ARCHIVE/)).toBeVisible();
});

test("keeps Issue Slice, Ad Hoc Delegation, and Repair disclosure separately operable", () => {
  const projection: MissionExecutionTreeProjection = {
    schema_version: 1,
    revision: 1,
    root_id: "mission:command-deck",
    counts: {
      issue_slices: 1,
      ad_hoc_delegations: 1,
      local_agent_sessions: 3,
      repairs: 1,
      blockers: 0,
      evidence_packages: 0,
    },
    nodes: [
      {
        id: "mission:command-deck", kind: "mission", identity: "command-deck", title: "Command Deck",
        parent_id: null, parent_session_id: null, lineage: "root", depth: 0,
        child_ids: ["issue:ISS-01", "ad-hoc:ADHOC-1"], state: "working", status: "Active Mission",
        shape: "circle", risk: "none", summary: "Mission", inspectable: false, attention: false,
        issue: null, session: null, card: null,
      },
      {
        id: "issue:ISS-01", kind: "issue-slice", identity: "ISS-01", title: "Issue work",
        parent_id: "mission:command-deck", parent_session_id: null, lineage: "root", depth: 1,
        child_ids: ["session:issue-parent"], state: "working", status: "Running", shape: "circle",
        risk: "none", summary: "Issue", inspectable: true, attention: false, issue: null, session: null, card: null,
      },
      {
        id: "session:issue-parent", kind: "agent-session", identity: "issue-parent", title: "Parent session",
        parent_id: "issue:ISS-01", parent_session_id: null, lineage: "root", depth: 2,
        child_ids: ["session:repair-child"], state: "working", status: "Running", shape: "circle",
        risk: "none", summary: "Parent", inspectable: true, attention: false, issue: null, session: null, card: null,
      },
      {
        id: "session:repair-child", kind: "agent-session", identity: "repair-child", title: "Repair session",
        parent_id: "session:issue-parent", parent_session_id: "issue-parent", lineage: "repair", depth: 3,
        child_ids: [], state: "queued", status: "Queued", shape: "repair",
        risk: "none", summary: "Repair", inspectable: true, attention: false, issue: null, session: null, card: null,
      },
      {
        id: "ad-hoc:ADHOC-1", kind: "ad-hoc-delegation", identity: "ADHOC-1", title: "Ad Hoc work",
        parent_id: "mission:command-deck", parent_session_id: null, lineage: "root", depth: 1,
        child_ids: ["session:ad-hoc"], state: "queued", status: "Queued", shape: "square",
        risk: "none", summary: "Ad Hoc", inspectable: true, attention: false, issue: null, session: null, card: null,
      },
      {
        id: "session:ad-hoc", kind: "agent-session", identity: "ad-hoc-session", title: "Ad Hoc session",
        parent_id: "ad-hoc:ADHOC-1", parent_session_id: null, lineage: "root", depth: 2,
        child_ids: [], state: "queued", status: "Queued", shape: "square",
        risk: "none", summary: "Ad Hoc", inspectable: true, attention: false, issue: null, session: null, card: null,
      },
    ],
  };
  render(
    <MissionExecutionTree
      projection={projection}
      selectedNodeId={null}
      onSelectNode={vi.fn()}
      onCloseInspector={vi.fn()}
      outputLines={[]}
      outputState="unavailable"
    />,
  );

  const issue = screen.getByRole("treeitem", { name: /issue-slice ISS-01/ });
  const adHoc = screen.getByRole("treeitem", { name: /ad-hoc-delegation ADHOC-1/ });
  const parentSession = screen.getByRole("treeitem", { name: /Local Agent session issue-parent/ });
  fireEvent.keyDown(issue, { key: "ArrowLeft" });
  expect(screen.queryByRole("treeitem", { name: /Local Agent session issue-parent/ })).not.toBeInTheDocument();
  fireEvent.keyDown(issue, { key: "ArrowRight" });
  expect(screen.getByRole("treeitem", { name: /Local Agent session issue-parent/ })).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "Collapse Ad Hoc Delegation ADHOC-1" }));
  expect(screen.queryByRole("treeitem", { name: /Local Agent session ad-hoc-session/ })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Expand Ad Hoc Delegation ADHOC-1" }));
  expect(screen.getByRole("treeitem", { name: /Local Agent session ad-hoc-session/ })).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "Collapse Local Agent session issue-parent" }));
  expect(screen.queryByRole("treeitem", { name: /Local Agent session repair-child/ })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Expand Local Agent session issue-parent" }));
  expect(screen.getByRole("treeitem", { name: /Local Agent session repair-child/ })).toBeVisible();
  expect(parentSession).toHaveAttribute("aria-expanded", "true");
});

test("uses a focus-contained dialog inspector at the documented constrained breakpoint", async () => {
  const originalMatchMedia = Object.getOwnPropertyDescriptor(window, "matchMedia");
  const media = {
    matches: true,
    media: "(max-width: 680px)",
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  };
  Object.defineProperty(window, "matchMedia", { configurable: true, value: vi.fn(() => media) });
  const dialogSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-DIALOG"],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-DIALOG",
          title: "Reach a constrained inspector",
          lifecycle: "Running",
          launch_eligible: false,
          sessions: [
            {
              session_id: "session-ISS-DIALOG-1", assigned_agent: "local-agent", role: "local-agent",
              provider: "ollama", model: "qwen", status: "running", stale: false, disconnected: false,
              operation_status: "streaming", failure: "",
            },
          ],
        }),
      ],
    },
    missions: [
      {
        id: "command-deck", title: "Command Deck Mission", issue_count: 1, is_active: true,
        sessions: [
          {
            session_id: "session-ISS-DIALOG-1", issue_id: "ISS-DIALOG", assigned_agent: "local-agent",
            status: "running", role: "local-agent", provider: "ollama", model: "qwen",
          },
        ],
        attention: [],
      },
    ],
  };
  try {
    render(<App client={{ loadSnapshot: async () => ({ kind: "ready", snapshot: dialogSnapshot }) }} />);
    const node = await screen.findByRole("treeitem", { name: /Local Agent session session-ISS-DIALOG-1/ });
    fireEvent.click(node);
    const dialog = await screen.findByRole("dialog", {
      name: "Local Agent session / session-ISS-DIALOG-1",
    });
    const close = within(dialog).getByRole("button", { name: "Close Mission Execution Tree inspector" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(close).toHaveFocus();
    const focusableButtons = within(dialog)
      .getAllByRole("button")
      .filter((button) => !button.hasAttribute("disabled"));
    const lastFocusable = focusableButtons.at(-1)!;
    fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(lastFocusable).toHaveFocus();
    fireEvent.keyDown(lastFocusable, { key: "Tab" });
    expect(close).toHaveFocus();
    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(node).toHaveFocus());
    expect(screen.queryByRole("dialog", { name: "Local Agent session / session-ISS-DIALOG-1" })).not.toBeInTheDocument();
  } finally {
    if (originalMatchMedia) Object.defineProperty(window, "matchMedia", originalMatchMedia);
    else Reflect.deleteProperty(window, "matchMedia");
  }
});

async function openCommandAudit() {
  fireEvent.click(screen.getByRole("button", { name: "Open command audit" }));
  return await screen.findByRole("region", { name: "Shell Terminal" });
}

async function openDetailViews() {
  fireEvent.click(await screen.findByRole("button", { name: "Open detail views" }));
  return await screen.findByRole("region", { name: "Workstation Detail Views" });
}

function executionTree(): HTMLElement {
  return screen.getByRole("region", { name: "Mission Execution Tree" });
}

function executionTreeNode(identity: string, kind = "Local Agent session"): HTMLElement {
  const escapedIdentity = identity.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return within(executionTree()).getByRole("treeitem", {
    name: new RegExp(`${kind} ${escapedIdentity}`),
  });
}

async function openExecutionInspector(
  identity: string,
  kind = "Local Agent session",
): Promise<HTMLElement> {
  const tree = await screen.findByRole("region", { name: "Mission Execution Tree" });
  const escapedIdentity = identity.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  fireEvent.click(
    within(tree).getByRole("treeitem", {
      name: new RegExp(`${kind} ${escapedIdentity}`),
    }),
  );
  const name = `${identity} execution inspector`;
  const dialogForInspector = (): HTMLElement | null =>
    [...document.querySelectorAll<HTMLElement>('[role="dialog"]')].find(
      (candidate) => candidate.getAttribute("aria-label") === name,
    ) ?? null;
  await waitFor(() =>
    expect(
      dialogForInspector() ?? screen.queryByRole("region", { name }),
    ).not.toBeNull(),
  );
  return dialogForInspector() ?? screen.getByRole("region", { name });
}

async function openContextInspector() {
  const button = await screen.findByRole("button", { name: "Inspect context" });
  await waitFor(() => expect(button).toBeEnabled());
  fireEvent.click(button);
  return await screen.findByRole("region", { name: "Context Inspector" });
}

function closeContextInspector() {
  fireEvent.click(screen.getByRole("button", { name: "Close context", expanded: true }));
}

function expectPromptScope(label: string) {
  expect(screen.getByLabelText("Prompt status line")).not.toHaveTextContent(/Conversation Scope/i);
  expect(screen.getByRole("button", { name: "Inspect context", expanded: false })).toHaveTextContent(
    `Context · ${label}`,
  );
}

function expectConversationScopeValue(value: string) {
  expect(screen.getByRole("combobox", { name: "Conversation Scope" })).toHaveValue(value);
}

function closeCommandAudit() {
  fireEvent.click(screen.getByRole("button", { name: "Close command audit", expanded: true }));
}

test("records usable S8 before hydrated S9 only after their rendered boundaries", async () => {
  const marks: Array<{
    stage: string;
    boundary: string;
    detail: Readonly<Record<string, unknown>>;
    workstationVisible: boolean;
  }> = [];
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        recordPerformanceMark: async (request) => {
          marks.push({
            stage: request.stage,
            boundary: request.boundary,
            detail: request.detail,
            workstationVisible: Boolean(
              document.querySelector('main[aria-label="Prompt Workstation"]'),
            ),
          });
          return { recorded: true };
        },
      }}
    />,
  );

  expect(await screen.findByRole("main", { name: "Prompt Workstation" })).toBeVisible();
  await waitFor(() =>
    expect(
      marks.some((mark) => mark.stage === "S9" && mark.boundary === "end"),
    ).toBe(true),
  );
  const index = (stage: string, boundary: string) =>
    marks.findIndex((mark) => mark.stage === stage && mark.boundary === boundary);

  expect(index("S8", "start")).toBeGreaterThanOrEqual(0);
  expect(index("S8", "end")).toBeGreaterThan(index("S8", "start"));
  expect(index("S9", "start")).toBeGreaterThan(index("S8", "end"));
  expect(index("S9", "end")).toBeGreaterThan(index("S9", "start"));
  expect(marks[index("S8", "end")].workstationVisible).toBe(true);
  expect(marks[index("S9", "end")].workstationVisible).toBe(true);
  expect(marks[index("S9", "end")].detail).toMatchObject({
    workspace_session_id: "workspace-command-deck",
    active_mission_id: "command-deck",
    hydration: {
      capabilities: true,
      consoleHistory: true,
      workingContext: true,
      workspaceQueue: true,
      shell: true,
    },
  });
});

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
            last_activity_at: "2026-07-12T08:31:45+00:00",
            runner_started_at: "2026-07-12T08:30:00+00:00",
            launch_correlation_id: "workstation-launch-ISS-01-1",
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
            entity_id: "ISS-02",
            queue_item_id: "",
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
    /\.deck-grid\s*\{[^}]*min-width:\s*0;[^}]*grid-template-columns:\s*minmax\(0,\s*1\.65fr\)\s+minmax\(360px,\s*0\.85fr\)/s,
  );
  const transcript = screen.getByRole("region", { name: "Prompt Transcript" });
  expect(within(transcript).getByText("Implement the next Alfredo workstation slice.")).toBeVisible();
  expect(within(transcript).getByText(/durable and route execution/)).toBeVisible();
  expect(
    within(transcript).queryByText(
      "Workstation action pending: ISS-02 delegation approval required.",
    ),
  ).not.toBeInTheDocument();
  expect(within(transcript).getByText("Orchestrator started canonical session session-ISS-01-1 for ISS-01 on qwen-coder-local.")).toBeVisible();
  expect(within(transcript).getByText("Receipt workstation-launch-ISS-01-1 · running")).toBeVisible();
  expect(screen.getByRole("complementary", { name: "Mission Work" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Mission Work" })).toBeVisible();
  expect(executionTree()).toBeVisible();
  expect(screen.queryByRole("heading", { name: "Active Workstations" })).not.toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Workstation Cards" })).not.toBeInTheDocument();
  expect(screen.queryByRole("tablist", { name: "Agent Workstation views" })).not.toBeInTheDocument();
  expect(screen.queryByRole("tab", { name: "Workstations" })).not.toBeInTheDocument();
  expect(screen.queryByRole("tab", { name: "Shell Terminal" })).not.toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Shell Terminal" })).not.toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Command history" })).not.toBeInTheDocument();
  expect(screen.queryByRole("textbox", { name: "Requested paths" })).not.toBeInTheDocument();
  expect(screen.queryByRole("combobox", { name: "Access level" })).not.toBeInTheDocument();
  expect(screen.queryByRole("textbox", { name: "Grant path" })).not.toBeInTheDocument();
  const runningNode = executionTreeNode("session-ISS-01-1");
  expect(runningNode).toHaveAttribute("data-state", "working");
  expect(runningNode).toHaveAttribute("data-lineage", "root");
  const runningInspector = await openExecutionInspector("session-ISS-01-1");
  expect(within(runningInspector).getByText("qwen3.6:27b")).toBeVisible();
  expect(within(runningInspector).getByText("running", { selector: "dd" })).toBeVisible();

  const statusLine = screen.getByLabelText("Prompt status line");
  expect(within(statusLine).getByText("Connection Connected")).toBeVisible();
  expect(statusLine).toHaveTextContent("Controller default · default");
  expect(statusLine).not.toHaveTextContent(/Conversation Scope/i);
  expect(within(statusLine).getByText("Workspace albert")).toBeVisible();
  expect(within(statusLine).getByText("Execution Waiting approval")).toBeVisible();
  expectPromptScope("Restore workspace session");
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

test("fails closed when a workstation last-activity timestamp is malformed", async () => {
  const malformedActivitySnapshot: WorkspaceSnapshot = {
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-01-malformed",
            issue_id: "ISS-01",
            assigned_agent: "timestamp-agent",
            status: "running",
            last_activity_at: "0",
          },
        ],
        attention: [],
      },
    ],
  };

  render(
    <App client={{ loadSnapshot: async () => ({ kind: "ready", snapshot: malformedActivitySnapshot }) }} />,
  );

  expect(await screen.findByRole("region", { name: "Mission Execution Tree" })).toBeVisible();
  const node = executionTreeNode("session-ISS-01-malformed");
  expect(node).toBeVisible();
  expect(node.querySelector("time")).toBeNull();
  expect(screen.queryByText("1970-01-01")).not.toBeInTheDocument();
});

test("default workstation design keeps operations panels out of the terminal-first surface", async () => {
  let workingContextLoads = 0;
  const projection: WorkingContextProjection = {
    schema_version: 1,
    revision: 1,
    scope: snapshot.conversation_scope,
    content_character_count: 84,
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
    ],
  };

  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadWorkingContext: async () => {
          workingContextLoads += 1;
          return { kind: "working-context", projection };
        },
      }}
    />,
  );

  const console = await screen.findByRole("region", { name: "Agent Console" });
  const missionWork = screen.getByRole("complementary", { name: "Mission Work" });
  await waitFor(() => expect(workingContextLoads).toBe(1));

  expect(within(console).getByRole("region", { name: "Prompt Transcript" })).toBeVisible();
  expect(within(console).getByRole("region", { name: "Prompt Composer" })).toBeVisible();
  expect(within(console).queryByText(/Conversation Scope \//)).not.toBeInTheDocument();
  expect(within(console).queryByRole("region", { name: "Context Inspector" })).not.toBeInTheDocument();
  expect(within(missionWork).getByRole("region", { name: "Mission Execution Tree" })).toBeVisible();
  expect(within(missionWork).queryByRole("region", { name: "Workstation Cards" })).not.toBeInTheDocument();
  expect(within(missionWork).getByRole("table", { name: "Issue Assignment Board" })).toBeVisible();
  expect(within(missionWork).queryByRole("region", { name: "Workstation Detail Views" })).not.toBeInTheDocument();
  expect(within(missionWork).queryByRole("navigation", { name: "Workstation detail views" })).not.toBeInTheDocument();
  expect(within(missionWork).queryByRole("heading", { name: "Mission Board" })).not.toBeInTheDocument();
});

test("keeps restored operations detail closed behind explicit Mission Work request", async () => {
  const restoredDetailSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    operations_view: "activity",
  };

  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: restoredDetailSnapshot }),
      }}
    />,
  );

  const console = await screen.findByRole("region", { name: "Agent Console" });
  const missionWork = screen.getByRole("complementary", { name: "Mission Work" });

  expect(within(console).getByRole("region", { name: "Prompt Transcript" })).toBeVisible();
  expect(within(missionWork).getByRole("region", { name: "Mission Execution Tree" })).toBeVisible();
  expect(within(missionWork).getByRole("table", { name: "Issue Assignment Board" })).toBeVisible();
  expect(within(missionWork).getByRole("button", { name: "Open detail views" })).toBeVisible();
  expect(within(missionWork).queryByRole("region", { name: "Workstation Detail Views" })).not.toBeInTheDocument();
  expect(within(missionWork).queryByRole("heading", { name: "Activity" })).not.toBeInTheDocument();
  expect(stylesSource).toMatch(
    /\.agent-workstations\s*\{[^}]*grid-template-rows:\s*auto\s+minmax\(0,\s*1fr\)/s,
  );
  expect(stylesSource).toMatch(/\.mission-work-scroll\s*\{[^}]*min-height:\s*0;[^}]*overflow:\s*auto;/s);
});

test("keeps review-ready workstation evidence affordances visible beside the Agent Console", async () => {
  const reviewReadySnapshot: WorkspaceSnapshot = {
    ...snapshot,
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-04"],
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
  const node = executionTreeNode("session-ISS-04-1");
  expect(node).toHaveAttribute("data-state", "decision-needed");
  const inspector = await openExecutionInspector("session-ISS-04-1");
  expect(within(inspector).getByText("App tests passed.")).toBeVisible();
  expect(within(inspector).getByText("mission-control/src/App.tsx")).toBeVisible();
  expect(within(inspector).getByText("Evidence and touched files")).toBeVisible();
  expect(
    within(inspector).getByRole("button", {
      name: "Evidence Package session-ISS-04-1",
    }),
  ).toBeVisible();
  expect(within(inspector).getByRole("button", { name: "Accept evidence" })).toBeVisible();
  expect(within(inspector).getByRole("button", { name: "Request repair" })).toBeVisible();
  expect(within(inspector).getByRole("textbox", { name: "Request repair reason" })).toBeVisible();
  expect(within(inspector).getByText(/marks session-ISS-04-1 complete and PR-ready/)).toBeVisible();
});

test("renders Issue Assignment Board rows with local detail and keeps scope controls out of tickets", async () => {
  const appendConsoleMessage = vi.fn();
  const assignmentSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 4,
    conversation_scope: {
      kind: "issue-slice",
      target_id: "ISS-READY",
      label: "Unassigned ready work",
    },
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 3,
      ordered_issue_ids: ["ISS-READY", "ISS-BLOCKED", "ISS-ACTIVE"],
      ready_issue_ids: ["ISS-READY"],
      approved_issue_ids: ["ISS-READY", "ISS-BLOCKED", "ISS-ACTIVE"],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-READY",
          title: "Unassigned ready work",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: true,
        }),
        appIssueSlice({
          issue_id: "ISS-BLOCKED",
          title: "Blocked dependency work",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          progress: "Waiting on release seam verification",
          blockers: [
            {
              issue_id: "ISS-00",
              title: "Release seam",
              lifecycle: "Ready",
              satisfied: false,
            },
          ],
        }),
        appIssueSlice({
          issue_id: "ISS-ACTIVE",
          title: "Active implementation",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          progress: "Runner streaming edits",
          sessions: [
            {
              session_id: "session-ISS-ACTIVE-1",
              assigned_agent: "qwen-coder-local",
              role: "local-agent",
              provider: "ollama",
              model: "qwen3.6:27b",
              status: "launched",
              stale: false,
              disconnected: false,
              operation_status: "streaming",
              failure: "",
            },
          ],
          model_assignment: {
            agent_id: "qwen-coder-local",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            availability: "available",
            availability_reason: "",
            operation_status: "streaming",
            failure: "",
          },
        }),
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
            session_id: "session-ISS-ACTIVE-1",
            issue_id: "ISS-ACTIVE",
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
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: assignmentSnapshot }),
        loadConsoleHistory: async () => ({
          kind: "history",
          history: {
            schema_version: 1,
            messages: [
              {
                message_id: "console-000001",
                sequence: 1,
                role: "user",
                content: "Keep issue browsing local.",
                scope: assignmentSnapshot.conversation_scope,
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
  const missionWork = screen.getByRole("complementary", { name: "Mission Work" });
  const tree = within(missionWork).getByRole("region", { name: "Mission Execution Tree" });
  const board = within(missionWork).getByRole("table", { name: "Issue Assignment Board" });

  expect(tree.compareDocumentPosition(board) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(within(board).getByRole("row", { name: /ISS-READY Unassigned ready work/ })).toHaveTextContent(
    "Unassigned",
  );
  expect(within(board).getByRole("row", { name: /ISS-READY Unassigned ready work/ })).toHaveTextContent(
    "Ready",
  );
  expect(within(board).getByRole("row", { name: /ISS-BLOCKED Blocked dependency work/ })).toHaveTextContent(
    "ISS-00 Ready open - Release seam",
  );
  expect(within(board).getByRole("row", { name: /ISS-ACTIVE Active implementation/ })).toHaveTextContent(
    "session-ISS-ACTIVE-1",
  );
  expect(within(board).queryByRole("button", { name: /Set scope to/ })).not.toBeInTheDocument();
  expect(within(board).queryByText(/Conversation Scope/)).not.toBeInTheDocument();

  fireEvent.click(within(board).getByRole("button", { name: "Inspect assignment ISS-BLOCKED" }));

  const detail = within(missionWork).getByRole("region", { name: "Issue Assignment Detail" });
  expect(detail).toHaveFocus();
  expect(within(detail).getByText("Blocked dependency work")).toBeVisible();
  expect(within(detail).getByText("ISS-00 Ready open - Release seam")).toBeVisible();
  expect(appendConsoleMessage).not.toHaveBeenCalled();
  expect(within(transcript).getByText("Keep issue browsing local.")).toBeVisible();
  expect(within(transcript).queryByText(/Release seam/)).not.toBeInTheDocument();

});

test("submits Issue Assignment Board launch through a typed workstation action", async () => {
  const before: WorkspaceSnapshot = {
    ...snapshot,
    revision: 9,
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-READY"],
      ready_issue_ids: ["ISS-READY"],
      approved_issue_ids: ["ISS-READY"],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-READY",
          title: "Unassigned launchable work",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: true,
        }),
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [],
        attention: [],
      },
    ],
  };
  const after: WorkspaceSnapshot = {
    ...before,
    revision: 10,
    mission_board: {
      ...before.mission_board,
      issue_slices: before.mission_board.issue_slices?.map((issue) => ({
        ...issue,
        launch_eligible: false,
        sessions: [
          {
            session_id: "session-ISS-READY-1",
            assigned_agent: "qwen-coder-local",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            status: "launched",
            stale: false,
            disconnected: false,
            operation_status: "streaming",
            failure: "",
          },
        ],
      })),
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-READY-1",
            issue_id: "ISS-READY",
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
  const actions: WorkstationActionRequest[] = [];
  const sessionRuns: Array<{ session_id: string; mission_id?: string }> = [];
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
              revision: 10,
              action_type: "issue-launch",
              issue_id: "ISS-READY",
              session_id: "session-ISS-READY-1",
              effect_summary: "Orchestrator launched ISS-READY as session-ISS-READY-1.",
            },
          };
        },
        runWorkstationSession: async (request) => {
          sessionRuns.push(request);
          return {
            kind: "session-finished",
            session: {
              schema_version: 1,
              mission_id: "command-deck",
              session_id: request.session_id,
              issue_id: "ISS-READY",
              status: "evidence-ready",
              runner_started_at: "2026-07-10T08:00:00Z",
              runner_ended_at: "2026-07-10T08:00:01Z",
              runner_exit_status: 0,
              evidence_valid: true,
            },
          };
        },
      }}
    />,
  );

  const board = await screen.findByRole("table", { name: "Issue Assignment Board" });
  fireEvent.click(within(board).getByRole("button", { name: "Launch ISS-READY" }));

  await waitFor(() =>
    expect(actions).toEqual([
      {
        correlation_id: "workstation-issue-launch-command-deck-ISS-READY-9",
        action_type: "issue-launch",
        actor: "mission-commander",
        expected_revision: 9,
        target: { kind: "issue-slice", id: "ISS-READY" },
        mission_id: "command-deck",
        issue_id: "ISS-READY",
        session_id: undefined,
        agent_id: undefined,
        reason: undefined,
        allowed_paths: [],
        command_policy: {},
      },
    ]),
  );
  await waitFor(() =>
    expect(sessionRuns).toEqual([
      { session_id: "session-ISS-READY-1", mission_id: "command-deck" },
    ]),
  );
  expect(screen.getByText(/session-ISS-READY-1 is queued and starting in the background/)).toBeVisible();
  expect(screen.getByText("Orchestrator validating workstation action.")).toBeVisible();
  expect(await screen.findByText(/Orchestrator accepted workstation action: Orchestrator launched ISS-READY/)).toBeVisible();
  expect(screen.getAllByText(/session-ISS-READY-1/).length).toBeGreaterThan(0);
});

test("keeps Mission-qualified Issue Assignment actions pending until their result is visible", async () => {
  const actionSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 9,
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-READY"],
      ready_issue_ids: ["ISS-READY"],
      approved_issue_ids: ["ISS-READY"],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-READY",
          title: "Mission-qualified launch feedback",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: true,
        }),
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [],
        attention: [],
      },
    ],
  };
  let resolveAction!: (
    result: Awaited<ReturnType<NonNullable<WorkspaceClient["submitWorkstationAction"]>>>,
  ) => void;
  const actionResult = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["submitWorkstationAction"]>>>
  >((resolve) => {
    resolveAction = resolve;
  });
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: actionSnapshot }),
        submitWorkstationAction: async () => actionResult,
      }}
    />,
  );

  const board = await screen.findByRole("table", { name: "Issue Assignment Board" });
  const launch = within(board).getByRole("button", { name: "Launch ISS-READY" });
  fireEvent.click(launch);

  const pending = await within(board).findByRole("status", {
    name: "ISS-READY issue assignment action state",
  });
  expect(pending).toHaveTextContent("pending: Waiting for Orchestrator acknowledgement");
  expect(pending).toHaveFocus();
  expect(launch).toBeDisabled();

  await act(async () => {
    resolveAction({
      kind: "stale",
      code: "stale-action",
      message: "Workspace revision advanced to 10.",
      current_revision: 10,
    });
  });

  const stale = await within(board).findByRole("status", {
    name: "ISS-READY issue assignment action state",
  });
  expect(stale).toHaveTextContent("stale: Workspace revision advanced to 10");
  expect(stale).toHaveTextContent("Refresh the canonical workspace state and retry");
  expect(launch).toBeEnabled();
});

test("approves an agent-ready review row and reloads it as launchable", async () => {
  const before: WorkspaceSnapshot = {
    ...snapshot,
    revision: 30,
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-REVIEW"],
      ready_issue_ids: [],
      approved_issue_ids: [],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-REVIEW",
          title: "Approve local tracker work",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          lifecycle: "Needs review",
          launch_eligible: false,
        }),
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [],
        attention: [],
      },
    ],
  };
  const after: WorkspaceSnapshot = {
    ...before,
    revision: 31,
    mission_board: {
      ...before.mission_board,
      ready_issue_ids: ["ISS-REVIEW"],
      approved_issue_ids: ["ISS-REVIEW"],
      issue_slices: before.mission_board.issue_slices?.map((issue) => ({
        ...issue,
        lifecycle: "Approved",
        launch_eligible: true,
      })),
    },
  };
  const actions: WorkstationActionRequest[] = [];
  const runWorkstationSession = vi.fn();
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
              revision: 31,
              action_type: "issue-approve",
              issue_id: "ISS-REVIEW",
              session_id: "",
              effect_summary: "Mission Commander approved ISS-REVIEW for launch.",
            },
          };
        },
        runWorkstationSession,
      }}
    />,
  );

  const board = await screen.findByRole("table", { name: "Issue Assignment Board" });
  const approve = within(board).getByRole("button", { name: "Approve for launch ISS-REVIEW" });
  expect(approve).toBeEnabled();
  fireEvent.click(approve);

  await waitFor(() =>
    expect(actions).toEqual([
      {
        correlation_id: "workstation-issue-approve-command-deck-ISS-REVIEW-30",
        action_type: "issue-approve",
        actor: "mission-commander",
        expected_revision: 30,
        target: { kind: "issue-slice", id: "ISS-REVIEW" },
        mission_id: "command-deck",
        issue_id: "ISS-REVIEW",
        session_id: undefined,
        agent_id: undefined,
        reason: undefined,
        allowed_paths: [],
        command_policy: {},
      },
    ]),
  );
  expect(await within(board).findByRole("button", { name: "Launch ISS-REVIEW" })).toBeEnabled();
  expect(runWorkstationSession).not.toHaveBeenCalled();
  expect(
    await screen.findByText(/Orchestrator accepted workstation action: Mission Commander approved ISS-REVIEW/),
  ).toBeVisible();
});

test("withholds Issue Assignment Board model assignment without an eligible capability catalog", async () => {
  const assignmentSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 11,
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-READY"],
      ready_issue_ids: ["ISS-READY"],
      approved_issue_ids: ["ISS-READY"],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-READY",
          title: "Unassigned launchable work",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: true,
        }),
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [],
        attention: [],
      },
    ],
  };
  const actions: WorkstationActionRequest[] = [];

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
              revision: 12,
              action_type: "model-assignment-change",
              issue_id: "ISS-READY",
              session_id: "",
              effect_summary: "Mission Commander assigned ISS-READY to gemma4-12b.",
            },
          };
        },
      }}
    />,
  );

  const board = await screen.findByRole("table", { name: "Issue Assignment Board" });
  const assign = within(board).getByRole("button", { name: "Assign model ISS-READY" });
  expect(assign).toBeDisabled();
  expect(
    within(board).queryByRole("combobox", { name: "Issue assignment agent ISS-READY" }),
  ).not.toBeInTheDocument();
  expect(
    within(board).queryByRole("textbox", { name: "Issue assignment agent ISS-READY" }),
  ).not.toBeInTheDocument();
  expect(
    within(board).getByRole("status", {
      name: "Issue assignment worker unavailable ISS-READY",
    }),
  ).toHaveTextContent("Worker assignment unavailable");
  fireEvent.change(within(board).getByRole("textbox", { name: "Issue assignment reason ISS-READY" }), {
    target: { value: "Use the available local worker." },
  });
  expect(assign).toBeDisabled();
  expect(actions).toEqual([]);
});

test("Issue Assignment Board exposes only ungated local workers for manual assignment", async () => {
  const assignmentSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-READY"],
      ready_issue_ids: ["ISS-READY"],
      approved_issue_ids: ["ISS-READY"],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-READY",
          title: "Unassigned launchable work",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: true,
        }),
      ],
    },
  };

  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: assignmentSnapshot }),
        loadAgentCapabilities: async () => ({
          kind: "capabilities",
          catalog: {
            schema_version: 1,
            default_agent_id: "",
            commands: [],
            skills: [],
            agents: [
              {
                id: "gemma-worker",
                role: "local-agent",
                provider: "ollama",
                runner: "ollama",
                model: "gemma4:12b",
                routing: "worker",
                availability: "available",
                availability_reason: "",
                assignable: true,
                delegate_only: false,
                requires_approval: false,
              },
              {
                id: "deepseek-delegate",
                role: "local-agent",
                provider: "ollama",
                runner: "ollama",
                model: "deepseek-r1:14b",
                routing: "worker",
                availability: "available",
                availability_reason: "",
                assignable: false,
                delegate_only: true,
                requires_approval: true,
              },
              {
                id: "cloud-worker",
                role: "local-agent",
                provider: "ollama",
                runner: "ollama",
                model: "qwen3-coder:cloud",
                routing: "worker",
                availability: "available",
                availability_reason: "",
                assignable: true,
                delegate_only: false,
                requires_approval: false,
              },
              {
                id: "remote-provider-worker",
                role: "local-agent",
                provider: "remote",
                runner: "fake",
                model: "remote-provider-worker:14b",
                routing: "worker",
                availability: "available",
                availability_reason: "",
                assignable: true,
                delegate_only: false,
                requires_approval: false,
              },
              {
                id: "remote-runner-worker",
                role: "local-agent",
                provider: "local",
                runner: "remote-api",
                model: "remote-runner-worker:14b",
                routing: "worker",
                availability: "available",
                availability_reason: "",
                assignable: true,
                delegate_only: false,
                requires_approval: false,
              },
              {
                id: "gated-worker",
                role: "local-agent",
                provider: "ollama",
                runner: "ollama",
                model: "gated:14b",
                routing: "worker",
                availability: "available",
                availability_reason: "",
                assignable: true,
                delegate_only: false,
                requires_approval: true,
              },
              {
                id: "routing-controller",
                role: "LOCAL-AGENT",
                provider: "ollama",
                runner: "ollama",
                model: "controller:14b",
                routing: "CONTROLLER",
                availability: "available",
                availability_reason: "",
                assignable: true,
                delegate_only: false,
                requires_approval: false,
              },
              {
                id: "missing-authority-metadata",
                role: "local-agent",
                provider: "ollama",
                runner: "ollama",
                model: "unknown-boundary:14b",
                routing: "worker",
                availability: "available",
                availability_reason: "",
              },
            ],
          },
        }),
      }}
    />,
  );

  const board = await screen.findByRole("table", { name: "Issue Assignment Board" });
  const assignmentAgent = within(board).getByRole("combobox", {
    name: "Issue assignment agent ISS-READY",
  });
  expect(within(assignmentAgent).getByRole("option", { name: /gemma-worker/ })).toBeVisible();
  expect(within(assignmentAgent).queryByRole("option", { name: /deepseek-delegate/ })).toBeNull();
  expect(within(assignmentAgent).queryByRole("option", { name: /cloud-worker/ })).toBeNull();
  expect(
    within(assignmentAgent).queryByRole("option", { name: /remote-provider-worker/ }),
  ).toBeNull();
  expect(
    within(assignmentAgent).queryByRole("option", { name: /remote-runner-worker/ }),
  ).toBeNull();
  expect(within(assignmentAgent).queryByRole("option", { name: /gated-worker/ })).toBeNull();
  expect(within(assignmentAgent).queryByRole("option", { name: /routing-controller/ })).toBeNull();
  expect(
    within(assignmentAgent).queryByRole("option", { name: /missing-authority-metadata/ }),
  ).toBeNull();
});

test("submits review-ready workstation card decisions through typed review validation", async () => {
  const before: WorkspaceSnapshot = {
    ...snapshot,
    revision: 13,
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-REVIEW"],
      ready_issue_ids: [],
      approved_issue_ids: ["ISS-REVIEW"],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-REVIEW",
          title: "Review card evidence",
          lifecycle: "Approved",
          progress: "Evidence Package is ready for review",
          sessions: [
            {
              session_id: "session-ISS-REVIEW-1",
              assigned_agent: "review-agent",
              role: "local-agent",
              provider: "ollama",
              model: "qwen3.6:27b",
              status: "evidence-ready",
              stale: false,
              disconnected: false,
              operation_status: "awaiting-review",
              failure: "",
            },
          ],
          evidence: {
            state: "complete",
            changed_files: ["mission-control/src/App.tsx"],
            commands_run: ["npm test -- --run App.test.tsx"],
            test_results: "Focused App tests passed.",
            risks: "None recorded.",
            artifact_links: ["app-local://evidence/session-ISS-REVIEW-1"],
          },
        }),
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
            session_id: "session-ISS-REVIEW-1",
            issue_id: "ISS-REVIEW",
            assigned_agent: "review-agent",
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
  const after: WorkspaceSnapshot = {
    ...before,
    revision: 14,
    mission_board: {
      ...before.mission_board,
      issue_slices: before.mission_board.issue_slices?.map((issue) => ({
        ...issue,
        lifecycle: "Complete",
        progress: "Evidence accepted and PR-ready",
        evidence: { ...issue.evidence, state: "accepted" },
      })),
    },
  };
  const requests: ReviewDecisionRequest[] = [];
  let snapshotLoads = 0;

  render(
    <App
      client={{
        loadSnapshot: async () => {
          snapshotLoads += 1;
          return { kind: "ready", snapshot: snapshotLoads === 1 ? before : after };
        },
        submitReviewDecision: async (request) => {
          requests.push(request);
          return {
            kind: "acknowledged",
            acknowledgement: {
              correlation_id: request.correlation_id,
              outcome: "acknowledged",
              revision: 14,
              issue_id: "ISS-REVIEW",
              session_id: "session-ISS-REVIEW-1",
              review_outcome: "Approved",
              next_action: "prepare-pr",
              issue_lifecycle: "Complete",
              effect_summary: "Issue Slice becomes Complete and PR-ready; it is not marked merged.",
            },
          };
        },
      }}
    />,
  );

  const inspector = await openExecutionInspector("session-ISS-REVIEW-1");
  const repair = within(inspector).getByRole("button", { name: "Request repair" });
  expect(repair).toBeDisabled();
  expect(within(inspector).getByRole("button", { name: "Escalate human review" })).toBeEnabled();
  fireEvent.change(within(inspector).getByRole("textbox", { name: "Request repair reason" }), {
    target: { value: "Repair acceptance copy." },
  });
  expect(repair).toBeEnabled();
  fireEvent.click(within(inspector).getByRole("button", { name: "Accept evidence" }));

  await waitFor(() =>
    expect(requests).toEqual([
      {
        correlation_id: "review-accept-command-deck-session-ISS-REVIEW-1-13",
        action_type: "review-decision",
        actor: "mission-commander",
        expected_revision: 13,
        target: {
          kind: "agent-session",
          id: "session-ISS-REVIEW-1",
        },
        mission_id: "command-deck",
        session_id: "session-ISS-REVIEW-1",
        decision: "accept",
        reason: "",
      },
    ]),
  );
  expect(screen.getByText("Orchestrator validating workstation action.")).toBeVisible();
  expect(await screen.findByText(/Orchestrator accepted workstation action: Issue Slice becomes Complete and PR-ready/)).toBeVisible();
});

test("scopes execution-tree Review Decision pending and acknowledged feedback to one decision and session", async () => {
  const firstSessionId = "session-ISS-REVIEW-A";
  const secondSessionId = "session-ISS-REVIEW-B";
  const before = reviewReadyTreeSnapshot([firstSessionId, secondSessionId]);
  const after = reviewReadyTreeSnapshot([firstSessionId, secondSessionId], 14);
  type ReviewDecisionResult = Awaited<ReturnType<NonNullable<WorkspaceClient["submitReviewDecision"]>>>;
  const decisionResolvers = new Map<string, (value: ReviewDecisionResult) => void>();
  let snapshotLoads = 0;

  render(
    <App
      client={{
        loadSnapshot: async () => {
          snapshotLoads += 1;
          return { kind: "ready", snapshot: snapshotLoads === 1 ? before : after };
        },
        submitReviewDecision: async (request) =>
          new Promise<ReviewDecisionResult>((resolve) => {
            decisionResolvers.set(request.session_id ?? "", resolve);
          }),
      }}
    />,
  );

  const firstInspector = await openExecutionInspector(firstSessionId);
  const firstAccept = within(firstInspector).getByRole("button", { name: "Accept evidence" });
  const firstRepair = within(firstInspector).getByRole("button", { name: "Request repair" });
  const firstEscalate = within(firstInspector).getByRole("button", { name: "Escalate human review" });
  fireEvent.change(within(firstInspector).getByRole("textbox", { name: "Request repair reason" }), {
    target: { value: "Repair remains available while acceptance is pending." },
  });
  fireEvent.click(firstAccept);
  await waitFor(() => expect(firstAccept).toBeDisabled());
  expect(firstRepair).toBeEnabled();
  expect(firstEscalate).toBeEnabled();
  expect(
    within(firstInspector).getByText(
      `Waiting for Orchestrator acknowledgement: Accept evidence for ${firstSessionId}.`,
    ),
  ).toBeVisible();

  const secondInspector = await openExecutionInspector(secondSessionId);
  const secondAccept = within(secondInspector).getByRole("button", { name: "Accept evidence" });
  expect(secondAccept).toBeEnabled();
  expect(within(secondInspector).getByRole("button", { name: "Escalate human review" })).toBeEnabled();
  fireEvent.click(secondAccept);
  await waitFor(() => expect(secondAccept).toBeDisabled());

  const firstStillPendingInspector = await openExecutionInspector(firstSessionId);
  expect(within(firstStillPendingInspector).getByRole("button", { name: "Accept evidence" })).toBeDisabled();

  await act(async () => {
    decisionResolvers.get(firstSessionId)?.({
      kind: "acknowledged",
      acknowledgement: {
        correlation_id: `review-accept-command-deck-${firstSessionId}-13`,
        outcome: "acknowledged",
        revision: 14,
        issue_id: "ISS-REVIEW",
        session_id: firstSessionId,
        review_outcome: "Approved",
        next_action: "prepare-pr",
        issue_lifecycle: "Complete",
        effect_summary: "Acceptance is recorded for the first Local Agent session.",
      },
    });
  });

  const secondStillPendingInspector = await openExecutionInspector(secondSessionId);
  expect(within(secondStillPendingInspector).getByRole("button", { name: "Accept evidence" })).toBeDisabled();
  await act(async () => {
    decisionResolvers.get(secondSessionId)?.({
      kind: "acknowledged",
      acknowledgement: {
        correlation_id: `review-accept-command-deck-${secondSessionId}-13`,
        outcome: "acknowledged",
        revision: 14,
        issue_id: "ISS-REVIEW",
        session_id: secondSessionId,
        review_outcome: "Approved",
        next_action: "prepare-pr",
        issue_lifecycle: "Complete",
        effect_summary: "Acceptance is recorded for the second Local Agent session.",
      },
    });
  });

  const acknowledgedInspector = await openExecutionInspector(firstSessionId);
  expect(
    within(acknowledgedInspector).getByText("Acceptance is recorded for the first Local Agent session."),
  ).toBeVisible();
});

test.each([
  [
    "stale",
    "Accept evidence",
    {
      kind: "stale" as const,
      code: "stale-action",
      message: "The review snapshot changed before acceptance.",
    },
    "The review snapshot changed before acceptance.",
  ],
  [
    "rejected",
    "Escalate human review",
    {
      kind: "rejected" as const,
      code: "evidence-incomplete",
      message: "The evidence package is incomplete.",
    },
    "The evidence package is incomplete.",
  ],
  [
    "reload failure",
    "Accept evidence",
    {
      kind: "acknowledged" as const,
      acknowledgement: {
        correlation_id: "review-accept-command-deck-session-ISS-REVIEW-OUTCOME-13",
        outcome: "acknowledged" as const,
        revision: 14,
        issue_id: "ISS-REVIEW",
        session_id: "session-ISS-REVIEW-OUTCOME",
        review_outcome: "Approved",
        next_action: "prepare-pr",
        issue_lifecycle: "Complete",
        effect_summary: "The canonical reload must still succeed.",
      },
    },
    "Review acknowledged but canonical snapshot reload failed.",
  ],
] as const)("shows %s Review Decision feedback beside the exact inspector action", async (
  _name,
  actionLabel,
  result,
  expectedMessage,
) => {
  const sessionId = "session-ISS-REVIEW-OUTCOME";
  const before = reviewReadyTreeSnapshot([sessionId]);
  let snapshotLoads = 0;
  render(
    <App
      client={{
        loadSnapshot: async () => {
          snapshotLoads += 1;
          if (snapshotLoads === 1) return { kind: "ready", snapshot: before };
          if (result.kind === "acknowledged") {
            return {
              kind: "persistence-read-failure",
              message: "Canonical state could not be reread.",
              recoverable: true,
            };
          }
          return { kind: "ready", snapshot: before };
        },
        submitReviewDecision: async () => result,
      }}
    />,
  );

  const inspector = await openExecutionInspector(sessionId);
  fireEvent.click(within(inspector).getByRole("button", { name: actionLabel }));
  expect(await within(inspector).findByText(expectedMessage, { exact: false })).toBeVisible();
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
          work_type: "AFK",
          tracker_status: "ready-for-agent",
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
            artifact_links: [
              "app-local://evidence/session-ISS-01-1",
              "app-local://missions/command-deck/sessions/session-ISS-01-1/artifacts/review_diff/review.diff",
            ],
          },
          working_context_sources: [],
        },
        {
          issue_id: "ISS-02",
          title: "Keep selected issue visible",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
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
        schema_version: 1,
        selected_agent: "qwen3.6-27b",
        selected_model: "qwen3.6:27b",
        starting_location: "/workspace",
        coding_workspace: "/workspace/albert",
        active_mission: "command-deck",
        phase: "workspace-ready",
        runtime_root: "/runtime/alfredo",
        recent_workspaces: ["/workspace/albert"],
      },
    }),
  };

  const first = render(<App client={continuityClient} />);
  fireEvent.click(await screen.findByRole("button", { name: "Inspect assignment ISS-02" }));
  const inspector = await openExecutionInspector("session-ISS-01-1");
  fireEvent.click(within(inspector).getByRole("button", { name: "Diff mission-control/src/App.tsx" }));
  await openCommandAudit();

  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Close command audit", expanded: true })).toBeVisible(),
  );
  first.unmount();

  render(<App client={continuityClient} />);
  const restoredInspector = await screen.findByRole("region", {
    name: "session-ISS-01-1 execution inspector",
  });
  expect(executionTreeNode("session-ISS-01-1")).toHaveAttribute("aria-selected", "true");
  expect(screen.getByText(/Saved review diff/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Close command audit", expanded: true })).toBeVisible();
  closeCommandAudit();
  const restoredAssignmentDetail = screen.getByRole("region", { name: "Issue Assignment Detail" });
  expect(restoredAssignmentDetail).toHaveFocus();
  expect(restoredAssignmentDetail).toHaveTextContent("ISS-02");
  expect(restoredAssignmentDetail).toHaveTextContent("Keep selected issue visible");
  expect(screen.getByRole("table", { name: "Issue Assignment Board" })).toHaveTextContent(
    "Keep selected issue visible",
  );
  const promptStatus = screen.getByLabelText("Prompt status line");
  expect(promptStatus).toHaveTextContent("Controller qwen3.6-27b · qwen3.6:27b");
  expect(promptStatus).toHaveTextContent("Workspace albert");
  expect(screen.queryByText("Runtime /runtime/alfredo")).not.toBeInTheDocument();
  expectPromptScope("Restore workspace session");
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
            entity_id: "ADHOC-000001",
            queue_item_id: "delegation-command-deck-ISS-02",
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

  const inspector = await openExecutionInspector("ADHOC-000001", "ad-hoc-delegation");
  expect(await within(inspector).findByRole("button", { name: "Approve" })).toBeEnabled();
  expect(within(inspector).getByRole("button", { name: "Reject" })).toBeDisabled();
  expect(within(inspector).getByRole("button", { name: "Defer" })).toBeDisabled();

  fireEvent.click(within(inspector).getByRole("button", { name: "Approve" }));

  expect(within(inspector).getByRole("status")).toHaveTextContent(
    "Waiting for Orchestrator acknowledgement: Approve Approve ISS-02 delegation.",
  );
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
  expect(executionTreeNode("session-ADHOC-000001-1")).toBeVisible();
});

test("records canonical Queue visibility at R5 before the direct runner claim at R6", async () => {
  const queueItem: WorkspaceQueueItem = {
    item_id: "delegation-performance-001",
    mission_id: "command-deck",
    item_type: "ad-hoc-delegation",
    status: "pending",
    source: "agent-console",
    requested_action: "Approve measured delegation",
    affected_boundary: "launch-boundary",
    consequence: "Approval queues one exact Local Agent session.",
    issue_id: "ADHOC-PERF-001",
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
            attention_id: queueItem.item_id,
            mission_id: "command-deck",
            kind: "delegation-approval",
            label: "Measured delegation approval required",
            queue_link: `workspace-queue#${queueItem.item_id}`,
            entity_id: "ADHOC-PERF-001",
            queue_item_id: queueItem.item_id,
          },
        ],
      },
    ],
  };
  const queued: WorkspaceSnapshot = {
    ...before,
    revision: 5,
    missions: before.missions?.map((mission) => ({
      ...mission,
      attention: [],
      sessions: [
        {
          session_id: "session-ADHOC-PERF-001-1",
          issue_id: "ADHOC-PERF-001",
          assigned_agent: "fake-performance-worker-v1",
          status: "queued",
          last_activity_at: "",
          runner_started_at: "",
        },
      ],
    })),
  };
  const running: WorkspaceSnapshot = {
    ...queued,
    revision: 6,
    missions: queued.missions?.map((mission) => ({
      ...mission,
      sessions: mission.sessions.map((session) => ({
        ...session,
        status: "running",
        last_activity_at: "2026-07-30T20:00:05+00:00",
        runner_started_at: "2026-07-30T20:00:01+00:00",
      })),
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
  const marks: Array<{
    stage: string;
    boundary: string;
    detail: Readonly<Record<string, unknown>>;
  }> = [];
  let snapshotLoads = 0;
  let releaseRunner!: () => void;
  const runnerGate = new Promise<void>((resolve) => {
    releaseRunner = resolve;
  });
  render(
    <App
      client={{
        loadSnapshot: async () => {
          snapshotLoads += 1;
          return {
            kind: "ready",
            snapshot:
              snapshotLoads === 1 ? before : snapshotLoads === 2 ? queued : running,
          };
        },
        loadWorkspaceQueue: async () => ({
          kind: "workspace-queue",
          projection:
            snapshotLoads < 2
              ? queueProjection
              : { ...queueProjection, revision: 3, items: [], groups: [] },
        }),
        submitWorkspaceQueueDecision: async (request) => ({
          kind: "acknowledged",
          acknowledgement: {
            correlation_id: request.correlation_id,
            outcome: "acknowledged",
            revision: 3,
            item_id: request.item_id,
            item_status: "approved",
            effect_summary: "Measured delegation queued.",
            session_id: "session-ADHOC-PERF-001-1",
          },
        }),
        runWorkstationSession: async () => {
          await runnerGate;
          return {
            kind: "session-finished",
            session: {
              schema_version: 1,
              mission_id: "command-deck",
              session_id: "session-ADHOC-PERF-001-1",
              issue_id: "ADHOC-PERF-001",
              status: "running",
              runner_started_at: "2026-07-30T20:00:01+00:00",
              runner_ended_at: "",
              runner_exit_status: null,
              evidence_valid: false,
            },
          };
        },
        recordPerformanceMark: async (request) => {
          marks.push({
            stage: request.stage,
            boundary: request.boundary,
            detail: request.detail,
          });
          return { recorded: true };
        },
      }}
    />,
  );

  const inspector = await openExecutionInspector("ADHOC-PERF-001", "ad-hoc-delegation");
  fireEvent.click(await within(inspector).findByRole("button", { name: "Approve" }));
  await waitFor(() =>
    expect(
      marks.some((mark) => mark.stage === "R5" && mark.boundary === "end"),
    ).toBe(true),
  );
  expect(
    marks.some((mark) => mark.stage === "R6" && mark.boundary === "end"),
  ).toBe(false);
  expect(screen.getByText(/session-ADHOC-PERF-001-1 is queued and starting in the background/)).toBeVisible();

  releaseRunner();
  await waitFor(() =>
    expect(
      marks.some((mark) => mark.stage === "R6" && mark.boundary === "end"),
    ).toBe(true),
  );
  const r5End = marks.findIndex(
    (mark) => mark.stage === "R5" && mark.boundary === "end",
  );
  const r6End = marks.findIndex(
    (mark) => mark.stage === "R6" && mark.boundary === "end",
  );
  expect(r6End).toBeGreaterThan(r5End);
  expect(marks[r6End].detail).toMatchObject({
    session_id: "session-ADHOC-PERF-001-1",
    rendered_status: "running",
    runner_started_at: "2026-07-30T20:00:01+00:00",
  });
});

test("keeps assigned ready issue launch on the board instead of inventing a workstation card", async () => {
  const before: WorkspaceSnapshot = {
    ...snapshot,
    revision: 4,
    mission_board: {
      ...snapshot.mission_board,
      issue_slices: [
        {
          issue_id: "ISS-01",
          title: "Restore workspace session",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
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

  const tree = await screen.findByRole("region", { name: "Mission Execution Tree" });
  const board = screen.getByRole("table", { name: "Issue Assignment Board" });
  expect(within(tree).queryByRole("treeitem", { name: /ISS-01/ })).toBeVisible();
  expect(within(tree).queryByRole("button", { name: "Launch ISS-01" })).not.toBeInTheDocument();
  expect(within(board).getByRole("row", { name: /ISS-01 Restore workspace session/ })).toHaveTextContent(
    "qwen-coder-local",
  );
  fireEvent.click(within(board).getByRole("button", { name: "Launch ISS-01" }));

  await waitFor(() =>
    expect(actions).toEqual([
      {
        correlation_id: "workstation-issue-launch-command-deck-ISS-01-4",
        action_type: "issue-launch",
        actor: "mission-commander",
        expected_revision: 4,
        target: { kind: "issue-slice", id: "ISS-01" },
        mission_id: "command-deck",
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
  const readyIssueNode = within(executionTree()).getByRole("treeitem", {
    name: /issue-slice ISS-01/,
  });
  if (readyIssueNode.getAttribute("aria-expanded") !== "true") fireEvent.click(readyIssueNode);
  expect(executionTreeNode("session-ISS-01-1")).toBeVisible();
});

test("changes a ready issue model assignment from the board with required agent and reason", async () => {
  const assignmentSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    revision: 6,
    mission_board: {
      ...snapshot.mission_board,
      issue_slices: [
        {
          issue_id: "ISS-01",
          title: "Restore workspace session",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
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
        loadAgentCapabilities: async () => ({
          kind: "capabilities",
          catalog: {
            schema_version: 1,
            default_agent_id: "",
            commands: [],
            skills: [],
            agents: [
              {
                id: "qwen3.6-27b",
                role: "local-agent",
                provider: "ollama",
                runner: "ollama",
                model: "qwen3.6:27b",
                routing: "worker",
                availability: "available",
                availability_reason: "",
                assignable: true,
                delegate_only: false,
                requires_approval: false,
              },
            ],
          },
        }),
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

  const tree = await screen.findByRole("region", { name: "Mission Execution Tree" });
  const board = screen.getByRole("table", { name: "Issue Assignment Board" });
  expect(
    within(tree).queryByRole("button", { name: "Change model assignment ISS-01" }),
  ).not.toBeInTheDocument();
  const button = within(board).getByRole("button", {
    name: "Change model assignment ISS-01",
  });
  expect(button).toBeDisabled();
  fireEvent.change(within(board).getByRole("combobox", { name: "Issue assignment agent ISS-01" }), {
    target: { value: "qwen3.6-27b" },
  });
  fireEvent.change(within(board).getByRole("textbox", { name: "Issue assignment reason ISS-01" }), {
    target: { value: "Use the stronger local model." },
  });
  expect(button).toBeEnabled();
  fireEvent.click(button);

  await waitFor(() =>
    expect(actions).toEqual([
      {
        correlation_id: "workstation-model-assignment-change-command-deck-ISS-01-6",
        action_type: "model-assignment-change",
        actor: "mission-commander",
        expected_revision: 6,
        target: { kind: "issue-slice", id: "ISS-01" },
        mission_id: "command-deck",
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
  const loadSessionArtifact = vi.fn(
    async (request: SessionArtifactReadRequest): Promise<SessionArtifactReadResult> => {
      const isDiff = request.artifact_ref.endsWith("review.diff");
      return {
        kind: "session-artifact",
        artifact: {
          schema_version: 1,
          mission_id: request.mission_id,
          session_id: request.session_id,
          artifact_id: isDiff ? "review_diff" : "evidence-package",
          label: isDiff ? "Review diff" : "Evidence Package",
          media_type: isDiff ? "text/x-diff" : "application/json",
          content: isDiff
            ? "--- a/mission-control/src/App.tsx\n+++ b/mission-control/src/App.tsx\n+bounded viewer\n"
            : '{"evidence_valid": true}',
          byte_count: isDiff ? 92 : 24,
          content_limit_bytes: 128000,
          truncated: false,
        },
      };
    },
  );
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
            artifact_links: [
              "app-local://evidence/session-ISS-01-1",
              "app-local://missions/command-deck/sessions/session-ISS-01-1/artifacts/review_diff/review.diff",
            ],
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
        loadSessionArtifact,
      }}
    />,
  );

  const transcript = await screen.findByRole("region", { name: "Prompt Transcript" });
  const inspector = await openExecutionInspector("session-ISS-01-1");
  expect(within(inspector).getByText("Activity trail")).toBeVisible();
  expect(within(inspector).getAllByText("npm test -- App.test.tsx").length).toBeGreaterThan(0);
  expect(
    within(inspector).getByRole("button", {
      name: "Evidence Package session-ISS-01-1",
    }),
  ).toBeVisible();
  expect(within(inspector).getByText("Diff should be reviewed before acceptance.")).toBeVisible();

  fireEvent.click(within(inspector).getByRole("button", { name: "Diff mission-control/src/App.tsx" }));
  const diffViewer = await screen.findByRole("region", { name: "Session evidence viewer" });
  expect(diffViewer).toHaveTextContent("mission-control/src/App.tsx");
  expect(await within(diffViewer).findByLabelText("Review diff content")).toHaveTextContent(
    "+bounded viewer",
  );
  expect(diffViewer).not.toHaveTextContent("/runtime/sessions");
  expect(loadSessionArtifact).toHaveBeenLastCalledWith({
    mission_id: "command-deck",
    session_id: "session-ISS-01-1",
    artifact_ref:
      "app-local://missions/command-deck/sessions/session-ISS-01-1/artifacts/review_diff/review.diff",
  });
  fireEvent.click(
    within(inspector).getByRole("button", {
      name: "Evidence Package session-ISS-01-1",
    }),
  );
  expect(
    await screen.findByLabelText("Evidence Package content"),
  ).toHaveTextContent('"evidence_valid": true');

  expect(appendConsoleMessage).not.toHaveBeenCalled();
  expect(within(transcript).getByText("Keep this transcript clean.")).toBeVisible();
  expect(within(transcript).queryByText(/Selected session session-ISS-01-1/)).not.toBeInTheDocument();
  expect(within(transcript).queryByText(/mission-control\/src\/App.tsx/)).not.toBeInTheDocument();
});

test("shows bounded evidence loading, failure, retry, truncation, and inline content", async () => {
  const artifactRef = "app-local://evidence/session-ISS-EVIDENCE-1";
  const evidenceSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    active_mission: {
      id: "command-deck",
      title: "Command Deck Mission",
      issue_count: 1,
    },
    mission_board: {
      ...snapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-EVIDENCE"],
      ready_issue_ids: [],
      approved_issue_ids: [],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-EVIDENCE",
          title: "Inspect bounded evidence",
          lifecycle: "Approved",
          progress: "Evidence is ready for inspection.",
          sessions: [
            {
              session_id: "session-ISS-EVIDENCE-1",
              assigned_agent: "evidence-agent",
              role: "local-agent",
              provider: "ollama",
              model: "qwen2.5-coder:14b",
              status: "evidence-ready",
              stale: false,
              disconnected: false,
              operation_status: "evidence-ready",
              failure: "",
            },
          ],
          evidence: {
            state: "ready",
            changed_files: ["src/evidence.ts"],
            commands_run: ["npm test"],
            test_results: "Tests passed.",
            risks: "None.",
            artifact_links: [artifactRef],
          },
        }),
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
            session_id: "session-ISS-EVIDENCE-1",
            issue_id: "ISS-EVIDENCE",
            assigned_agent: "evidence-agent",
            status: "evidence-ready",
            role: "local-agent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
          },
        ],
        attention: [],
      },
    ],
  };
  let resolveFirst: ((result: SessionArtifactReadResult) => void) | null = null;
  const firstLoad = new Promise<SessionArtifactReadResult>((resolve) => {
    resolveFirst = resolve;
  });
  const loadSessionArtifact = vi
    .fn<(request: SessionArtifactReadRequest) => Promise<SessionArtifactReadResult>>()
    .mockImplementationOnce(async () => firstLoad)
    .mockResolvedValueOnce({
      kind: "session-artifact",
      artifact: {
        schema_version: 1,
        mission_id: "command-deck",
        session_id: "session-ISS-EVIDENCE-1",
        artifact_id: "evidence-package",
        label: "Evidence Package",
        media_type: "application/json",
        content: '{"evidence_valid": true}',
        byte_count: 24,
        content_limit_bytes: 128000,
        truncated: true,
      },
    });
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: evidenceSnapshot }),
        loadSessionArtifact,
      }}
    />,
  );
  const inspector = await openExecutionInspector("session-ISS-EVIDENCE-1");
  const evidenceTrigger = within(inspector).getByRole("button", {
    name: "Evidence Package session-ISS-EVIDENCE-1",
  });
  evidenceTrigger.focus();
  fireEvent.click(evidenceTrigger);

  const viewer = await screen.findByRole("region", { name: "Session evidence viewer" });
  expect(within(viewer).getByRole("status")).toHaveTextContent("Loading bounded session evidence");
  await act(async () => {
    resolveFirst?.({
      kind: "session-artifact-failure",
      code: "session-artifact-unavailable",
      message: "The registered evidence artifact could not be read.",
      recoverable: true,
    });
    await firstLoad;
  });
  expect(within(viewer).getByRole("alert")).toHaveTextContent("Evidence load failed");
  fireEvent.click(within(viewer).getByRole("button", { name: "Retry evidence" }));

  expect(await within(viewer).findByLabelText("Evidence Package content")).toHaveTextContent(
    '"evidence_valid": true',
  );
  expect(within(viewer).getByText(/Content is truncated at 128,000 bytes/)).toBeVisible();
  expect(loadSessionArtifact).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole("link", { name: /Evidence Package session-ISS-EVIDENCE-1/ })).not.toBeInTheDocument();
  fireEvent.click(within(viewer).getByRole("button", { name: "Close session evidence viewer" }));
  expect(screen.queryByRole("region", { name: "Session evidence viewer" })).not.toBeInTheDocument();
  await waitFor(() => expect(evidenceTrigger).toHaveFocus());
});

test("opens command audit without mixing prompt and terminal drafts", async () => {
  render(<App client={client} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Keep this console draft" },
  });
  await openCommandAudit();

  expect(screen.getByRole("region", { name: "Mission Execution Tree" })).toBeVisible();
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

test("returns from command audit to the console-first Mission Work layout", async () => {
  render(<App client={client} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  await openCommandAudit();
  expect(screen.getByRole("region", { name: "Shell Terminal" })).toBeVisible();
  expect(screen.queryByRole("region", { name: "Workstation Detail Views" })).not.toBeInTheDocument();

  closeCommandAudit();

  expect(screen.getByRole("region", { name: "Agent Console" })).toBeVisible();
  expect(screen.getByRole("complementary", { name: "Mission Work" })).toBeVisible();
  expect(screen.getByRole("region", { name: "Mission Execution Tree" })).toBeVisible();
  expect(screen.queryByRole("region", { name: "Workstation Cards" })).not.toBeInTheDocument();
  expect(screen.getByRole("table", { name: "Issue Assignment Board" })).toBeVisible();
  expect(screen.queryByRole("region", { name: "Shell Terminal" })).not.toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Workstation Detail Views" })).not.toBeInTheDocument();
  expect(screen.queryByRole("navigation", { name: "Workstation detail views" })).not.toBeInTheDocument();
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

test("keeps polling after an empty update batch and discovers out-of-band subagent status", async () => {
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
            session_id: "session-out-of-band",
            issue_id: "ISS-01",
            assigned_agent: "gemma4-12b",
            status: "running",
            runner_started_at: "2026-07-12T08:30:00+00:00",
            launch_correlation_id: "workstation-launch-out-of-band",
            role: "local-agent",
            provider: "ollama",
            model: "gemma4:12b",
          },
        ],
        attention: [],
      },
    ],
  };
  let loads = 0;
  render(
    <App
      syncIntervalMs={1}
      client={{
        loadSnapshot: async () => {
          loads += 1;
          return { kind: "ready", snapshot: loads >= 3 ? runningSnapshot : snapshot };
        },
        loadUpdates: async (afterRevision) => ({
          kind: "updates",
          batch: { after_revision: afterRevision, current_revision: afterRevision, events: [] },
        }),
      }}
    />,
  );

  expect(await screen.findByText("Execution Session running")).toBeVisible();
  expect(loads).toBeGreaterThanOrEqual(3);
  expect(screen.getByText(/Orchestrator started canonical session session-out-of-band for ISS-01 on gemma4-12b/)).toBeVisible();
});

test("quiet polling retries transient failures without replacing identical canonical state", async () => {
  const queueSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    operations_view: "workspace-queue",
  };
  let snapshotLoads = 0;
  let updateLoads = 0;
  let queueLoads = 0;
  render(
    <App
      syncIntervalMs={1}
      client={{
        loadSnapshot: async () => {
          snapshotLoads += 1;
          return { kind: "ready", snapshot: queueSnapshot };
        },
        loadUpdates: async (afterRevision) => {
          updateLoads += 1;
          if (updateLoads === 1) {
            return {
              kind: "sync-failure",
              code: "temporary-read-failure",
              message: "Runtime file is briefly unavailable.",
              recoverable: true,
            };
          }
          return {
            kind: "updates",
            batch: { after_revision: afterRevision, current_revision: afterRevision, events: [] },
          };
        },
        loadWorkspaceQueue: async () => {
          queueLoads += 1;
          return {
            kind: "workspace-queue",
            projection: { schema_version: 1, revision: 0, items: [], groups: [] },
          };
        },
      }}
    />,
  );

  await waitFor(() => expect(snapshotLoads).toBeGreaterThanOrEqual(3));
  expect(updateLoads).toBeGreaterThanOrEqual(3);
  expect(queueLoads).toBe(1);
  expect(screen.getByLabelText("Connection status")).toHaveTextContent("Connected");
});

test("resumes an acknowledged queued Local Agent after workstation restart", async () => {
  const queuedSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ISS-01-queued",
            issue_id: "ISS-01",
            assigned_agent: "qwen-coder-local",
            status: "queued",
            role: "local-agent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
          },
        ],
        attention: [],
      },
    ],
  };
  const runs: unknown[] = [];
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: queuedSnapshot }),
        runWorkstationSession: async (request) => {
          runs.push(request);
          return {
            kind: "session-finished",
            session: {
              schema_version: 1,
              mission_id: "command-deck",
              session_id: request.session_id,
              issue_id: "ISS-01",
              status: "evidence-ready",
              runner_started_at: "2026-07-10T08:00:00Z",
              runner_ended_at: "2026-07-10T08:00:01Z",
              runner_exit_status: 0,
              evidence_valid: true,
            },
          };
        },
      }}
    />,
  );

  await waitFor(() =>
    expect(runs).toEqual([
      { session_id: "session-ISS-01-queued", mission_id: "command-deck" },
    ]),
  );
  expect(await screen.findByText(/session-ISS-01-queued is queued and starting/)).toBeVisible();
  expect(await screen.findByText(/finished with status evidence-ready/)).toBeVisible();
});

test("retries a transient queued runner failure at most three times with Mission-qualified identity", async () => {
  const queuedSession = {
    session_id: "session-retry-queued",
    issue_id: "ISS-01",
    assigned_agent: "gemma4-12b",
    status: "queued",
    role: "local-agent",
    provider: "ollama",
    model: "gemma4:12b",
  };
  const queuedSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [queuedSession],
        attention: [],
      },
    ],
  };
  const finishedSnapshot: WorkspaceSnapshot = {
    ...queuedSnapshot,
    missions: queuedSnapshot.missions?.map((mission) => ({
      ...mission,
      sessions: mission.sessions.map((session) => ({ ...session, status: "evidence-ready" })),
    })),
  };
  const runs: Array<{ session_id: string; mission_id?: string }> = [];
  let finished = false;
  render(
    <App
      syncIntervalMs={10_000}
      client={{
        loadSnapshot: async () => ({
          kind: "ready",
          snapshot: finished ? finishedSnapshot : queuedSnapshot,
        }),
        runWorkstationSession: async (request) => {
          runs.push(request);
          if (runs.length < 3) {
            return {
              kind: "session-failed",
              code: "backend-startup-failure",
              message: `Transient runner bridge failure ${runs.length}`,
            };
          }
          finished = true;
          return {
            kind: "session-finished",
            session: {
              schema_version: 1,
              mission_id: "command-deck",
              session_id: request.session_id,
              issue_id: "ISS-01",
              status: "evidence-ready",
              runner_started_at: "2026-07-10T08:00:00Z",
              runner_ended_at: "2026-07-10T08:00:01Z",
              runner_exit_status: 0,
              evidence_valid: true,
            },
          };
        },
      }}
    />,
  );

  await waitFor(() => expect(runs).toHaveLength(3));
  expect(runs).toEqual([
    { session_id: "session-retry-queued", mission_id: "command-deck" },
    { session_id: "session-retry-queued", mission_id: "command-deck" },
    { session_id: "session-retry-queued", mission_id: "command-deck" },
  ]);
  expect(screen.getByText(/attempt 2 of 3/)).toBeVisible();
  expect(screen.getByText(/attempt 3 of 3/)).toBeVisible();
  expect(await screen.findByText(/finished with status evidence-ready/)).toBeVisible();
  await new Promise((resolve) => window.setTimeout(resolve, 225));
  expect(runs).toHaveLength(3);
});

test("does not retry a failed dispatch after canonical session state becomes terminal", async () => {
  const missionSession = {
    session_id: "session-terminal-after-failure",
    issue_id: "ISS-01",
    assigned_agent: "gemma4-12b",
    status: "queued",
    role: "local-agent",
    provider: "ollama",
    model: "gemma4:12b",
  };
  const withStatus = (status: string): WorkspaceSnapshot => ({
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [{ ...missionSession, status }],
        attention: [],
      },
    ],
  });
  let terminal = false;
  const runWorkstationSession = vi.fn(async () => {
    terminal = true;
    return {
      kind: "session-failed" as const,
      code: "runner-failed",
      message: "Runner persisted a terminal failure before the bridge disconnected.",
    };
  });
  render(
    <App
      syncIntervalMs={10_000}
      client={{
        loadSnapshot: async () => ({
          kind: "ready",
          snapshot: withStatus(terminal ? "failed" : "queued"),
        }),
        runWorkstationSession,
      }}
    />,
  );

  await waitFor(() => expect(runWorkstationSession).toHaveBeenCalledTimes(1));
  expect(await screen.findByText(/failed to run: Runner persisted a terminal failure/)).toBeVisible();
  await new Promise((resolve) => window.setTimeout(resolve, 125));
  expect(runWorkstationSession).toHaveBeenCalledTimes(1);
});

test("dispatches colliding mission-local queued session ids once per Mission", async () => {
  const sharedSessionId = "session-ISS-01-1";
  const queuedSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    missions: ["command-deck", "background-mission"].map((missionId, index) => ({
      id: missionId,
      title: index === 0 ? "Command Deck Mission" : "Background Mission",
      issue_count: 1,
      is_active: index === 0,
      sessions: [
        {
          session_id: sharedSessionId,
          issue_id: "ISS-01",
          assigned_agent: index === 0 ? "active-worker" : "background-worker",
          status: "queued",
          role: "local-agent",
          provider: "ollama",
          model: "qwen2.5-coder:14b",
        },
      ],
      attention: [],
    })),
  };
  const runs: Array<{ session_id: string; mission_id?: string }> = [];
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: queuedSnapshot }),
        runWorkstationSession: async (request) => {
          runs.push(request);
          return {
            kind: "session-finished",
            session: {
              schema_version: 1,
              mission_id: request.mission_id ?? "",
              session_id: request.session_id,
              issue_id: "ISS-01",
              status: "evidence-ready",
              runner_started_at: "2026-07-10T08:00:00Z",
              runner_ended_at: "2026-07-10T08:00:01Z",
              runner_exit_status: 0,
              evidence_valid: true,
            },
          };
        },
      }}
    />,
  );

  await waitFor(() => expect(runs).toHaveLength(2));
  expect(runs).toEqual([
    { session_id: sharedSessionId, mission_id: "command-deck" },
    { session_id: sharedSessionId, mission_id: "background-mission" },
  ]);
  expect(screen.getAllByText(/is queued and starting in the background/)).toHaveLength(2);
});

test("shows selected controller and model near the prompt composer", async () => {
  const loadLaunchContext = vi.fn(async () => ({
    kind: "launch-context" as const,
    context: {
      schema_version: 1 as const,
      selected_agent: "qwen3.6-27b",
      selected_model: "qwen3.6:27b",
      starting_location: "/workspace",
      coding_workspace: "/workspace/albert",
      active_mission: "command-deck",
      phase: "workspace-ready" as const,
      runtime_root: "/home/mission/.alfredo/runtime",
      recent_workspaces: ["/workspace/albert", "/workspace/other project's copy"],
    },
  }));

  render(<App client={{ ...client, loadLaunchContext }} />);

  await screen.findByRole("heading", { name: "Command Deck Mission" });
  const launchContext = await screen.findByLabelText("Prompt status line");
  expect(launchContext).toHaveTextContent("Controller qwen3.6-27b · qwen3.6:27b");
  expect(within(launchContext).queryByText("Model qwen3.6:27b")).not.toBeInTheDocument();
  const recentWorkspace = screen.getByRole("combobox", { name: "Workspace to relaunch" });
  expect(recentWorkspace).toHaveValue("/workspace/albert");
  expect(within(recentWorkspace).getAllByRole("option")).toHaveLength(2);

  fireEvent.change(recentWorkspace, { target: { value: "/workspace/other project's copy" } });

  expect(screen.getByRole("textbox", { name: "Workspace relaunch command" })).toHaveValue(
    `cd -- '/workspace/other project'"'"'s copy' && alfredo workstation --agent 'qwen3.6-27b'`,
  );
  expect(screen.getByRole("button", { name: "Copy workspace relaunch command" })).toBeEnabled();
  expect(within(launchContext).getByText("Workspace albert")).toBeVisible();
});

test("does not report a stale workspace relaunch copy after the selection changes", async () => {
  let resolveCopy!: () => void;
  const copyPending = new Promise<void>((resolve) => {
    resolveCopy = resolve;
  });
  const writeText = vi.fn(() => copyPending);
  const previousClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });

  try {
    render(
      <App
        client={{
          ...client,
          loadLaunchContext: async () => ({
            kind: "launch-context",
            context: {
              schema_version: 1,
              selected_agent: "qwen3.6-27b",
              selected_model: "qwen3.6:27b",
              starting_location: "/workspace",
              coding_workspace: "/workspace/albert",
              active_mission: "command-deck",
              phase: "workspace-ready",
              runtime_root: "/home/mission/.alfredo/runtime",
              recent_workspaces: ["/workspace/albert", "/workspace/next"],
            },
          }),
        }}
      />,
    );

    await screen.findByRole("heading", { name: "Command Deck Mission" });
    const workspace = screen.getByRole("combobox", { name: "Workspace to relaunch" });
    fireEvent.click(screen.getByRole("button", { name: "Copy workspace relaunch command" }));
    expect(writeText).toHaveBeenCalledWith(
      "cd -- '/workspace/albert' && alfredo workstation --agent 'qwen3.6-27b'",
    );

    fireEvent.change(workspace, { target: { value: "/workspace/next" } });
    await act(async () => {
      resolveCopy();
      await copyPending;
    });

    expect(screen.queryByRole("status", { name: "Workspace relaunch status" })).not.toBeInTheDocument();
  } finally {
    if (previousClipboard) {
      Object.defineProperty(navigator, "clipboard", previousClipboard);
    } else {
      Reflect.deleteProperty(navigator, "clipboard");
    }
  }
});

test.each([
  ["worker id", "gemma4-12b", "gemma4:12b", "qwen3-14b"],
  ["controller model alias", "qwen3.6:27b", "qwen3.6:27b", "qwen3.6-27b"],
])(
  "normalizes an invalid or aliased launch-context %s before controller discussion",
  async (_caseName, launchedAgent, launchedModel, expectedControllerId) => {
    const generateConsoleResponse = vi.fn(async (request) => ({
      kind: "message" as const,
      message: {
        message_id: "console-controller-normalized-2",
        sequence: 2,
        role: "assistant" as const,
        content: "Controller discussion is available.",
        scope: snapshot.conversation_scope,
        outcome: "model-commentary" as const,
        source: "frontier-model" as const,
      },
      route: { intent: "discussion" as const, task_request: "", acceptance_criteria: [] },
    }));
    const normalizedClient: WorkspaceClient = {
      loadSnapshot: async () => ({ kind: "ready", snapshot }),
      loadLaunchContext: async () => ({
        kind: "launch-context",
        context: {
          schema_version: 1,
          selected_agent: launchedAgent,
          selected_model: launchedModel,
          starting_location: "/workspace",
          coding_workspace: "/workspace/albert",
          active_mission: "command-deck",
          phase: "workspace-ready",
          runtime_root: "/tmp/albert-runtime",
          recent_workspaces: [],
        },
      }),
      loadAgentCapabilities: async () => ({
        kind: "capabilities",
        catalog: {
          schema_version: 1,
          default_agent_id: "qwen3-14b",
          commands: [],
          skills: [],
          agents: [
            {
              id: "qwen3-14b",
              role: "frontier",
              provider: "ollama",
              runner: "ollama",
              model: "qwen3:14b",
              routing: "controller",
              availability: "available",
              availability_reason: "",
              assignable: false,
              delegate_only: false,
              requires_approval: false,
            },
            {
              id: "qwen3.6-27b",
              role: "frontier",
              provider: "ollama",
              runner: "ollama",
              model: "qwen3.6:27b",
              routing: "router",
              availability: "available",
              availability_reason: "",
              assignable: false,
              delegate_only: false,
              requires_approval: false,
            },
            {
              id: "gemma4-12b",
              role: "local-agent",
              provider: "ollama",
              runner: "ollama",
              model: "gemma4:12b",
              routing: "worker",
              availability: "available",
              availability_reason: "",
              assignable: true,
              delegate_only: false,
              requires_approval: false,
            },
          ],
        },
      }),
      appendConsoleMessage: async (request) => ({
        kind: "message",
        message: {
          message_id: "console-controller-normalized-1",
          sequence: 1,
          role: "user",
          content: request.content,
          scope: snapshot.conversation_scope,
          outcome: "proposed",
          source: "mission-commander",
        },
      }),
      generateConsoleResponse,
    };

    render(<App client={normalizedClient} />);
    await screen.findByRole("heading", { name: "Command Deck Mission" });
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "Controller model" })).toHaveValue(
        expectedControllerId,
      ),
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
      target: { value: "How does controller selection work?" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

    await waitFor(() => expect(generateConsoleResponse).toHaveBeenCalledTimes(1));
    expect(generateConsoleResponse.mock.calls[0][0]).toMatchObject({
      agent_id: expectedControllerId,
    });
  },
);

test("discovers controllers, slash commands, and installed skills beside the composer", async () => {
  render(
    <App
      client={{
        ...client,
        loadAgentCapabilities: async () => ({
          kind: "capabilities",
          catalog: {
            schema_version: 1,
            default_agent_id: "qwen3-14b",
            commands: [
              {
                name: "/run",
                usage: "/run <command>",
                description: "Run a governed shell command.",
                category: "execution",
              },
              {
                name: "/status",
                usage: "/status",
                description: "Show current execution status.",
                category: "monitoring",
              },
            ],
            skills: [
              {
                name: "diagnose",
                description: "Reproduce and isolate hard bugs.",
                source: "/workspace/.agents/skills/diagnose/SKILL.md",
                invocation: "/use diagnose",
              },
            ],
            agents: [
              {
                id: "cloud-controller",
                role: "frontier",
                provider: "ollama",
                runner: "ollama",
                model: "qwen-cloud:cloud",
                routing: "controller",
                availability: "available",
                availability_reason: "",
                assignable: false,
                delegate_only: false,
                requires_approval: false,
              },
              {
                id: "qwen3-14b",
                role: "frontier",
                provider: "ollama",
                runner: "ollama",
                model: "qwen3:14b",
                routing: "controller",
                availability: "available",
                availability_reason: "",
                assignable: false,
                delegate_only: false,
                requires_approval: false,
              },
              {
                id: "qwen3.6-27b",
                role: "frontier",
                provider: "ollama",
                runner: "ollama",
                model: "qwen3.6:27b",
                routing: "router",
                availability: "available",
                availability_reason: "",
                assignable: false,
                delegate_only: false,
                requires_approval: false,
              },
            ],
          },
        }),
      }}
    />,
  );

  const controller = await screen.findByRole("combobox", { name: "Controller model" });
  expect(controller).toHaveValue("qwen3-14b");
  expect(within(controller).queryByRole("option", { name: /cloud-controller/ })).not.toBeInTheDocument();
  fireEvent.change(controller, { target: { value: "qwen3.6-27b" } });
  expect(controller).toHaveValue("qwen3.6-27b");

  fireEvent.click(screen.getByRole("button", { name: "Browse commands and skills" }));
  const menu = screen.getByRole("region", { name: "Commands and skills" });
  expect(within(menu).getByText("/run <command>")).toBeVisible();
  expect(within(menu).getByText("$diagnose")).toBeVisible();
  fireEvent.click(within(menu).getByText("$diagnose"));
  const composer = screen.getByRole("textbox", { name: "Message Alfredo" });
  expect(composer).toHaveValue("/use diagnose ");

  fireEvent.change(composer, { target: { value: "/" } });
  expect(screen.getByRole("region", { name: "Commands and skills" })).toBeVisible();
});

test("surfaces and retries initial capability and console history load failures", async () => {
  let capabilityLoads = 0;
  let historyLoads = 0;
  render(
    <App
      client={{
        ...client,
        loadAgentCapabilities: async () => {
          capabilityLoads += 1;
          return capabilityLoads === 1
            ? {
                kind: "capabilities-failure",
                code: "catalog-unavailable",
                message: "Capability catalog is temporarily unavailable.",
                recoverable: true,
              }
            : {
                kind: "capabilities",
                catalog: {
                  schema_version: 1,
                  default_agent_id: "qwen3-14b",
                  commands: [],
                  skills: [],
                  agents: [],
                },
              };
        },
        loadConsoleHistory: async () => {
          historyLoads += 1;
          return historyLoads === 1
            ? {
                kind: "history-failure",
                code: "history-locked",
                message: "Console history is temporarily locked.",
                recoverable: true,
              }
            : {
                kind: "history",
                history: {
                  schema_version: 1,
                  messages: [
                    {
                      message_id: "console-restored-after-retry",
                      sequence: 1,
                      role: "assistant",
                      content: "Console history restored after retry.",
                      scope: snapshot.conversation_scope,
                      outcome: "model-commentary",
                      source: "frontier-model",
                    },
                  ],
                },
              };
        },
      }}
    />,
  );
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  expect(await screen.findByText(/Capability catalog load failed: Capability catalog is temporarily unavailable/)).toBeVisible();
  expect(screen.getByText(/Console history load failed: Console history is temporarily locked/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Retry capability catalog" }));
  fireEvent.click(screen.getByRole("button", { name: "Retry console history" }));

  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Browse commands and skills" })).toBeEnabled(),
  );
  expect(await screen.findByText("Console history restored after retry.")).toBeVisible();
  expect(capabilityLoads).toBe(2);
  expect(historyLoads).toBe(2);
});

test("manages keyboard focus when the commands and skills palette opens and closes", async () => {
  render(
    <App
      client={{
        ...client,
        loadAgentCapabilities: async () => ({
          kind: "capabilities",
          catalog: {
            schema_version: 1,
            default_agent_id: "",
            commands: [
              {
                name: "/run",
                usage: "/run <command>",
                description: "Run a governed shell command.",
                category: "execution",
              },
            ],
            skills: [
              {
                name: "diagnosing-bugs",
                description: "Reproduce and isolate hard bugs.",
                source: "/skills/diagnosing-bugs/SKILL.md",
                invocation: "/use diagnosing-bugs",
              },
            ],
            agents: [],
          },
        }),
      }}
    />,
  );
  const trigger = await screen.findByRole("button", { name: "Browse commands and skills" });
  await waitFor(() => expect(trigger).toBeEnabled());
  fireEvent.click(trigger);

  const menu = screen.getByRole("region", { name: "Commands and skills" });
  const firstOption = within(menu).getByRole("button", { name: /\/run <command>/ });
  const skillOption = within(menu).getByRole("button", { name: /\$diagnosing-bugs/ });
  const close = within(menu).getByRole("button", { name: "Close commands and skills" });
  await waitFor(() => expect(firstOption).toHaveFocus());
  expect(firstOption.tabIndex).toBe(0);
  expect(skillOption.tabIndex).toBe(0);
  expect(close.tabIndex).toBe(0);

  fireEvent.keyDown(firstOption, { key: "Escape" });
  expect(screen.queryByRole("region", { name: "Commands and skills" })).not.toBeInTheDocument();
  expect(trigger).toHaveFocus();

  fireEvent.click(trigger);
  await waitFor(() => expect(within(screen.getByRole("region", { name: "Commands and skills" })).getByRole(
    "button",
    { name: /\/run <command>/ },
  )).toHaveFocus());
  fireEvent.click(screen.getByRole("button", { name: "Close commands and skills" }));
  expect(trigger).toHaveFocus();

  fireEvent.click(trigger);
  const reopened = screen.getByRole("region", { name: "Commands and skills" });
  fireEvent.click(within(reopened).getByRole("button", { name: /\$diagnosing-bugs/ }));
  expect(screen.getByRole("textbox", { name: "Message Alfredo" })).toHaveFocus();
});

test("traverses submitted prompt history and restores the unsent draft", async () => {
  let sequence = 0;
  const appendConsoleMessage = vi.fn(async (request: AgentConsoleMessageRequest) => {
    sequence += 1;
    return {
      kind: "message" as const,
      message: {
        message_id: `console-history-${sequence}`,
        sequence,
        role: "user" as const,
        content: request.content,
        scope: snapshot.conversation_scope,
        outcome: "proposed" as const,
        source: "mission-commander",
      },
    };
  });
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        appendConsoleMessage,
      }}
    />,
  );

  const composer = await screen.findByRole("textbox", { name: "Message Alfredo" });
  for (const [index, prompt] of ["first prompt", "second prompt"].entries()) {
    fireEvent.change(composer, { target: { value: prompt } });
    fireEvent.keyDown(composer, { key: "Enter", code: "Enter" });
    await waitFor(() => expect(appendConsoleMessage).toHaveBeenCalledTimes(index + 1));
  }

  fireEvent.change(composer, { target: { value: "unsent draft" } });
  fireEvent.keyDown(composer, { key: "ArrowUp" });
  expect(composer).toHaveValue("second prompt");
  fireEvent.keyDown(composer, { key: "ArrowUp" });
  expect(composer).toHaveValue("first prompt");
  fireEvent.keyDown(composer, { key: "ArrowDown" });
  expect(composer).toHaveValue("second prompt");
  fireEvent.keyDown(composer, { key: "ArrowDown" });
  expect(composer).toHaveValue("unsent draft");

  fireEvent.keyDown(composer, { key: "ArrowUp" });
  expect(composer).toHaveValue("second prompt");
  fireEvent.keyDown(composer, { key: "Enter" });
  await waitFor(() => expect(appendConsoleMessage).toHaveBeenCalledTimes(3));
  fireEvent.keyDown(composer, { key: "ArrowDown" });
  expect(composer).toHaveValue("");
});

test("continues through multiline prompt history and preserves a multiline unsent draft", async () => {
  let sequence = 0;
  const appendConsoleMessage = vi.fn(async (request: AgentConsoleMessageRequest) => {
    sequence += 1;
    return {
      kind: "message" as const,
      message: {
        message_id: `console-multiline-history-${sequence}`,
        sequence,
        role: "user" as const,
        content: request.content,
        scope: snapshot.conversation_scope,
        outcome: "proposed" as const,
        source: "mission-commander",
      },
    };
  });
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        appendConsoleMessage,
      }}
    />,
  );

  const composer =
    (await screen.findByRole("textbox", { name: "Message Alfredo" })) as HTMLTextAreaElement;
  for (const [index, prompt] of [
    "first prompt\nwith details",
    "second prompt\nwith details",
  ].entries()) {
    fireEvent.change(composer, { target: { value: prompt } });
    fireEvent.keyDown(composer, { key: "Enter", code: "Enter" });
    await waitFor(() => expect(appendConsoleMessage).toHaveBeenCalledTimes(index + 1));
  }

  const unsentDraft = "unsent draft\nwith more details";
  fireEvent.change(composer, { target: { value: unsentDraft } });
  composer.setSelectionRange(unsentDraft.indexOf("\n") + 1, unsentDraft.indexOf("\n") + 1);
  const ordinaryArrowUp = createEvent.keyDown(composer, { key: "ArrowUp", code: "ArrowUp" });
  fireEvent(composer, ordinaryArrowUp);
  expect(ordinaryArrowUp.defaultPrevented).toBe(false);

  composer.setSelectionRange(0, 0);
  fireEvent.keyDown(composer, { key: "ArrowUp", code: "ArrowUp" });
  expect(composer).toHaveValue("second prompt\nwith details");
  fireEvent.keyDown(composer, { key: "ArrowUp", code: "ArrowUp" });
  expect(composer).toHaveValue("first prompt\nwith details");
  fireEvent.keyDown(composer, { key: "ArrowDown", code: "ArrowDown" });
  expect(composer).toHaveValue("second prompt\nwith details");
  fireEvent.keyDown(composer, { key: "ArrowDown", code: "ArrowDown" });
  expect(composer).toHaveValue(unsentDraft);
});

test("supports keyboard slash and at-capability completion with predictable dismissal", async () => {
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadAgentCapabilities: async () => ({
          kind: "capabilities",
          catalog: {
            schema_version: 1,
            default_agent_id: "",
            commands: [
              {
                name: "/status",
                usage: "/status",
                description: "Show current execution status.",
                category: "monitoring",
              },
            ],
            skills: [],
            agents: [],
          },
        }),
      }}
    />,
  );

  const composer = await screen.findByRole("textbox", { name: "Message Alfredo" });
  fireEvent.change(composer, { target: { value: "/st" } });
  expect(screen.getByRole("option", { name: /\/status/ })).toBeVisible();
  fireEvent.keyDown(composer, { key: "Tab" });
  expect(composer).toHaveValue("/status ");
  expect(composer).toHaveFocus();

  fireEvent.change(composer, { target: { value: "Ask @way" } });
  expect(screen.getByRole("option", { name: /@wayfinder/ })).toBeVisible();
  fireEvent.keyDown(composer, { key: "Escape" });
  expect(screen.queryByRole("option", { name: /@wayfinder/ })).not.toBeInTheDocument();
  expect(composer).not.toHaveAttribute("aria-controls");
  expect(composer).not.toHaveAttribute("aria-activedescendant");
  expect(composer).toHaveFocus();

  fireEvent.change(composer, { target: { value: "Ask @wayf" } });
  fireEvent.change(composer, { target: { value: "Ask @way" } });
  fireEvent.keyDown(composer, { key: "Tab" });
  expect(composer).toHaveValue("Ask @wayfinder ");
  expect(composer).toHaveFocus();
});

test("labels Coding Workspace, Wayfinder, Orchestrator, and Mission outcomes", async () => {
  const launchContext = {
    schema_version: 1 as const,
    selected_agent: "qwen3-14b",
    selected_model: "qwen3:14b",
    starting_location: "/workspace",
    coding_workspace: null,
    active_mission: null,
    phase: "selection-required" as const,
    runtime_root: "/tmp/alfredo-runtime",
    recent_workspaces: [],
  };
  const selectionGate = render(
    <App
      client={{
        loadLaunchContext: async () => ({ kind: "launch-context", context: launchContext }),
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
      }}
    />,
  );
  expect(await screen.findByText("Capability: Coding Workspace")).toBeVisible();
  selectionGate.unmount();

  const missionChoiceContext = {
    ...launchContext,
    coding_workspace: "/workspace/repository",
    phase: "mission-choice-required" as const,
    revision: 1,
    known_missions: [{ id: "existing-mission", title: "Existing Mission" }],
  };
  const missionGate = render(
    <App
      client={{
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: missionChoiceContext,
        }),
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
      }}
    />,
  );
  expect(await screen.findByText("Capability: Mission")).toBeVisible();
  missionGate.unmount();

  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadConsoleHistory: async () => ({
          kind: "history",
          history: {
            schema_version: 1,
            messages: [
              {
                message_id: "console-wayfinder-capability",
                sequence: 1,
                role: "assistant",
                content: "Wayfinder held the gate open.",
                scope: snapshot.conversation_scope,
                outcome: "acknowledged",
                source: "wayfinder-agent",
                correlation_id: "wayfinder-receipt",
                action_phase: "shared-understanding-agent-acknowledged",
              },
              {
                message_id: "console-orchestrator-capability",
                sequence: 2,
                role: "assistant",
                content: "Orchestrator recorded the Mission outcome.",
                scope: snapshot.conversation_scope,
                outcome: "acknowledged",
                source: "orchestrator",
                correlation_id: "orchestrator-receipt",
                action_phase: "mission-formed",
              },
            ],
          },
        }),
      }}
    />,
  );
  expect(await screen.findByText("Capability: Wayfinder")).toBeVisible();
  expect(screen.getByText("Capability: Orchestrator")).toBeVisible();
  expect(screen.getByText("Capability: Mission")).toBeVisible();
});

test("completes the Coding Workspace to Wayfinder journey by keyboard at constrained reduced motion", async () => {
  const selectionContext = {
    schema_version: 1 as const,
    selected_agent: "qwen3-14b",
    selected_model: "qwen3:14b",
    starting_location: "/workspace",
    coding_workspace: null,
    active_mission: null,
    phase: "selection-required" as const,
    runtime_root: "/tmp/alfredo-runtime",
    recent_workspaces: [],
  };
  const codingWorkspace = "/workspace/keyboard-journey";
  const mission = { id: "keyboard-journey", title: "Keyboard Journey" };
  const journeyClient: WorkspaceClient = {
    loadLaunchContext: async () => ({ kind: "launch-context", context: selectionContext }),
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadConsoleHistory: async () => ({
      kind: "history",
      history: { schema_version: 1, messages: [] },
    }),
    selectCodingWorkspace: async (request) => ({
      kind: "acknowledged",
      acknowledgement: {
        schema_version: 1,
        correlation_id: request.correlation_id,
        outcome: "acknowledged",
        starting_location: selectionContext.starting_location,
        coding_workspace: codingWorkspace,
        selection_mode: request.selection_mode,
        active_mission: null,
        replayed: false,
        known_missions: [mission],
        message: "Coding Workspace acknowledged by the Orchestrator; no Mission has been selected.",
      },
    }),
    chooseMission: async (request) => ({
      kind: "acknowledged",
      acknowledgement: {
        schema_version: 1,
        correlation_id: request.correlation_id,
        outcome: "acknowledged",
        coding_workspace: codingWorkspace,
        choice: request.choice,
        active_mission: request.mission_id,
        revision: 2,
        replayed: false,
        missions: [mission],
        message: "Keyboard Journey started.",
      },
    }),
    appendConsoleMessage: async (request) => ({
      kind: "message",
      message: {
        message_id: "console-keyboard-journey-user",
        sequence: 1,
        role: request.role,
        content: request.content,
        scope: snapshot.conversation_scope,
        outcome: request.outcome,
        source: request.source,
      },
    }),
    generateConsoleResponse: async () => ({
      kind: "message",
      message: {
        message_id: "console-keyboard-journey-wayfinder",
        sequence: 2,
        role: "assistant",
        content: "Wayfinder Chart mode is active.",
        scope: snapshot.conversation_scope,
        outcome: "acknowledged",
        source: "wayfinder-agent",
      },
      route: { intent: "discussion", task_request: "", acceptance_criteria: [] },
      wayfinder: {
        mode: "chart",
        gate: { status: "pending", opened_by: "", receipt_id: "" },
        flow: {
          flow_id: "keyboard-journey-flow",
          mode: "chart",
          originating_message_id: "console-keyboard-journey-user",
          scope: snapshot.conversation_scope,
          reference: "",
        },
        continuing: false,
        turn_complete: true,
      },
    }),
  };

  const originalWidth = window.innerWidth;
  const originalMatchMedia = window.matchMedia;
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 640 });
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      matches: query === "(prefers-reduced-motion: reduce)",
      media: query,
      onchange: null,
      addListener: () => undefined,
      removeListener: () => undefined,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      dispatchEvent: () => false,
    }),
  });
  fireEvent(window, new Event("resize"));

  const activateFocusedButton = async (button: HTMLElement): Promise<void> => {
    button.focus();
    expect(button).toHaveFocus();
    await act(async () => {
      fireEvent.keyDown(button, { key: "Enter", code: "Enter" });
      // jsdom does not synthesize a native button click from Enter; this is
      // the browser's default activation for the already-focused control.
      button.click();
    });
  };

  try {
    render(<App client={journeyClient} syncIntervalMs={100_000} />);
    const workspacePath = await screen.findByRole("textbox", { name: "Coding Workspace path" });
    expect(workspacePath).toHaveAccessibleName("Coding Workspace path");
    fireEvent.change(workspacePath, { target: { value: codingWorkspace } });
    const createRepository = screen.getByRole("button", { name: "Create new repository" });
    expect(createRepository).toHaveAccessibleName("Create new repository");
    await activateFocusedButton(createRepository);
    expect(await screen.findByText("Mission selection required")).toBeVisible();
    expect(screen.getByText("Capability: Mission")).toBeVisible();

    const missionTitle = screen.getByRole("textbox", { name: "New Mission title" });
    expect(missionTitle).toHaveAccessibleName("New Mission title");
    fireEvent.change(missionTitle, { target: { value: mission.title } });
    const startMission = screen.getByRole("button", { name: "Start New Mission" });
    await activateFocusedButton(startMission);

    expect(await screen.findByRole("heading", { name: "Command Deck Mission" })).toBeVisible();
    const composer = screen.getByRole("textbox", { name: "Message Alfredo" });
    expect(composer).toHaveAccessibleName("Message Alfredo");
    composer.focus();
    expect(composer).toHaveFocus();
    fireEvent.change(composer, { target: { value: "Start a new project" } });
    fireEvent.keyDown(composer, { key: "Enter", code: "Enter" });
    expect(await screen.findByRole("status", { name: "Wayfinder route" })).toHaveTextContent(
      "Chart mode",
    );
    const wayfinderRoute = screen.getByRole("status", { name: "Wayfinder route" });
    expect(within(wayfinderRoute).getByText("Capability: Wayfinder")).toBeVisible();
    expect(screen.getAllByText("Capability: Mission").length).toBeGreaterThan(0);
    expect(window.innerWidth).toBeLessThan(680);
    expect(window.matchMedia("(prefers-reduced-motion: reduce)").matches).toBe(true);
    const reducedMotionRules = stylesSource.slice(
      stylesSource.indexOf("@media (prefers-reduced-motion: reduce)"),
    );
    expect(reducedMotionRules).toMatch(/animation:\s*none\s*!important/);
    expect(reducedMotionRules).toMatch(/transition:\s*none\s*!important/);
  } finally {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: originalWidth });
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
    fireEvent(window, new Event("resize"));
  }
});

test("keeps commands and context mutually exclusive above the composer", async () => {
  render(
    <App
      client={{
        ...client,
        loadAgentCapabilities: async () => ({
          kind: "capabilities",
          catalog: {
            schema_version: 1,
            default_agent_id: "",
            commands: [
              {
                name: "/status",
                usage: "/status",
                description: "Show current execution status.",
                category: "monitoring",
              },
            ],
            skills: [],
            agents: [],
          },
        }),
      }}
    />,
  );

  const commandsButton = await screen.findByRole("button", {
    name: "Browse commands and skills",
  });
  fireEvent.click(commandsButton);
  expect(screen.getByRole("region", { name: "Commands and skills" })).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "Inspect context" }));
  expect(screen.getByRole("region", { name: "Context Inspector" })).toBeVisible();
  expect(screen.queryByRole("region", { name: "Commands and skills" })).not.toBeInTheDocument();

  fireEvent.click(commandsButton);
  expect(screen.getByRole("region", { name: "Commands and skills" })).toBeVisible();
  expect(screen.queryByRole("region", { name: "Context Inspector" })).not.toBeInTheDocument();
});

test("waits for delayed launch context, then honors and persists controller selection", async () => {
  let resolveLaunchContext!: (result: {
    kind: "launch-context";
    context: {
      schema_version: 1;
      selected_agent: string;
      selected_model: string;
      starting_location: string;
      coding_workspace: string;
      active_mission: string;
      phase: "workspace-ready";
      runtime_root: string;
      recent_workspaces: string[];
    };
  }) => void;
  const delayedLaunchContext = new Promise<Parameters<typeof resolveLaunchContext>[0]>((resolve) => {
    resolveLaunchContext = resolve;
  });
  const controllerCatalog = {
    schema_version: 1 as const,
    default_agent_id: "qwen3-14b",
    commands: [],
    skills: [],
    agents: [
      {
        id: "qwen3-14b",
        role: "frontier",
        provider: "ollama",
        runner: "ollama",
        model: "qwen3:14b",
        routing: "controller",
        availability: "available",
        availability_reason: "",
        assignable: false,
        delegate_only: false,
        requires_approval: false,
      },
      {
        id: "qwen3.6-27b",
        role: "frontier",
        provider: "ollama",
        runner: "ollama",
        model: "qwen3.6:27b",
        routing: "router",
        availability: "available",
        availability_reason: "",
        assignable: false,
        delegate_only: false,
        requires_approval: false,
      },
    ],
  };
  const launchAwareClient: WorkspaceClient = {
    ...client,
    loadAgentCapabilities: async () => ({ kind: "capabilities", catalog: controllerCatalog }),
    loadLaunchContext: async () => delayedLaunchContext,
  };

  const firstMount = render(<App client={launchAwareClient} />);
  expect(screen.getByRole("status")).toHaveTextContent("Connecting to Alfredo");
  await act(async () => {
    resolveLaunchContext({
      kind: "launch-context",
      context: {
        schema_version: 1,
        selected_agent: "qwen3.6-27b",
        selected_model: "qwen3.6:27b",
        starting_location: "/workspace",
        coding_workspace: "/workspace/albert",
        active_mission: "command-deck",
        phase: "workspace-ready",
        runtime_root: "/home/mission/.alfredo/runtime",
        recent_workspaces: ["/workspace/albert"],
      },
    });
  });
  const controller = await screen.findByRole("combobox", { name: "Controller model" });
  expect(controller).toHaveValue("qwen3.6-27b");

  fireEvent.change(controller, { target: { value: "qwen3-14b" } });
  expect(controller).toHaveValue("qwen3-14b");
  firstMount.unmount();

  render(<App client={launchAwareClient} />);
  const restoredController = await screen.findByRole("combobox", { name: "Controller model" });
  await waitFor(() => expect(restoredController).toHaveValue("qwen3-14b"));
});

test("uses launch controller authority while the capability catalog is delayed", async () => {
  let resolveCapabilities!: (
    result: Awaited<ReturnType<NonNullable<WorkspaceClient["loadAgentCapabilities"]>>>,
  ) => void;
  let resolveLaunchContext!: (
    result: Awaited<ReturnType<NonNullable<WorkspaceClient["loadLaunchContext"]>>>,
  ) => void;
  const delayedCapabilities = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["loadAgentCapabilities"]>>>
  >((resolve) => {
    resolveCapabilities = resolve;
  });
  const delayedLaunchContext = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["loadLaunchContext"]>>>
  >((resolve) => {
    resolveLaunchContext = resolve;
  });
  const generateConsoleResponse = vi.fn(async (
    _request: Parameters<NonNullable<WorkspaceClient["generateConsoleResponse"]>>[0],
  ) => ({
    kind: "message" as const,
    message: {
      message_id: "console-first-discussion-000002",
      sequence: 2,
      role: "assistant" as const,
      content: "The launch-selected controller handled the first discussion.",
      scope: snapshot.conversation_scope,
      outcome: "model-commentary" as const,
      source: "frontier-model" as const,
    },
    route: { intent: "discussion" as const, task_request: "", acceptance_criteria: [] },
  }));
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadAgentCapabilities: async () => delayedCapabilities,
        loadLaunchContext: async () => delayedLaunchContext,
        appendConsoleMessage: async (request) => ({
          kind: "message",
          message: {
            message_id: "console-first-discussion-000001",
            sequence: 1,
            role: "user",
            content: request.content,
            scope: snapshot.conversation_scope,
            outcome: "proposed",
            source: "mission-commander",
          },
        }),
        generateConsoleResponse,
      }}
    />,
  );
  await act(async () => {
    resolveLaunchContext({
      kind: "launch-context",
      context: {
        schema_version: 1,
        selected_agent: "qwen3.6-27b",
        selected_model: "qwen3.6:27b",
        starting_location: "/workspace",
        coding_workspace: "/workspace/albert",
        active_mission: "command-deck",
        phase: "workspace-ready",
        runtime_root: "/home/mission/.alfredo/runtime",
        recent_workspaces: [],
      },
    });
  });
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "How should we structure the next project slice?" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
  expect(
    await screen.findByText("How should we structure the next project slice?"),
  ).toBeVisible();
  expect(generateConsoleResponse).not.toHaveBeenCalled();

  await act(async () => {
    resolveCapabilities({
      kind: "capabilities",
      catalog: {
        schema_version: 1,
        default_agent_id: "qwen3-14b",
        commands: [],
        skills: [],
        agents: [
          {
            id: "qwen3-14b",
            role: "frontier",
            provider: "ollama",
            runner: "ollama",
            model: "qwen3:14b",
            routing: "controller",
            availability: "available",
            availability_reason: "",
            assignable: false,
            delegate_only: false,
            requires_approval: false,
          },
          {
            id: "qwen3.6-27b",
            role: "frontier",
            provider: "ollama",
            runner: "ollama",
            model: "qwen3.6:27b",
            routing: "router",
            availability: "available",
            availability_reason: "",
            assignable: false,
            delegate_only: false,
            requires_approval: false,
          },
        ],
      },
    });
  });
  await waitFor(() => expect(generateConsoleResponse).toHaveBeenCalledTimes(1));
  expect(generateConsoleResponse.mock.calls[0][0]).toMatchObject({
    agent_id: "qwen3.6-27b",
  });
});

test("does not bypass a failed controller capability boundary", async () => {
  const generateConsoleResponse = vi.fn();
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadAgentCapabilities: async () => ({
          kind: "capabilities-failure",
          code: "catalog-unavailable",
          message: "Controller registry is temporarily unavailable.",
          recoverable: true,
        }),
        appendConsoleMessage: async (request) => ({
          kind: "message",
          message: {
            message_id: "console-controller-boundary-000001",
            sequence: 1,
            role: "user",
            content: request.content,
            scope: snapshot.conversation_scope,
            outcome: "proposed",
            source: "mission-commander",
          },
        }),
        generateConsoleResponse,
      }}
    />,
  );
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "How should we document the next decision?" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  expect(
    await screen.findByText(/Controller response paused because the capability catalog/),
  ).toBeVisible();
  expect(generateConsoleResponse).not.toHaveBeenCalled();
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

test("reuses the exact terminal correlation after a lost submit response", async () => {
  const requests: Array<Parameters<NonNullable<WorkspaceClient["submitShellTerminalCommand"]>>[0]> = [];
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: { schema_version: 1 as const, revision: requests.length, commands: [], grants: [] },
  }));
  const submitShellTerminalCommand: NonNullable<WorkspaceClient["submitShellTerminalCommand"]> =
    vi.fn(async (request) => {
      requests.push(request);
      if (requests.length === 1) {
        return {
          kind: "command-rejected" as const,
          code: "bridge-response-lost",
          message: "The command response was lost after submission.",
        };
      }
      return {
        kind: "command-result" as const,
        result: {
          command_id: "terminal-command-000009",
          correlation_id: request.correlation_id,
          classification: "auto-allowed" as const,
          status: "outcome-unknown" as const,
          exit_code: null,
          stdout: "",
          stderr: "The command started; inspect effects before deciding what to do next.",
        },
      };
    });
  render(<App client={{ ...client, loadShellTerminal, submitShellTerminalCommand }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await openCommandAudit();
  const commandInput = screen.getByRole("textbox", { name: "Command" });
  fireEvent.change(commandInput, { target: { value: "python3 -m unittest --help" } });

  fireEvent.click(screen.getByRole("button", { name: "Run command" }));
  await waitFor(() => expect(requests).toHaveLength(1));
  await screen.findByText("The command response was lost after submission.");
  expect(commandInput).toHaveValue("python3 -m unittest --help");

  fireEvent.click(screen.getByRole("button", { name: "Run command" }));
  await waitFor(() => expect(requests).toHaveLength(2));
  expect(requests[1].correlation_id).toBe(requests[0].correlation_id);
  expect(requests[1]).toEqual(requests[0]);
  fireEvent.click(
    await screen.findByRole("button", {
      name: "Inspect full output for terminal-command-000009",
    }),
  );
  expect(screen.getByLabelText("Full command output for terminal-command-000009")).toHaveTextContent(
    "inspect effects",
  );
});

test("polls authoritative terminal state while a command is executing", async () => {
  let executionStarted = false;
  let resolveSubmission!: (
    result: Awaited<ReturnType<NonNullable<WorkspaceClient["submitShellTerminalCommand"]>>>,
  ) => void;
  const pendingSubmission = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["submitShellTerminalCommand"]>>>
  >((resolve) => {
    resolveSubmission = resolve;
  });
  const executingCommand = {
    command_id: "terminal-command-000010",
    correlation_id: "terminal-live-ui-1",
    command: "python3 -m unittest --help",
    classification: "auto-allowed" as const,
    status: "executing" as const,
    exit_code: null,
    working_directory: "/workspace/albert",
    requested_paths: [],
    access_level: "read" as const,
    requester: "mission-commander",
    approver: "",
    decider: "",
    reason: "",
  };
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: executionStarted ? 1 : 0,
      commands: executionStarted ? [executingCommand] : [],
      grants: [],
    },
  }));
  const submitShellTerminalCommand = vi.fn((request) => {
    executionStarted = true;
    executingCommand.correlation_id = request.correlation_id;
    return pendingSubmission;
  });
  render(<App client={{ ...client, loadShellTerminal, submitShellTerminalCommand }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  const terminal = await openCommandAudit();
  fireEvent.change(screen.getByRole("textbox", { name: "Command" }), {
    target: { value: "python3 -m unittest --help" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Run command" }));

  expect(await within(terminal).findByText("auto-allowed / executing", {}, { timeout: 1500 })).toBeVisible();

  await act(async () => {
    resolveSubmission({
      kind: "command-result",
      result: {
        command_id: executingCommand.command_id,
        correlation_id: executingCommand.correlation_id,
        classification: "auto-allowed",
        status: "completed",
        exit_code: 0,
        stdout: "done\n",
        stderr: "",
      },
    });
    await pendingSubmission;
  });
  expect(await screen.findByLabelText("Command output")).toHaveTextContent("done");
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

test("reloads canonical terminal state after a lost approval response", async () => {
  const pending = pendingTerminalCommand("human-required");
  const completed = { ...pending, status: "completed" as const, exit_code: 0 };
  let approvalReachedBackend = false;
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: approvalReachedBackend ? 2 : 1,
      commands: [approvalReachedBackend ? completed : pending],
      grants: [],
    },
  }));
  const decideShellTerminalCommand = vi.fn(async () => {
    approvalReachedBackend = true;
    return {
      kind: "command-rejected" as const,
      code: "bridge-response-lost",
      message: "The approval response was lost after execution.",
    };
  });
  render(<App client={{ ...client, loadShellTerminal, decideShellTerminalCommand }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  const terminal = await openCommandAudit();
  fireEvent.click(await screen.findByRole("button", { name: `Approve ${pending.command_id}` }));

  expect(await within(terminal).findByText("The approval response was lost after execution.")).toBeVisible();
  expect(await within(terminal).findByText("human-required / completed")).toBeVisible();
  await waitFor(() => expect(loadShellTerminal.mock.calls.length).toBeGreaterThanOrEqual(3));
});

test("polls authoritative terminal state while an approved command executes", async () => {
  const pending = pendingTerminalCommand("human-required");
  const executing = { ...pending, status: "executing" as const, exit_code: null };
  let executionStarted = false;
  let resolveDecision!: (
    result: Awaited<ReturnType<NonNullable<WorkspaceClient["decideShellTerminalCommand"]>>>,
  ) => void;
  const pendingDecision = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["decideShellTerminalCommand"]>>>
  >((resolve) => {
    resolveDecision = resolve;
  });
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: executionStarted ? 2 : 1,
      commands: [executionStarted ? executing : pending],
      grants: [],
    },
  }));
  const decideShellTerminalCommand = vi.fn(() => {
    executionStarted = true;
    return pendingDecision;
  });
  render(<App client={{ ...client, loadShellTerminal, decideShellTerminalCommand }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  const terminal = await openCommandAudit();
  fireEvent.click(await screen.findByRole("button", { name: `Approve ${pending.command_id}` }));

  expect(
    await within(terminal).findByText("human-required / executing", {}, { timeout: 1500 }),
  ).toBeVisible();

  await act(async () => {
    resolveDecision({
      kind: "command-result",
      result: {
        command_id: pending.command_id,
        correlation_id: pending.correlation_id,
        classification: pending.classification,
        status: "completed",
        exit_code: 0,
        stdout: "approved output\n",
        stderr: "",
      },
    });
    await pendingDecision;
  });
  expect(await screen.findByLabelText("Command output")).toHaveTextContent("approved output");
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
    request_id: "path-grant-request-000001",
    path: "/external/docs",
    access_level: "write" as const,
    duration_seconds: 900,
    granted_by: "mission-commander" as const,
    granted_at: "2026-07-01T12:00:00Z",
    expires_at: "2099-07-01T12:15:00Z",
  };
  let grantRequested = false;
  let grantCreated = false;
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: grantCreated ? 2 : 1,
      commands: [command],
      grants: grantCreated ? [grant] : [],
      path_grant_requests: grantRequested
        ? [{
            request_id: grant.request_id,
            correlation_id: "terminal-inline-grant-1",
            mission_id: "command-deck",
            path: "/external/docs",
            access_level: "write" as const,
            duration_seconds: 900,
            requester: "mission-commander",
            requested_at: "2026-07-01T11:59:00Z",
            reason:
              "Shell Terminal working directory is outside the workspace and has no active write Additional Path Grant.",
            affected_action: "touch report.md",
            status: grantCreated ? "granted" as const : "pending" as const,
          }]
        : [],
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
  const submitShellTerminalCommand = vi.fn(async () => {
    grantRequested = true;
    return {
      kind: "command-rejected" as const,
      code: "invalid-action",
      message:
        "Shell Terminal working directory is outside the workspace and has no active write Additional Path Grant.",
    };
  });
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
    request_id: "path-grant-request-000001",
    expected_revision: 1,
    path: "/external/docs",
    access_level: "write",
    duration_seconds: 900,
    requester: "mission-commander",
  });
  expect(await within(transcript).findByText(/Orchestrator accepted workstation action: Created path-grant-000001/)).toBeVisible();
});

test("persists contextual path grant denials before showing Agent Console acknowledgement", async () => {
  let grantRequested = false;
  let grantDenied = false;
  const requestReason =
    "Shell Terminal requested path is outside the workspace and has no active read Additional Path Grant: /external/notes.md";
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: grantDenied ? 2 : 1,
      commands: [],
      grants: [],
      path_grant_requests: grantRequested
        ? [{
            request_id: "path-grant-request-000001",
            correlation_id: "terminal-path-denial-1",
            mission_id: "command-deck",
            path: "/external/notes.md",
            access_level: "read" as const,
            duration_seconds: 900,
            requester: "mission-commander",
            requested_at: "2026-07-11T07:59:00Z",
            reason: requestReason,
            affected_action: "cat /external/notes.md",
            status: grantDenied ? "denied" as const : "pending" as const,
          }]
        : [],
    },
  }));
  const submitShellTerminalCommand = vi.fn(async () => {
    grantRequested = true;
    return {
      kind: "command-rejected" as const,
      code: "invalid-action",
      message: requestReason,
    };
  });
  let denialAttempts = 0;
  const denyAdditionalPathGrant = vi.fn(
    async (request: AdditionalPathGrantDenialRequest): Promise<AdditionalPathGrantDenialResult> => {
      denialAttempts += 1;
      if (denialAttempts === 1) {
        return {
          kind: "path-grant-rejected",
          code: "stale-action",
          message: "Shell Terminal revision changed; review and retry the denial.",
        };
      }
      grantDenied = true;
      return {
        kind: "path-grant-denied",
        denial: {
          denial_id: "path-grant-denial-000001",
          correlation_id: request.correlation_id,
          request_id: request.request_id,
          path: request.path,
          access_level: request.access_level,
          duration_seconds: request.duration_seconds,
          denied_by: "mission-commander",
          denied_at: "2026-07-11T08:00:00Z",
          reason: request.reason,
          affected_action: request.affected_action,
        },
      };
    },
  );
  render(
    <App
      client={{
        ...client,
        loadShellTerminal,
        submitShellTerminalCommand,
        denyAdditionalPathGrant,
      }}
    />,
  );
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

  await waitFor(() =>
    expect(denyAdditionalPathGrant).toHaveBeenCalledWith({
      correlation_id: expect.stringMatching(
        /^path-grant-denied-path-grant-request-000001-/,
      ),
      request_id: "path-grant-request-000001",
      expected_revision: 1,
      path: "/external/notes.md",
      access_level: "read",
      duration_seconds: 900,
      requester: "mission-commander",
      reason: requestReason,
      affected_action: "cat /external/notes.md",
    }),
  );
  expect(
    await within(transcript).findByText(/Orchestrator rejected workstation action: Shell Terminal revision changed/),
  ).toBeVisible();
  expect(within(grantPrompt).queryByText("Mission Commander denied this grant request.")).not.toBeInTheDocument();

  fireEvent.click(
    within(grantPrompt).getByRole("button", {
      name: "Deny grant request for /external/notes.md",
    }),
  );
  await waitFor(() => expect(denyAdditionalPathGrant).toHaveBeenCalledTimes(2));
  expect(
    await within(transcript).findAllByText(/Mission Commander requested Deny Additional Path Grant for \/external\/notes.md/),
  ).toHaveLength(2);
  expect(
    await within(transcript).findByText(/Orchestrator accepted workstation action: Recorded path-grant-denial-000001/),
  ).toBeVisible();
  expect(within(grantPrompt).getByText("Mission Commander denied this grant request.")).toBeVisible();
});

test("restores a canonical pending path grant request without parsing rejection prose", async () => {
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: 4,
      commands: [],
      grants: [],
      path_grant_requests: [{
        request_id: "path-grant-request-000004",
        correlation_id: "terminal-restored-grant-1",
        mission_id: "command-deck",
        path: "/external/exact-second-path",
        access_level: "read" as const,
        duration_seconds: 900,
        requester: "mission-commander",
        requested_at: "2026-07-11T09:00:00Z",
        reason: "Typed authority is required for the exact second path.",
        affected_action: "cat first.txt second.txt",
        status: "pending" as const,
      }],
    },
  }));

  render(<App client={{ ...client, loadShellTerminal }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  const prompt = await within(
    screen.getByRole("region", { name: "Prompt Transcript" }),
  ).findByRole("group", {
    name: "Additional Path Grant request for /external/exact-second-path",
  });
  expect(within(prompt).getByText("Access / read")).toBeVisible();
  expect(within(prompt).getByText("Typed authority is required for the exact second path.")).toBeVisible();
  expect(within(prompt).getByText("Affected action / cat first.txt second.txt")).toBeVisible();
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

  expect(
    await within(screen.getByRole("region", { name: "Shell Terminal" })).findByRole("alert"),
  ).toHaveTextContent(
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
    const stackedLayout = stylesSource.slice(
      stylesSource.indexOf("@media (max-width: 1040px)"),
      stylesSource.indexOf("@media (max-width: 680px)"),
    );
    expect(stackedLayout).toMatch(
      /\.deck-grid\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\);[^}]*grid-template-rows:\s*minmax\(34rem,\s*68vh\)\s+auto;/s,
    );
    expect(stackedLayout).toMatch(/\.prompt-workspace\s*\{[^}]*min-height:\s*34rem;/s);
    expect(stackedLayout).toMatch(/\.agent-workstations\s*\{[^}]*min-height:\s*28rem;/s);
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
  expectPromptScope("Restore workspace session");
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
  const shellTerminal = screen.getByRole("region", { name: "Shell Terminal" });
  expect(shellTerminal).toBeVisible();
  expect(within(shellTerminal).getByRole("alert")).toHaveTextContent(
    "Shell Terminal transport is unavailable",
  );
  for (const control of document.querySelectorAll("button, input, select, textarea, a[href]")) {
    expect(control).toHaveAccessibleName();
  }
});

test("exposes named workstation hierarchy regions and compact assignment row labels", async () => {
  const accessibleSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    mission_board: {
      ...snapshot.mission_board,
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-A11Y",
          title: "Harden workstation hierarchy",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: true,
          lifecycle: "Approved",
        }),
      ],
      ordered_issue_ids: ["ISS-A11Y"],
      ready_issue_ids: ["ISS-A11Y"],
    },
  };

  render(<App client={{ loadSnapshot: async () => ({ kind: "ready", snapshot: accessibleSnapshot }) }} />);

  expect(await screen.findByRole("region", { name: "Agent Console" })).toBeVisible();
  expect(screen.getByRole("region", { name: "Mission Execution Tree" })).toBeVisible();
  expect(screen.getByRole("region", { name: "Issue Assignment Board" })).toBeVisible();

  const board = screen.getByRole("table", { name: "Issue Assignment Board" });
  const row = within(board).getByRole("row", {
    name: /ISS-A11Y Harden workstation hierarchy Unassigned Ready Clear No workstation/,
  });
  const cells = within(row).getAllByRole("cell");
  for (const [index, label] of ["Issue", "Owner", "State", "Blockers", "Workstation", "Actions"].entries()) {
    expect(cells[index]).toHaveAttribute("data-label", label);
  }
  expect(stylesSource).toMatch(
    /@media \(max-width: 520px\)[\s\S]*\.issue-assignment-board td::before \{[\s\S]*content: attr\(data-label\)/,
  );
});

test("exposes execution tree sessions as keyboard-reachable accessible summaries", async () => {
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
            artifact_links: [
              "app-local://evidence/session-ISS-01-1",
              "app-local://missions/command-deck/sessions/session-ISS-01-1/artifacts/review_diff/review.diff",
            ],
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

    const sessionNode = executionTreeNode("session-ISS-01-1");
    sessionNode.focus();
    expect(sessionNode).toHaveFocus();
    expect(sessionNode).toHaveAccessibleName(
      /Local Agent session session-ISS-01-1; .*; Working; No elevated risk/,
    );

    const inspector = await openExecutionInspector("session-ISS-01-1");
    expect(within(inspector).getByRole("button", { name: "Diff mission-control/src/App.tsx" })).toBeVisible();

    const cancel = within(inspector).getByRole("button", { name: "Cancel session" });
    expect(cancel).toBeDisabled();
    expect(cancel).toHaveAccessibleDescription(
      /Acknowledgement records cancellation for session-ISS-01-1;.*Enter a reason to enable Cancel session for session-ISS-01-1\./,
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
            entity_id: "ADHOC-000001",
            queue_item_id: "delegation-command-deck-ISS-02",
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
    const tree = await screen.findByRole("region", { name: "Mission Execution Tree" });
    expect(prompt.compareDocumentPosition(tree) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
    expect(screen.getByRole("region", { name: "Prompt Composer" })).toBeVisible();
    const queueInspector = await openExecutionInspector("ADHOC-000001", "ad-hoc-delegation");
    expect(within(queueInspector).getByRole("button", { name: "Approve" })).toBeEnabled();
    const failedNode = executionTreeNode("session-ISS-03-1");
    expect(failedNode).toHaveAttribute("data-risk", "failed");
    const failedInspector = await openExecutionInspector("session-ISS-03-1");
    expect(within(failedInspector).getByText("Failed", { selector: "dd" })).toBeVisible();
    expect(within(failedInspector).getByRole("button", { name: "Retry" })).toBeDisabled();
    expect(within(failedInspector).getByRole("button", { name: "Retry" })).toHaveAccessibleDescription(
      /Acknowledgement queues one canonical repair or retry session for session-ISS-03-1;.*Enter a reason to enable Retry for session-ISS-03-1\./,
    );

    await openDetailViews();
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

  expect(await screen.findByRole("heading", { name: "Command Deck Mission" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Open detail views" })).toBeVisible();
  expect(screen.queryByRole("heading", { name: "Review Workspace" })).not.toBeInTheDocument();
  await openDetailViews();
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

  await openDetailViews();
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

test("surfaces and retries a Review Workspace projection load failure", async () => {
  let loads = 0;
  render(
    <App
      client={{
        loadSnapshot: async () => ({
          kind: "ready",
          snapshot: { ...snapshot, operations_view: "review-workspace" },
        }),
        loadReviewWorkspace: async () => {
          loads += 1;
          return loads === 1
            ? {
                kind: "review-workspace-failure",
                code: "evidence-unavailable",
                message: "Evidence projection is temporarily unavailable.",
                recoverable: true,
              }
            : {
                kind: "review-workspace",
                projection: {
                  schema_version: 1,
                  revision: 2,
                  mission_id: "command-deck",
                  items: [],
                },
              };
        },
      }}
    />,
  );

  await openDetailViews();
  expect(
    await screen.findByText(/Review Workspace load failed: Evidence projection is temporarily unavailable/),
  ).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Retry Review Workspace" }));
  expect(await screen.findByText("No evidence awaiting review")).toBeVisible();
  expect(loads).toBe(2);
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
  const loadSessionArtifact = vi.fn(async (request: SessionArtifactReadRequest) => ({
    kind: "session-artifact" as const,
    artifact: {
      schema_version: 1 as const,
      mission_id: request.mission_id,
      session_id: request.session_id,
      artifact_id: "evidence-package",
      label: "Evidence Package",
      media_type: "application/json",
      content: "{\"evidence_valid\":true}",
      byte_count: 23,
      content_limit_bytes: 128_000,
      truncated: false,
    },
  }));
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
    loadSessionArtifact,
  };

  render(<App client={activityClient} />);

  await openDetailViews();
  const activity = await screen.findByRole("region", { name: "Activity Journal" });
  expect(await within(activity).findByText("Mission Commander selected Operations Workspace view Activity.")).toBeVisible();
  expect(within(activity).getByText("Mission Commander recorded Review Workspace decision Approved.")).toBeVisible();
  expect(within(activity).getByText("issue-slice / ISS-01")).toBeVisible();
  expect(
    within(activity).queryByRole("link", {
      name: "Evidence Package session-ISS-01-1",
    }),
  ).not.toBeInTheDocument();
  expect(within(activity).getByText("Evidence Package session-ISS-01-1")).toBeVisible();
  fireEvent.click(within(activity).getByRole("button", { name: "Open activity evidence 1" }));
  const viewer = await screen.findByRole("region", { name: "Session evidence viewer" });
  await waitFor(() => expect(viewer).toHaveFocus());
  expect(within(viewer).getByLabelText("Evidence Package content")).toHaveTextContent(
    '"evidence_valid":true',
  );
  expect(loadSessionArtifact).toHaveBeenCalledWith({
    mission_id: "command-deck",
    session_id: "session-ISS-01-1",
    artifact_ref: "app-local://evidence/session-ISS-01-1",
  });

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

test("surfaces and retries an Activity Journal load failure without discarding canonical state", async () => {
  let loads = 0;
  render(
    <App
      client={{
        loadSnapshot: async () => ({
          kind: "ready",
          snapshot: { ...snapshot, operations_view: "activity" },
        }),
        loadActivityJournal: async () => {
          loads += 1;
          return loads === 1
            ? {
                kind: "activity-journal-failure",
                code: "journal-unavailable",
                message: "Activity storage is temporarily unavailable.",
                recoverable: true,
              }
            : {
                kind: "activity-journal",
                projection: { schema_version: 1, revision: 4, entries: [] },
              };
        },
      }}
    />,
  );

  await openDetailViews();
  expect(
    await screen.findByText(
      "Activity Journal load failed: Activity storage is temporarily unavailable.",
    ),
  ).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Retry Activity Journal" }));
  expect(await screen.findByText("No Activity Journal entries")).toBeVisible();
  await waitFor(() => expect(loads).toBe(2));
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
    loadMissionDrafts: async () => ({
      kind: "mission-drafts",
      projection: { schema_version: 1, revision: 1, drafts: [] },
    }),
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

  await openDetailViews();
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
  expect(await within(queue).findByText("No decisions pending")).toBeVisible();
});

test("Queue replaces assignment work and shows only pending governance decisions", async () => {
  const pendingItem: WorkspaceQueueItem = {
    item_id: "frontier-confirmation-command-deck-000001",
    mission_id: "command-deck",
    item_type: "frontier-confirmation",
    status: "pending",
    source: "frontier-model",
    requested_action: "Confirm the bounded implementation choice",
    affected_boundary: "implementation-choice",
    consequence: "Approval lets the active automated work continue.",
    issue_id: "ISS-22",
    proposed_changes: { choice: "Keep the Queue decision-only." },
  };
  const resolvedItem: WorkspaceQueueItem = {
    ...pendingItem,
    item_id: "frontier-confirmation-command-deck-000000",
    status: "approved",
    requested_action: "Historical confirmation already approved",
  };
  const drafts: MissionDraftProjection = {
    schema_version: 1,
    revision: 3,
    drafts: [
      {
        draft_id: "mission-draft-command-deck-000001",
        mission_id: "command-deck",
        status: "draft",
        proposed_goal: "Confirm this pending Mission Draft",
        included_ad_hoc_work: [],
        excluded_ad_hoc_work_ids: [],
        new_work_items: ["One bounded follow-up."],
        dependencies: [],
        unresolved_decisions: [],
      },
      {
        draft_id: "mission-draft-command-deck-000000",
        mission_id: "command-deck",
        status: "confirmed",
        proposed_goal: "Historical Mission Draft already confirmed",
        included_ad_hoc_work: [],
        excluded_ad_hoc_work_ids: [],
        new_work_items: [],
        dependencies: [],
        unresolved_decisions: [],
      },
    ],
  };

  render(
    <App
      client={{
        loadSnapshot: async () => ({
          kind: "ready",
          snapshot: { ...snapshot, operations_view: "workspace-queue" },
        }),
        loadWorkspaceQueue: async () => ({
          kind: "workspace-queue",
          projection: {
            schema_version: 1,
            revision: 4,
            items: [resolvedItem, pendingItem],
            groups: [
              {
                group_id: "frontier-confirmation:command-deck",
                item_type: "frontier-confirmation",
                mission_id: "command-deck",
                item_count: 2,
                items: [resolvedItem, pendingItem],
              },
            ],
          },
        }),
        loadMissionDrafts: async () => ({ kind: "mission-drafts", projection: drafts }),
      }}
    />,
  );

  const missionWork = await screen.findByRole("complementary", { name: "Mission Work" });
  await openDetailViews();
  const queue = await within(missionWork).findByRole("region", { name: "Workspace Queue" });

  expect(within(queue).getByText("2 decisions pending")).toBeVisible();
  expect(within(missionWork).queryByRole("table", { name: "Issue Assignment Board" })).not.toBeInTheDocument();
  expect(within(queue).queryByRole("region", { name: "Mission Draft creation" })).not.toBeInTheDocument();
  expect(within(queue).queryByRole("region", { name: "Ad Hoc Delegation proposal" })).not.toBeInTheDocument();
  expect(within(queue).getByText("Confirm the bounded implementation choice")).toBeVisible();
  expect(within(queue).queryByText("Historical confirmation already approved")).not.toBeInTheDocument();
  expect(within(queue).getByText("Confirm this pending Mission Draft")).toBeVisible();
  expect(within(queue).queryByText("Historical Mission Draft already confirmed")).not.toBeInTheDocument();
});

test("Queue keeps the combined decision count explicitly loading until Mission Drafts arrive", async () => {
  const pendingMissionDrafts = new Promise<never>(() => {});
  render(
    <App
      client={{
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
        loadMissionDrafts: async () => pendingMissionDrafts,
      }}
    />,
  );

  await openDetailViews();
  const queue = await screen.findByRole("region", { name: "Workspace Queue" });
  expect(within(queue).getByText("Loading decisions")).toBeVisible();
  expect(within(queue).queryByText("0 governance decisions pending")).not.toBeInTheDocument();
  expect(within(queue).queryByText("No decisions pending")).not.toBeInTheDocument();
});

test("surfaces and retries a Workspace Queue projection load failure", async () => {
  let loads = 0;
  render(
    <App
      client={{
        loadSnapshot: async () => ({
          kind: "ready",
          snapshot: { ...snapshot, operations_view: "workspace-queue" },
        }),
        loadWorkspaceQueue: async () => {
          loads += 1;
          return loads === 1
            ? {
                kind: "workspace-queue-failure",
                code: "queue-unavailable",
                message: "Governance queue is temporarily unavailable.",
                recoverable: true,
              }
            : {
                kind: "workspace-queue",
                projection: {
                  schema_version: 1,
                  revision: 2,
                  items: [],
                  groups: [],
                },
              };
        },
        loadMissionDrafts: async () => ({
          kind: "mission-drafts",
          projection: { schema_version: 1, revision: 1, drafts: [] },
        }),
      }}
    />,
  );

  await openDetailViews();
  expect(
    await screen.findByText(/Workspace Queue load failed: Governance queue is temporarily unavailable/),
  ).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Retry Workspace Queue" }));
  expect(await screen.findByText("No decisions pending")).toBeVisible();
  expect(loads).toBe(2);
});

test("surfaces and retries a Mission Draft projection load failure", async () => {
  let draftLoads = 0;
  render(
    <App
      client={{
        loadSnapshot: async () => ({
          kind: "ready",
          snapshot: { ...snapshot, operations_view: "workspace-queue" },
        }),
        loadWorkspaceQueue: async () => ({
          kind: "workspace-queue",
          projection: {
            schema_version: 1,
            revision: 2,
            items: [],
            groups: [],
          },
        }),
        loadMissionDrafts: async () => {
          draftLoads += 1;
          return draftLoads === 1
            ? {
                kind: "mission-drafts-failure",
                code: "drafts-unavailable",
                message: "Mission Draft persistence is temporarily unavailable.",
                recoverable: true,
              }
            : {
                kind: "mission-drafts",
                projection: {
                  schema_version: 1,
                  revision: 2,
                  drafts: [],
                },
              };
        },
      }}
    />,
  );

  await openDetailViews();
  expect(
    await screen.findByText(
      /Mission Draft load failed: Mission Draft persistence is temporarily unavailable/,
    ),
  ).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Retry Mission Drafts" }));
  expect(await screen.findByText("No decisions pending")).toBeVisible();
  expect(screen.queryByRole("region", { name: "Mission Drafts" })).not.toBeInTheDocument();
  expect(draftLoads).toBe(2);
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

  await openDetailViews();
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

  await openDetailViews();
  const queue = await screen.findByRole("region", { name: "Workspace Queue" });
  const drafts = await within(queue).findByRole("region", { name: "Mission Drafts" });
  fireEvent.change(within(drafts).getByLabelText("Mission Draft decision reason"), {
    target: { value: "Mission Commander confirmed this scope." },
  });
  fireEvent.click(within(drafts).getByRole("button", { name: "Confirm mission-draft-command-deck-000001" }));

  await waitFor(() =>
    expect(
      screen.getByRole("status", { name: "Mission Draft decision status" }),
    ).toHaveTextContent("Acknowledged"),
  );
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
  expect(await within(queue).findByText("No decisions pending")).toBeVisible();
  expect(within(queue).queryByRole("region", { name: "Mission Drafts" })).not.toBeInTheDocument();
});

test("preserves canonical Mission state when a Mission Draft decision reload fails", async () => {
  const draftProjection: MissionDraftProjection = {
    schema_version: 1,
    revision: 6,
    drafts: [
      {
        draft_id: "mission-draft-command-deck-000010",
        mission_id: "command-deck",
        status: "draft",
        proposed_goal: "Keep the accepted Mission visible during recovery.",
        included_ad_hoc_work: [],
        excluded_ad_hoc_work_ids: [],
        new_work_items: ["Confirm the recovery behavior."],
        dependencies: [],
        unresolved_decisions: [],
      },
    ],
  };
  let snapshotLoads = 0;
  render(
    <App
      client={{
        loadSnapshot: async () => {
          snapshotLoads += 1;
          return snapshotLoads === 1
            ? {
                kind: "ready",
                snapshot: { ...snapshot, operations_view: "workspace-queue" },
              }
            : {
                kind: "persistence-read-failure",
                message: "Canonical state could not be reread.",
                recoverable: true,
              };
        },
        loadWorkspaceQueue: async () => ({
          kind: "workspace-queue",
          projection: { schema_version: 1, revision: 1, items: [], groups: [] },
        }),
        loadMissionDrafts: async () => ({
          kind: "mission-drafts",
          projection: draftProjection,
        }),
        submitMissionDraftDecision: async (request) => ({
          kind: "acknowledged",
          acknowledgement: {
            correlation_id: request.correlation_id,
            outcome: "acknowledged",
            revision: 7,
            draft_id: request.draft_id,
            draft_status: "confirmed",
            effect_summary: "Mission Draft confirmed as ISS-RECOVERY.",
            accepted_issue_id: "ISS-RECOVERY",
          },
        }),
      }}
    />,
  );

  await openDetailViews();
  const queue = await screen.findByRole("region", { name: "Workspace Queue" });
  const drafts = await within(queue).findByRole("region", { name: "Mission Drafts" });
  fireEvent.change(within(drafts).getByLabelText("Mission Draft decision reason"), {
    target: { value: "The bounded scope is accepted." },
  });
  fireEvent.click(
    within(drafts).getByRole("button", {
      name: "Confirm mission-draft-command-deck-000010",
    }),
  );

  await waitFor(() =>
    expect(
      screen.getByRole("status", { name: "Mission Draft decision status" }),
    ).toHaveTextContent(
      "Mission Draft was acknowledged, but canonical snapshot reload failed: " +
        "Canonical state could not be reread. Retry the canonical workspace load.",
    ),
  );
  expect(screen.getByRole("heading", { name: "Command Deck Mission" })).toBeVisible();
  expect(screen.getByRole("region", { name: "Mission Drafts" })).toBeVisible();
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
  await openDetailViews();
  const workspace = await screen.findByRole("region", { name: "Review Workspace" });

  fireEvent.click(within(workspace).getByRole("button", { name: "Accept session-ISS-01-1" }));

  expect(await screen.findByRole("status", { name: "Review decision status" })).toHaveTextContent("Pending");
  expect(screen.getByText(/Workstation action: Mission Commander requested Accept evidence for session-ISS-01-1/)).toBeVisible();
  expect(screen.getByText("Orchestrator validating workstation action.")).toBeVisible();
  expect(screen.getByText("session-ISS-01-1")).toBeVisible();
  expect(requests).toEqual([
    {
      correlation_id: "review-accept-command-deck-session-ISS-01-1-4",
      action_type: "review-decision",
      actor: "mission-commander",
      expected_revision: 4,
      target: {
        kind: "agent-session",
        id: "session-ISS-01-1",
      },
      mission_id: "command-deck",
      session_id: "session-ISS-01-1",
      decision: "accept",
      reason: "",
    },
  ]);

  await act(async () => {
    resolveDecision({
      kind: "acknowledged",
      acknowledgement: {
        correlation_id: "review-accept-command-deck-session-ISS-01-1-4",
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
  await openDetailViews();
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
      correlation_id: "review-repair-command-deck-session-ISS-01-1-4",
      action_type: "review-decision",
      actor: "mission-commander",
      expected_revision: 4,
      target: {
        kind: "agent-session",
        id: "session-ISS-01-1",
      },
      mission_id: "command-deck",
      session_id: "session-ISS-01-1",
      decision: "repair",
      reason: "Acceptance copy is missing.",
    },
  ]);
});

test.each([
  {
    label: "Issue Slice",
    issueId: "ISS-REPAIR",
    priorSessionId: "session-ISS-REPAIR-1",
    repairSessionId: "session-ISS-REPAIR-2",
  },
  {
    label: "Ad Hoc Delegation",
    issueId: "ADHOC-000001",
    priorSessionId: "session-ADHOC-000001-1",
    repairSessionId: "session-ADHOC-000001-2",
  },
])(
  "reloads and dispatches one canonical $label repair without stale UI state",
  async ({ issueId, priorSessionId, repairSessionId }) => {
    const session = (
      status: string,
      repairActionAvailable: boolean,
      sessionId = priorSessionId,
    ) => ({
      session_id: sessionId,
      issue_id: issueId,
      assigned_agent: "repair-worker",
      status,
      role: "local-agent",
      provider: "test-harness",
      model: "deterministic-fake",
      task_title: `Repair ${issueId}`,
      review_outcome: repairActionAvailable ? "Needs repair" : "",
      review_next_action: repairActionAvailable ? "same-local-agent-repair" : "",
      repair_action_available: repairActionAvailable,
      repair_task_packet: repairActionAvailable
        ? {
            issue_id: issueId,
            goal: `Repair ${issueId}`,
            acceptance_criteria: ["The failed acceptance assertion passes."],
            allowed_paths: ["src", "tests"],
            command_policy: { npm: "auto-allowed" },
            evidence_requirements: ["Record the repaired test result."],
            assigned_agent: "repair-worker",
            review_reason: "The reviewed result misses its acceptance assertion.",
          }
        : null,
    });
    const withSessions = (
      revision: number,
      sessions: readonly ReturnType<typeof session>[],
    ): WorkspaceSnapshot => ({
      ...snapshot,
      revision,
      operations_view: "review-workspace",
      mission_board: {
        ...snapshot.mission_board,
        issue_count: 0,
        ordered_issue_ids: [],
        ready_issue_ids: [],
        approved_issue_ids: [],
        issue_slices: [],
      },
      missions: [
        {
          id: "command-deck",
          title: "Command Deck Mission",
          issue_count: 0,
          is_active: true,
          sessions,
          attention: [],
        },
      ],
    });
    const initialSnapshot = withSessions(4, [session("evidence-ready", false)]);
    const repairedSnapshot = withSessions(5, [session("evidence-ready", true)]);
    const queuedSnapshot = withSessions(6, [
      session("evidence-ready", false),
      session("queued", false, repairSessionId),
    ]);
    const completedSnapshot = withSessions(6, [
      session("evidence-ready", false),
      session("evidence-ready", false, repairSessionId),
    ]);
    const snapshots = [initialSnapshot, repairedSnapshot, queuedSnapshot, completedSnapshot];
    let snapshotIndex = 0;
    const reviewRequests: ReviewDecisionRequest[] = [];
    const workstationRequests: WorkstationActionRequest[] = [];
    const runRequests: { readonly session_id: string; readonly mission_id?: string }[] = [];
    let resolveRun: ((value: Awaited<ReturnType<NonNullable<WorkspaceClient["runWorkstationSession"]>>>) => void) | null = null;
    const reviewProjection: ReviewWorkspaceProjection = {
      schema_version: 1,
      revision: 4,
      mission_id: "command-deck",
      items: [
        {
          mission_id: "command-deck",
          issue_id: issueId,
          issue_title: `Repair ${issueId}`,
          session_id: priorSessionId,
          assigned_agent: "repair-worker",
          status: "evidence-ready",
          lifecycle: "Evidence ready",
          evidence_complete: true,
          missing_evidence: [],
          can_accept: true,
          evidence: {
            changed_files: ["src/repair.ts"],
            diff_summary: "The first result needs repair.",
            commands_run: ["npm test"],
            test_results: "One acceptance assertion failed.",
            risks: "None.",
            proposed_context_updates: "",
            artifact_links: [],
          },
          visibility_limitations: [],
        },
      ],
    };
    const repairClient: WorkspaceClient = {
      loadSnapshot: async () => ({
        kind: "ready",
        snapshot: snapshots[Math.min(snapshotIndex++, snapshots.length - 1)],
      }),
      loadReviewWorkspace: async () => ({ kind: "review-workspace", projection: reviewProjection }),
      submitReviewDecision: async (request) => {
        reviewRequests.push(request);
        return {
          kind: "acknowledged",
          acknowledgement: {
            correlation_id: request.correlation_id,
            outcome: "acknowledged",
            revision: 5,
            issue_id: issueId,
            session_id: priorSessionId,
            review_outcome: "Needs repair",
            next_action: "same-local-agent-repair",
            issue_lifecycle: "Needs repair",
            effect_summary: `${issueId} needs repair; launch the canonical repair action.`,
          },
        };
      },
      submitWorkstationAction: async (request) => {
        workstationRequests.push(request);
        return {
          kind: "acknowledged",
          acknowledgement: {
            correlation_id: request.correlation_id,
            outcome: "acknowledged",
            revision: 6,
            action_type: "issue-retry",
            issue_id: issueId,
            session_id: repairSessionId,
            effect_summary: `Orchestrator queued repair ${repairSessionId}.`,
          },
        };
      },
      runWorkstationSession: async (request) => {
        runRequests.push(request);
        return await new Promise((resolve) => {
          resolveRun = resolve;
        });
      },
    };

    render(<App client={repairClient} syncIntervalMs={60_000} />);
    await openDetailViews();
    const review = await screen.findByRole("region", { name: "Review Workspace" });
    fireEvent.change(within(review).getByLabelText(`Review reason ${priorSessionId}`), {
      target: { value: "The reviewed result misses its acceptance assertion." },
    });
    fireEvent.click(within(review).getByRole("button", { name: `Request repair ${priorSessionId}` }));

    const repairInspector = await openExecutionInspector(priorSessionId);
    const launch = await within(repairInspector).findByRole("button", {
      name: "Launch repair",
    });
    expect(launch).toBeEnabled();
    expect(launch).toHaveAccessibleDescription(
      /queues exactly one canonical repair session for .* from its inherited review task packet; it does not run inline or duplicate the prior session/,
    );
    const repairPreview = within(repairInspector).getByRole("region", {
      name: "Inherited repair task packet",
    });
    expect(repairPreview).toHaveTextContent(`GoalRepair ${issueId}`);
    expect(repairPreview).toHaveTextContent("AcceptanceThe failed acceptance assertion passes.");
    expect(repairPreview).toHaveTextContent("Allowed pathssrc, tests");
    expect(repairPreview).toHaveTextContent("Command policynpm: auto-allowed");
    expect(repairPreview).toHaveTextContent("Evidence requiredRecord the repaired test result.");
    expect(repairPreview).toHaveTextContent("Assigned Local Agentrepair-worker");
    expect(repairPreview).toHaveTextContent(
      "Review reasonThe reviewed result misses its acceptance assertion.",
    );
    fireEvent.click(launch);
    fireEvent.click(launch);

    await waitFor(() => expect(runRequests).toEqual([
      { session_id: repairSessionId, mission_id: "command-deck" },
    ]));
    expect(reviewRequests).toHaveLength(1);
    expect(workstationRequests).toHaveLength(1);
    expect(workstationRequests[0]).toMatchObject({
      correlation_id: `workstation-issue-retry-command-deck-${priorSessionId}-5`,
      action_type: "issue-retry",
      actor: "mission-commander",
      expected_revision: 5,
      target: { kind: "agent-session", id: priorSessionId },
      mission_id: "command-deck",
      issue_id: issueId,
      session_id: priorSessionId,
    });
    expect(workstationRequests[0].reason).toBeUndefined();

    await act(async () => {
      resolveRun?.({
        kind: "session-finished",
        session: {
          schema_version: 1,
          mission_id: "command-deck",
          session_id: repairSessionId,
          issue_id: issueId,
          status: "evidence-ready",
          runner_started_at: "2026-07-11T10:00:00Z",
          runner_ended_at: "2026-07-11T10:00:01Z",
          runner_exit_status: 0,
          evidence_valid: true,
        },
      });
    });
    await waitFor(() => expect(snapshotIndex).toBeGreaterThanOrEqual(4));
    expect(runRequests).toHaveLength(1);
    expect(screen.queryByText(/stale action/i)).not.toBeInTheDocument();
  },
);

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
  await openDetailViews();
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
      correlation_id: "review-escalate-human-command-deck-session-ISS-01-1-4",
      action_type: "review-decision",
      actor: "mission-commander",
      expected_revision: 4,
      target: {
        kind: "agent-session",
        id: "session-ISS-01-1",
      },
      mission_id: "command-deck",
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
  await openDetailViews();
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

  await openDetailViews();
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

  await openDetailViews();
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

test("automatically retries a transient disconnect and reloads a fresh canonical snapshot", async () => {
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

  await waitFor(() => expect(snapshotLoads).toBeGreaterThanOrEqual(2));
  expect(screen.getByRole("status", { name: "Connection status" })).toHaveTextContent(
    "Connected",
  );
  expect(await screen.findByRole("button", { name: "Open detail views" })).toBeVisible();
  await openDetailViews();
  expect(await screen.findByRole("heading", { name: "Activity" })).toBeVisible();
  expect(screen.getByRole("status", { name: "Connection status" })).toHaveTextContent(
    "Connected",
  );
  expect(snapshotLoads).toBeGreaterThanOrEqual(2);
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

  expect(await screen.findByRole("heading", { name: "Background Mission" })).toBeVisible();
  expect(screen.getByRole("status", { name: "Connection status" })).toHaveTextContent(
    "Connected",
  );
  expect(screen.getByText("Keep this conversation anchored")).toBeVisible();
  expect(composer).toHaveValue("Continue on ISS-01");
  expectPromptScope("Restore workspace session");

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
            entity_id: "ISS-01",
            queue_item_id: "",
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

  await openDetailViews();
  fireEvent.change(screen.getByRole("combobox", { name: "Active Mission" }), {
    target: { value: "background-mission" },
  });

  expect(await screen.findByRole("heading", { name: "Background Mission" })).toBeVisible();
  expect(screen.getByText("Keep this workspace conversation continuous")).toBeVisible();
  expectPromptScope("Restore workspace session");
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

test("clears Mission-local issue session detail when Active Mission changes", async () => {
  const collidingSessionId = "session-shared-1";
  const session = {
    session_id: collidingSessionId,
    assigned_agent: "local-agent",
    role: "local-agent",
    provider: "ollama",
    model: "gemma4:12b",
    status: "running",
    stale: false,
    disconnected: false,
    operation_status: "streaming",
    failure: "",
  };
  const before: WorkspaceSnapshot = {
    ...snapshot,
    mission_board: {
      ...snapshot.mission_board,
      ordered_issue_ids: ["ISS-01"],
      issue_slices: [
        appIssueSlice({
          issue_id: "ISS-01",
          title: "Active mission issue",
          sessions: [session],
        }),
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 1,
        is_active: true,
        sessions: [],
        attention: [],
      },
      {
        id: "background-mission",
        title: "Background Mission",
        issue_count: 1,
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
      issue_count: 1,
    },
    mission_board: {
      ...before.mission_board,
      prd_title: "Background Mission",
      ordered_issue_ids: ["BG-01"],
      issue_slices: [
        appIssueSlice({
          issue_id: "BG-01",
          title: "Background mission issue",
          sessions: [session],
        }),
      ],
    },
    missions: before.missions?.map((mission) => ({
      ...mission,
      is_active: mission.id === "background-mission",
    })),
  };
  let switched = false;
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot: switched ? after : before }),
        switchMission: async (request) => {
          switched = true;
          return {
            kind: "acknowledged",
            acknowledgement: {
              correlation_id: request.correlation_id,
              outcome: "acknowledged",
              revision: 5,
            },
          };
        },
      }}
    />,
  );
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await openDetailViews();
  fireEvent.click(screen.getByRole("button", { name: "Inspect ISS-01" }));
  const initialInspector = screen.getByRole("region", { name: "Issue Slice Inspector" });
  fireEvent.click(within(initialInspector).getByRole("button", { name: `Session ${collidingSessionId}` }));
  expect(initialInspector.querySelector(".issue-session-detail")).not.toBeNull();

  fireEvent.change(screen.getByRole("combobox", { name: "Active Mission" }), {
    target: { value: "background-mission" },
  });
  expect(await screen.findByRole("heading", { name: "Background Mission" })).toBeVisible();
  const switchedInspector = screen.getByRole("region", { name: "Issue Slice Inspector" });
  expect(switchedInspector).toHaveTextContent("BG-01");
  expect(switchedInspector.querySelector(".issue-session-detail")).toBeNull();
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

  await openDetailViews();
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
  expectPromptScope("Command Deck Mission");

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

  await openDetailViews();
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

  await openDetailViews();
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
  await openDetailViews();
  fireEvent.click(screen.getByRole("button", { name: "Review" }));

  expect(await screen.findByRole("heading", { name: "Review Workspace" })).toBeVisible();
  expect(screen.getByText("Persisted guidance")).toBeVisible();
  expect(composer).toHaveValue("Unfinished mission question");
  expectPromptScope("Restore workspace session");
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

  await openContextInspector();
  fireEvent.change(screen.getByRole("combobox", { name: "Conversation Scope" }), {
    target: { value: "mission:command-deck" },
  });
  expect(scopeRequests).toEqual([]);
  fireEvent.click(screen.getByRole("button", { name: "Apply target" }));

  await waitFor(() => expectConversationScopeValue("mission:command-deck"));
  closeContextInspector();
  expectPromptScope("Command Deck Mission");
  expect(await screen.findByText(/Workstation action: Mission Commander requested Change Conversation Scope to Command Deck Mission/)).toBeVisible();
  const acceptedScopeChange = await screen.findByText(/Orchestrator accepted workstation action: Conversation Scope now targets Command Deck Mission/);
  expect(acceptedScopeChange).toBeVisible();
  expect(
    within(acceptedScopeChange.closest("article")!).getByText(
      "Receipt conversation-scope-mission-command-deck-4 · workstation-action-acknowledged",
    ),
  ).toBeVisible();
  expect(scopeRequests).toEqual([
    {
      correlation_id: "conversation-scope-mission-command-deck-4",
      action_type: "conversation-scope-change",
      actor: "mission-commander",
      expected_revision: 4,
      target: { kind: "conversation-scope", id: "command-deck" },
      scope_kind: "mission",
      scope_target: "command-deck",
      scope_label: "Command Deck Mission",
    },
  ]);
});

test("fails closed when a Workstation acknowledgement returns another correlation", async () => {
  const mismatchedClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    changeScope: async () => ({
      kind: "acknowledged",
      acknowledgement: {
        correlation_id: "forged-scope-acknowledgement",
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
            event_id: "workspace-5-scope-mismatch",
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
  render(<App client={mismatchedClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  await openContextInspector();
  fireEvent.change(screen.getByRole("combobox", { name: "Conversation Scope" }), {
    target: { value: "mission:command-deck" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Apply target" }));

  expect(
    await screen.findByText(
      /Orchestrator rejected workstation action: Orchestrator acknowledgement correlation did not match/,
    ),
  ).toBeVisible();
  expect(
    screen.queryByText(
      /Orchestrator accepted workstation action: Conversation Scope now targets/,
    ),
  ).not.toBeInTheDocument();
  expect(screen.queryByText(/^Receipt forged-scope/)).not.toBeInTheDocument();
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
  const inspector = await openContextInspector();

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

test("surfaces and retries a Working Context load failure", async () => {
  let loads = 0;
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadWorkingContext: async () => {
          loads += 1;
          return loads === 1
            ? {
                kind: "working-context-failure",
                code: "context-unavailable",
                message: "Context storage could not be read.",
                recoverable: true,
              }
            : {
                kind: "working-context",
                projection: {
                  schema_version: 1,
                  revision: 2,
                  scope: snapshot.conversation_scope,
                  sources: [],
                  content_character_count: 0,
                },
              };
        },
      }}
    />,
  );

  const inspector = await openContextInspector();
  expect(
    within(inspector).getByText(
      "Working Context load failed: Context storage could not be read.",
    ),
  ).toBeVisible();
  expect(within(inspector).getByText("Context unavailable")).toBeVisible();
  fireEvent.click(within(inspector).getByRole("button", { name: "Retry Working Context" }));
  await waitFor(() => expect(loads).toBe(2));
  expect(within(inspector).getByText("0 / 4000 chars")).toBeVisible();
});

test("shows the backend reason when a prompt cannot be persisted and restores the draft", async () => {
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        appendConsoleMessage: async () => ({
          kind: "message-rejected",
          code: "scope-mismatch",
          message: "Conversation Scope changed before the prompt was saved.",
        }),
      }}
    />,
  );

  const composer = await screen.findByRole("textbox", { name: "Message Alfredo" });
  fireEvent.change(composer, { target: { value: "Fix the polling" } });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  expect(
    await screen.findByText(
      /Prompt was not saved: Conversation Scope changed before the prompt was saved/,
    ),
  ).toBeVisible();
  expect(composer).toHaveValue("Fix the polling");
});

test("preserves a newer composer draft when an earlier prompt save is rejected", async () => {
  let resolveAppend!: (
    result: Awaited<ReturnType<NonNullable<WorkspaceClient["appendConsoleMessage"]>>>,
  ) => void;
  const appendPending = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["appendConsoleMessage"]>>>
  >((resolve) => {
    resolveAppend = resolve;
  });
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        appendConsoleMessage: async () => appendPending,
      }}
    />,
  );

  const composer = await screen.findByRole("textbox", { name: "Message Alfredo" });
  fireEvent.change(composer, { target: { value: "First prompt awaiting persistence" } });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
  fireEvent.change(composer, { target: { value: "Newer prompt that must not be lost" } });

  resolveAppend({
    kind: "message-rejected",
    code: "scope-mismatch",
    message: "Conversation Scope changed before the first prompt was saved.",
  });

  expect(
    await screen.findByText(
      /Prompt was not saved: Conversation Scope changed before the first prompt was saved/,
    ),
  ).toBeVisible();
  expect(composer).toHaveValue("Newer prompt that must not be lost");
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
  const inspector = await openContextInspector();
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

test("runs slash commands through the governed terminal without a model round trip", async () => {
  const terminalRequests: unknown[] = [];
  const generateConsoleResponse = vi.fn();
  const commandClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    appendConsoleMessage: async (request) => ({
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
    }),
    generateConsoleResponse,
    submitShellTerminalCommand: async (request) => {
      terminalRequests.push(request);
      return {
        kind: "command-result",
        result: {
          command_id: "terminal-command-000001",
          correlation_id: request.correlation_id,
          classification: "auto-allowed",
          status: "completed",
          exit_code: 0,
          stdout: "tests passed",
          stderr: "",
        },
      };
    },
  };
  render(<App client={commandClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  const composer = screen.getByRole("textbox", { name: "Message Alfredo" });

  fireEvent.change(composer, { target: { value: "/run npm test -- --run" } });
  fireEvent.keyDown(composer, { key: "Enter", shiftKey: false });

  await waitFor(() => expect(terminalRequests).toHaveLength(1));
  expect(terminalRequests[0]).toMatchObject({
    command: "npm test -- --run",
    working_directory: "/workspace/albert",
    requester: "mission-commander",
  });
  expect(generateConsoleResponse).not.toHaveBeenCalled();
  expect(await screen.findByText(/Captured 1 stdout line/)).toBeVisible();
  fireEvent.click(
    screen.getByRole("button", { name: "Inspect full output for terminal-command-000001" }),
  );
  expect(screen.getByText("tests passed")).toBeVisible();
});

test("runs deterministic slash help without waiting for controller capability authority", async () => {
  const capabilityPending = new Promise<never>(() => {});
  const generateConsoleResponse = vi.fn(async (
    request: Parameters<NonNullable<WorkspaceClient["generateConsoleResponse"]>>[0],
  ) => ({
    kind: "message" as const,
    message: {
      message_id: "console-help-response-000002",
      sequence: 2,
      role: "assistant" as const,
      content: "Available Alfredo commands: /help, /skills, /status, /run, /task, /use",
      scope: snapshot.conversation_scope,
      outcome: "model-commentary" as const,
      source: "frontier-model" as const,
    },
    route: { intent: "discussion" as const, task_request: "", acceptance_criteria: [] },
  }));
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadAgentCapabilities: async () => capabilityPending,
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: {
            schema_version: 1,
            selected_agent: "qwen3-14b",
            selected_model: "qwen3:14b",
            starting_location: "/workspace",
            coding_workspace: "/workspace/albert",
            active_mission: "command-deck",
            phase: "workspace-ready",
            runtime_root: "/home/mission/.alfredo/runtime",
            recent_workspaces: [],
          },
        }),
        appendConsoleMessage: async (request) => ({
          kind: "message",
          message: {
            message_id: "console-help-prompt-000001",
            sequence: 1,
            role: "user",
            content: request.content,
            scope: snapshot.conversation_scope,
            outcome: "proposed",
            source: "mission-commander",
          },
        }),
        generateConsoleResponse,
      }}
    />,
  );
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "/help" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  expect(await screen.findByText(/Available Alfredo commands/)).toBeVisible();
  expect(generateConsoleResponse).toHaveBeenCalledTimes(1);
  expect(generateConsoleResponse.mock.calls[0][0]).toMatchObject({
    message_id: "console-help-prompt-000001",
    agent_id: undefined,
  });
});

test("shows a generic governed /run rejection in the default Agent Console chronology", async () => {
  const generateConsoleResponse = vi.fn();
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadShellTerminal: async () => ({
          kind: "shell-terminal",
          projection: { schema_version: 1, revision: 1, commands: [], grants: [] },
        }),
        appendConsoleMessage: async (request) => ({
          kind: "message",
          message: {
            message_id: "console-run-rejected-1",
            sequence: 1,
            role: "user",
            content: request.content,
            scope: snapshot.conversation_scope,
            outcome: "proposed",
            source: "mission-commander",
          },
        }),
        generateConsoleResponse,
        submitShellTerminalCommand: async () => ({
          kind: "command-rejected",
          code: "sandbox-unavailable",
          message: "Bubblewrap sandbox is unavailable. Install bwrap and retry the command.",
        }),
      }}
    />,
  );
  const composer = await screen.findByRole("textbox", { name: "Message Alfredo" });

  fireEvent.change(composer, { target: { value: "/run npm test -- --run" } });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  const transcript = screen.getByRole("region", { name: "Prompt Transcript" });
  expect(
    await within(transcript).findByText(
      "Shell Terminal command rejected: Bubblewrap sandbox is unavailable. Install bwrap and retry the command.",
    ),
  ).toBeVisible();
  expect(screen.getByRole("button", { name: "Open command audit" })).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  expect(generateConsoleResponse).not.toHaveBeenCalled();
});

test("restores a transport-failed workstation outcome after desktop refresh", async () => {
  let sequence = 0;
  const transportUnavailableClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    appendConsoleMessage: async (request) => {
      sequence += 1;
      return {
        kind: "message",
        message: {
          message_id: `console-transport-failure-${sequence}`,
          sequence,
          role: "user",
          content: request.content,
          scope: snapshot.conversation_scope,
          outcome: "proposed",
          source: "mission-commander",
        },
      };
    },
  };
  const first = render(<App client={transportUnavailableClient} />);
  const composer = await screen.findByRole("textbox", { name: "Message Alfredo" });
  fireEvent.change(composer, { target: { value: "/run npm test -- --run" } });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  const failure =
    "Shell Terminal command rejected: Shell Terminal command transport is unavailable.";
  expect(await screen.findByText(failure)).toBeVisible();
  const continuityKey = Object.keys(window.localStorage).find((key) =>
    key.startsWith("alfredo:failed-workstation-actions:v1:"),
  );
  expect(continuityKey).toBeDefined();
  expect(window.localStorage.getItem(continuityKey!)).toContain(failure);
  first.unmount();

  render(<App client={transportUnavailableClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  expect(window.localStorage.getItem(continuityKey!)).toContain(failure);
  expect(await screen.findByText(failure)).toBeVisible();
});

test("renders chat, workstation actions, and commands in one chronological transcript", async () => {
  let sequence = 0;
  let resolveProposal!: (
    result: Awaited<
      ReturnType<NonNullable<WorkspaceClient["submitAdHocDelegationProposal"]>>
    >,
  ) => void;
  const proposal = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["submitAdHocDelegationProposal"]>>>
  >((resolve) => {
    resolveProposal = resolve;
  });
  const timelineClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadAgentCapabilities: async () => ({
      kind: "capabilities",
      catalog: {
        schema_version: 1,
        default_agent_id: "qwen3-14b",
        commands: [],
        skills: [],
        agents: [
          {
            id: "qwen3-14b",
            role: "frontier",
            provider: "ollama",
            runner: "ollama",
            model: "qwen3:14b",
            routing: "controller",
            availability: "available",
            availability_reason: "",
            assignable: false,
            delegate_only: false,
            requires_approval: false,
          },
          {
            id: "gemma4-12b",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "gemma4:12b",
            routing: "worker",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: false,
            requires_approval: false,
          },
        ],
      },
    }),
    appendConsoleMessage: async (request) => {
      sequence += 1;
      return {
        kind: "message",
        message: {
          message_id: `console-${String(sequence).padStart(6, "0")}`,
          sequence,
          role: request.role,
          content: request.content,
          scope: snapshot.conversation_scope,
          outcome: request.outcome,
          source: request.source,
        },
      };
    },
    generateConsoleResponse: async () => {
      sequence += 1;
      return {
        kind: "message",
        message: {
          message_id: `console-${String(sequence).padStart(6, "0")}`,
          sequence,
          role: "assistant",
          content: "The earlier task is still awaiting approval.",
          scope: snapshot.conversation_scope,
          outcome: "model-commentary",
          source: "frontier-model",
        },
        route: { intent: "discussion", task_request: "", acceptance_criteria: [] },
      };
    },
    submitShellTerminalCommand: async (request) => ({
      kind: "command-result",
      result: {
        command_id: "terminal-command-timeline",
        correlation_id: request.correlation_id,
        classification: "auto-allowed",
        status: "completed",
        exit_code: 0,
        stdout: "first command complete",
        stderr: "",
      },
    }),
    submitAdHocDelegationProposal: async () => proposal,
  };

  render(<App client={timelineClient} />);
  const composer = await screen.findByRole("textbox", { name: "Message Alfredo" });
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Browse commands and skills" })).toBeEnabled(),
  );

  fireEvent.change(composer, { target: { value: "/run printf first" } });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
  await screen.findByText(/Captured 1 stdout line/);

  fireEvent.change(composer, { target: { value: "/task Fix chronological rendering" } });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
  await screen.findByText(/requested Propose coding task: Fix chronological rendering/);

  await act(async () => {
    resolveProposal({
      kind: "acknowledged",
      acknowledgement: {
        correlation_id: "chat-task-console-000002-4",
        outcome: "acknowledged",
        revision: 1,
        item_id: "ad-hoc-command-deck-timeline",
        item_status: "pending",
        effect_summary: "Coding task is pending approval.",
        session_id: null,
      },
    });
  });
  expect(
    await screen.findByText(/accepted workstation action: Coding task is pending approval/),
  ).toBeVisible();
  fireEvent.change(composer, { target: { value: "What is the task status?" } });
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Send prompt" })).toBeEnabled(),
  );

  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
  await screen.findByText("The earlier task is still awaiting approval.");

  const transcript = screen.getByRole("region", { name: "Prompt Transcript" });
  const runTurn = within(transcript).getByText("/run printf first").closest("[data-timeline-key]");
  const commandTurn = within(transcript).getByText(/Captured 1 stdout line/).closest("[data-timeline-key]");
  const taskTurn = within(transcript).getByText("/task Fix chronological rendering").closest("[data-timeline-key]");
  const actionTurn = within(transcript)
    .getByText(/requested Propose coding task: Fix chronological rendering/)
    .closest("[data-timeline-key]");
  const laterChatTurn = within(transcript).getByText("What is the task status?").closest("[data-timeline-key]");
  for (const turn of [runTurn, commandTurn, taskTurn, actionTurn, laterChatTurn]) {
    expect(turn).not.toBeNull();
  }
  expect(runTurn!.compareDocumentPosition(commandTurn!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(commandTurn!.compareDocumentPosition(taskTurn!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(taskTurn!.compareDocumentPosition(actionTurn!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(actionTurn!.compareDocumentPosition(laterChatTurn!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(new Set(
    [...transcript.querySelectorAll<HTMLElement>("[data-timeline-key]")].map(
      (turn) => turn.dataset.timelineKey,
    ),
  ).size).toBe(transcript.querySelectorAll("[data-timeline-key]").length);

});

test("restores durable delegation milestones beside their originating controller turn before later chat", async () => {
  const orderedSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ADHOC-000042",
            issue_id: "ADHOC-000042",
            assigned_agent: "gemma4-12b",
            status: "queued",
            role: "local-agent",
            provider: "ollama",
            model: "gemma4:12b",
          },
        ],
        attention: [],
      },
    ],
  };
  const queueProjection: WorkspaceQueueProjection = {
    schema_version: 1,
    revision: 7,
    items: [
      {
        item_id: "ad-hoc-delegation-command-deck-000042",
        mission_id: "command-deck",
        item_type: "ad-hoc-delegation",
        status: "approved",
        source: "agent-console",
        requested_action: "Delegate the polling investigation",
        affected_boundary: "ad-hoc-delegation",
        consequence: "Approval queues one bounded Local Agent session.",
        issue_id: "ADHOC-000042",
        proposal_correlation_id: "proposal-order-42",
        decision_correlation_id: "approval-order-42",
        proposed_changes: {
          originating_message_id: "console-order-origin",
        },
      },
    ],
    groups: [],
  };
  const orderedClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot: orderedSnapshot }),
    loadConsoleHistory: async () => ({
      kind: "history",
      history: {
        schema_version: 1,
        messages: [
          {
            message_id: "console-order-origin",
            sequence: 1,
            role: "user",
            content: "Could you investigate why polling sometimes stalls?",
            scope: snapshot.conversation_scope,
            outcome: "proposed",
            source: "mission-commander",
          },
          {
            message_id: "console-order-controller",
            sequence: 2,
            role: "assistant",
            content: "I will route that investigation to a governed Local Agent.",
            scope: snapshot.conversation_scope,
            outcome: "model-commentary",
            source: "frontier-model",
          },
          {
            message_id: "console-order-later-user",
            sequence: 3,
            role: "user",
            content: "While that runs, explain the documentation layout.",
            scope: snapshot.conversation_scope,
            outcome: "proposed",
            source: "mission-commander",
          },
          {
            message_id: "console-order-later-assistant",
            sequence: 4,
            role: "assistant",
            content: "The project documentation is organized under .agent.",
            scope: snapshot.conversation_scope,
            outcome: "model-commentary",
            source: "frontier-model",
          },
        ],
      },
    }),
    loadWorkspaceQueue: async () => ({
      kind: "workspace-queue",
      projection: queueProjection,
    }),
  };

  render(<App client={orderedClient} />);

  const transcript = await screen.findByRole("region", { name: "Prompt Transcript" });
  const origin = await within(transcript).findByText(
    "Could you investigate why polling sometimes stalls?",
  );
  const controller = within(transcript).getByText(
    "I will route that investigation to a governed Local Agent.",
  );
  const proposal = await within(transcript).findByText(
    "Coding task proposal ADHOC-000042 was recorded from Agent Console.",
  );
  const decision = within(transcript).getByText(
    "Mission Commander approved coding task ADHOC-000042.",
  );
  const queued = within(transcript).getByText(
    "Orchestrator queued coding task ADHOC-000042 as session-ADHOC-000042 on gemma4-12b.",
  );
  expect(within(proposal.closest("article")!).getByText("Receipt proposal-order-42 · proposal")).toBeVisible();
  expect(within(decision.closest("article")!).getByText("Receipt approval-order-42 · decision")).toBeVisible();
  expect(within(queued.closest("article")!).getByText("Receipt approval-order-42 · session-queued")).toBeVisible();
  const laterChat = within(transcript).getByText(
    "While that runs, explain the documentation layout.",
  );
  const turns = [origin, controller, proposal, decision, queued, laterChat].map((node) =>
    node.closest("[data-timeline-key]"),
  );
  expect(turns.every(Boolean)).toBe(true);
  for (let index = 0; index < turns.length - 1; index += 1) {
    expect(
      turns[index]!.compareDocumentPosition(turns[index + 1]!) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  }
});

test("suppresses legacy Queue effect claims until exact receipt identities are available", async () => {
  const legacySnapshot: WorkspaceSnapshot = {
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ADHOC-000043",
            issue_id: "ADHOC-000043",
            assigned_agent: "gemma4-12b",
            status: "queued",
          },
        ],
        attention: [
          {
            attention_id: "ad-hoc-delegation-command-deck-000043",
            mission_id: "command-deck",
            kind: "ad-hoc-delegation",
            label: "ADHOC-000043 Ad Hoc Delegation pending",
            queue_link: "workspace-queue#ad-hoc-delegation-command-deck-000043",
            entity_id: "ADHOC-000043",
            queue_item_id: "ad-hoc-delegation-command-deck-000043",
          },
        ],
      },
    ],
  };
  const legacyQueue: WorkspaceQueueProjection = {
    schema_version: 1,
    revision: 8,
    items: [
      {
        item_id: "ad-hoc-delegation-command-deck-000043",
        mission_id: "command-deck",
        item_type: "ad-hoc-delegation",
        status: "approved",
        source: "agent-console",
        requested_action: "Delegate legacy work",
        affected_boundary: "ad-hoc-delegation",
        consequence: "Approval queues one bounded Local Agent session.",
        issue_id: "ADHOC-000043",
        proposed_changes: {
          originating_message_id: "console-legacy-origin",
        },
      },
    ],
    groups: [],
  };
  const legacyClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot: legacySnapshot }),
    loadConsoleHistory: async () => ({
      kind: "history",
      history: { schema_version: 1, messages: [] },
    }),
    loadWorkspaceQueue: async () => ({
      kind: "workspace-queue",
      projection: legacyQueue,
    }),
  };

  render(<App client={legacyClient} />);

  const transcript = await screen.findByRole("region", { name: "Prompt Transcript" });
  await waitFor(() => expect(screen.getByText("STATE / 0004")).toBeVisible());
  expect(
    within(transcript).queryByText(
      "Coding task proposal ADHOC-000043 was recorded from Agent Console.",
    ),
  ).not.toBeInTheDocument();
  expect(
    within(transcript).queryByText(
      "Mission Commander approved coding task ADHOC-000043.",
    ),
  ).not.toBeInTheDocument();
  expect(
    within(transcript).queryByText(
      "Orchestrator queued coding task ADHOC-000043 as session-ADHOC-000043 on gemma4-12b.",
    ),
  ).not.toBeInTheDocument();
  expect(
    within(transcript).queryByText(
      "Workstation action pending: ADHOC-000043 Ad Hoc Delegation pending.",
    ),
  ).not.toBeInTheDocument();
});

test("renders running evidence Review Decision and accepted completion as distinct receipt-bound milestones", async () => {
  const lifecycleSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 3,
        is_active: true,
        sessions: [
          {
            session_id: "session-ADHOC-000042",
            issue_id: "ADHOC-000042",
            assigned_agent: "gemma4-12b",
            status: "reviewed",
            runner_started_at: "2026-08-02T10:00:00Z",
            launch_correlation_id: "approval-order-42",
            evidence_correlation_id: "evidence:command-deck:session-ADHOC-000042",
            review_correlation_id: "review-order-42",
            review_outcome: "Approved",
            review_next_action: "complete",
          },
        ],
        attention: [],
      },
    ],
  };
  const lifecycleClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot: lifecycleSnapshot }),
    loadConsoleHistory: async () => ({
      kind: "history",
      history: { schema_version: 1, messages: [] },
    }),
  };

  render(<App client={lifecycleClient} />);

  const transcript = await screen.findByRole("region", { name: "Prompt Transcript" });
  const running = await within(transcript).findByText(
    "Orchestrator started canonical session session-ADHOC-000042 for ADHOC-000042 on gemma4-12b.",
  );
  const evidence = within(transcript).getByText(
    "Local Agent gemma4-12b submitted validated evidence for session-ADHOC-000042.",
  );
  const decision = within(transcript).getByText(
    "Review Decision: Approved for session-ADHOC-000042.",
  );
  const completion = within(transcript).getByText(
    "Accepted completion: session-ADHOC-000042 is complete.",
  );
  expect(within(running.closest("article")!).getByText("Capability: Orchestrator")).toBeVisible();
  expect(within(evidence.closest("article")!).getByText("Capability: Local Agent")).toBeVisible();
  expect(within(running.closest("article")!).getByText("Receipt approval-order-42 · running")).toBeVisible();
  expect(within(evidence.closest("article")!).getByText("Receipt evidence:command-deck:session-ADHOC-000042 · evidence")).toBeVisible();
  expect(within(decision.closest("article")!).getByText("Receipt review-order-42 · review-decision")).toBeVisible();
  expect(within(completion.closest("article")!).getByText("Receipt review-order-42 · accepted-completion")).toBeVisible();
  const turns = [running, evidence, decision, completion].map((node) =>
    node.closest("[data-timeline-key]"),
  );
  for (let index = 0; index < turns.length - 1; index += 1) {
    expect(
      turns[index]!.compareDocumentPosition(turns[index + 1]!) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  }
  expect(within(transcript).queryByText(/Workstation outcome:/)).not.toBeInTheDocument();
});

test("auto-follows near the transcript end without yanking a reader from older turns", async () => {
  let resolveAppend!: (
    result: Awaited<ReturnType<NonNullable<WorkspaceClient["appendConsoleMessage"]>>>,
  ) => void;
  let resolveResponse!: (
    result: Awaited<ReturnType<NonNullable<WorkspaceClient["generateConsoleResponse"]>>>,
  ) => void;
  const appendPending = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["appendConsoleMessage"]>>>
  >((resolve) => {
    resolveAppend = resolve;
  });
  const responsePending = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["generateConsoleResponse"]>>>
  >((resolve) => {
    resolveResponse = resolve;
  });
  render(
    <App
      client={{
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
                content: "Older project discussion",
                scope: snapshot.conversation_scope,
                outcome: "model-commentary",
                source: "frontier-model",
              },
            ],
          },
        }),
        appendConsoleMessage: async () => appendPending,
        generateConsoleResponse: async () => responsePending,
      }}
    />,
  );
  const transcript = await screen.findByRole("region", { name: "Prompt Transcript" });
  Object.defineProperties(transcript, {
    scrollHeight: { configurable: true, get: () => 1000 },
    clientHeight: { configurable: true, get: () => 200 },
    scrollTop: { configurable: true, writable: true, value: 120 },
  });
  fireEvent.scroll(transcript);

  const composer = screen.getByRole("textbox", { name: "Message Alfredo" });
  fireEvent.change(composer, { target: { value: "Explain the current architecture" } });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
  expect(await screen.findByText("Explain the current architecture")).toBeVisible();
  expect(transcript.scrollTop).toBe(1000);

  transcript.scrollTop = 790;
  fireEvent.scroll(transcript);
  await act(async () => {
    resolveAppend({
      kind: "message",
      message: {
        message_id: "console-000002",
        sequence: 2,
        role: "user",
        content: "Explain the current architecture",
        scope: snapshot.conversation_scope,
        outcome: "proposed",
        source: "mission-commander",
      },
    });
  });
  expect(await screen.findByText("Alfredo is working…")).toBeVisible();
  expect(transcript.scrollTop).toBe(1000);

  transcript.scrollTop = 120;
  fireEvent.scroll(transcript);
  await act(async () => {
    resolveResponse({
      kind: "message",
      message: {
        message_id: "console-000003",
        sequence: 3,
        role: "assistant",
        content: "Architecture response arrived",
        scope: snapshot.conversation_scope,
        outcome: "model-commentary",
        source: "frontier-model",
      },
      route: { intent: "discussion", task_request: "", acceptance_criteria: [] },
    });
  });
  expect(await screen.findByText("Architecture response arrived")).toBeVisible();
  expect(transcript.scrollTop).toBe(120);
});

test("turns /task into an approval-gated Local Agent proposal", async () => {
  const proposals: unknown[] = [];
  const taskClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadAgentCapabilities: async () => ({
      kind: "capabilities",
      catalog: {
        schema_version: 1,
        default_agent_id: "qwen3-14b",
        commands: [],
        skills: [],
        agents: [
          {
            id: "qwen3-14b",
            role: "frontier",
            provider: "ollama",
            runner: "ollama",
            model: "qwen3:14b",
            routing: "controller",
            availability: "available",
            availability_reason: "",
            assignable: false,
            delegate_only: false,
            requires_approval: false,
          },
          {
            id: "gemma4-12b",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "gemma4:12b",
            routing: "worker",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: false,
            requires_approval: false,
          },
        ],
      },
    }),
    appendConsoleMessage: async (request) => ({
      kind: "message",
      message: {
        message_id: "console-000123",
        sequence: 1,
        role: "user",
        content: request.content,
        scope: snapshot.conversation_scope,
        outcome: "proposed",
        source: "mission-commander",
      },
    }),
    submitAdHocDelegationProposal: async (request) => {
      proposals.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 1,
          item_id: "ad-hoc-command-deck-000001",
          item_status: "pending",
          effect_summary: "Ad Hoc Delegation ADHOC-000001 is pending approval.",
          session_id: null,
        },
      };
    },
  };
  render(<App client={taskClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "/task Fix the failing workspace test" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  await waitFor(() => expect(proposals).toHaveLength(1));
  expect(proposals[0]).toMatchObject({
    acceptance_criteria: ["Fix the failing workspace test"],
    allowed_paths: ["/workspace/albert"],
    proposed_agent: "gemma4-12b",
    originating_message_id: "console-000123",
  });
  expect(await screen.findByText(/pending approval/)).toBeVisible();
});

test("turns /use skill work into an approval-gated proposal without controller inference", async () => {
  const proposals: unknown[] = [];
  const generateConsoleResponse = vi.fn();
  const runWorkstationSession = vi.fn();
  const skillClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadAgentCapabilities: async () => ({
      kind: "capabilities",
      catalog: {
        schema_version: 1,
        default_agent_id: "qwen3-14b",
        commands: [],
        skills: [
          {
            name: "diagnosing-bugs",
            description: "Diagnose difficult bugs before changing code.",
            invocation: "/use diagnosing-bugs",
            source: "/skills/diagnosing-bugs/SKILL.md",
          },
        ],
        agents: [
          {
            id: "qwen3-14b",
            role: "frontier",
            provider: "ollama",
            runner: "ollama",
            model: "qwen3:14b",
            routing: "controller",
            availability: "available",
            availability_reason: "",
            assignable: false,
            delegate_only: false,
            requires_approval: false,
          },
          {
            id: "gemma4-12b",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "gemma4:12b",
            routing: "worker",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: false,
            requires_approval: false,
          },
        ],
      },
    }),
    appendConsoleMessage: async (request) => ({
      kind: "message",
      message: {
        message_id: "console-use-000001",
        sequence: 1,
        role: "user",
        content: request.content,
        scope: snapshot.conversation_scope,
        outcome: "proposed",
        source: "mission-commander",
      },
    }),
    generateConsoleResponse,
    submitAdHocDelegationProposal: async (request) => {
      proposals.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 1,
          item_id: "ad-hoc-command-deck-000002",
          item_status: "pending",
          effect_summary: "Ad Hoc Delegation ADHOC-000002 is pending approval.",
          session_id: null,
        },
      };
    },
    runWorkstationSession,
  };
  render(<App client={skillClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "/use diagnosing-bugs Fix the intermittent workspace restore failure" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  await waitFor(() => expect(proposals).toHaveLength(1));
  expect(proposals[0]).toMatchObject({
    acceptance_criteria: [
      "/use diagnosing-bugs Fix the intermittent workspace restore failure",
    ],
    allowed_paths: ["/workspace/albert"],
    proposed_agent: "gemma4-12b",
    originating_message_id: "console-use-000001",
  });
  expect(generateConsoleResponse).not.toHaveBeenCalled();
  expect(runWorkstationSession).not.toHaveBeenCalled();
  expect(
    await screen.findByText(
      /Workstation action: Mission Commander requested Propose coding task with skill diagnosing-bugs/,
    ),
  ).toBeVisible();
  expect(
    await screen.findByText(/Orchestrator accepted workstation action: .*pending approval/),
  ).toBeVisible();
});

test("rejects an unknown /use skill before creating an approvable task", async () => {
  const submitAdHocDelegationProposal = vi.fn();
  const generateConsoleResponse = vi.fn();
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadAgentCapabilities: async () => ({
          kind: "capabilities",
          catalog: {
            schema_version: 1,
            default_agent_id: "qwen3-14b",
            commands: [],
            skills: [
              {
                name: "diagnosing-bugs",
                description: "Diagnose difficult bugs.",
                invocation: "/use diagnosing-bugs",
                source: "/skills/diagnosing-bugs/SKILL.md",
              },
            ],
            agents: [],
          },
        }),
        appendConsoleMessage: async (request) => ({
          kind: "message",
          message: {
            message_id: "console-use-unknown-1",
            sequence: 1,
            role: "user",
            content: request.content,
            scope: snapshot.conversation_scope,
            outcome: "proposed",
            source: "mission-commander",
          },
        }),
        submitAdHocDelegationProposal,
        generateConsoleResponse,
      }}
    />,
  );
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Browse commands and skills" })).toBeEnabled(),
  );

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "/use typo-skill Fix the workspace" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  expect(
    await screen.findByText("Unknown skill typo-skill. Use /skills to choose an installed skill."),
  ).toBeVisible();
  expect(submitAdHocDelegationProposal).not.toHaveBeenCalled();
  expect(generateConsoleResponse).not.toHaveBeenCalled();
});

test("routes explicit remediation and subagent requests to an ungated worker", async () => {
  const proposals: unknown[] = [];
  const generateConsoleResponse = vi.fn();
  let appendedMessages = 0;
  const taskClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadAgentCapabilities: async () => ({
      kind: "capabilities",
      catalog: {
        schema_version: 1,
        default_agent_id: "qwen3-14b",
        commands: [],
        skills: [],
        agents: [
          {
            id: "qwen3-14b",
            role: "frontier",
            provider: "ollama",
            runner: "ollama",
            model: "qwen3:14b",
            routing: "controller",
            availability: "available",
            availability_reason: "",
            assignable: false,
            delegate_only: false,
            requires_approval: false,
          },
          {
            id: "deepseek-delegate",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "deepseek-r1:14b",
            routing: "worker",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: true,
            requires_approval: true,
          },
          {
            id: "gated-worker",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "gated-worker:14b",
            routing: "worker",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: false,
            requires_approval: true,
          },
          {
            id: "cloud-worker",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "cloud-worker:cloud",
            routing: "worker",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: false,
            requires_approval: false,
          },
          {
            id: "frontier-routed-worker",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "frontier-routed-worker:14b",
            routing: "frontier",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: false,
            requires_approval: false,
          },
          {
            id: "gemma4-12b",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "gemma4:12b",
            routing: "worker",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: false,
            requires_approval: false,
          },
        ],
      },
    }),
    appendConsoleMessage: async (request) => {
      appendedMessages += 1;
      return {
        kind: "message",
        message: {
          message_id: `console-natural-task-${String(appendedMessages).padStart(6, "0")}`,
          sequence: appendedMessages,
          role: "user",
          content: request.content,
          scope: snapshot.conversation_scope,
          outcome: "proposed",
          source: "mission-commander",
        },
      };
    },
    generateConsoleResponse,
    submitAdHocDelegationProposal: async (request) => {
      proposals.push(request);
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 1,
          item_id: "ad-hoc-command-deck-000003",
          item_status: "pending",
          effect_summary: "Ad Hoc Delegation ADHOC-000003 is pending approval.",
          session_id: null,
        },
      };
    },
  };
  render(<App client={taskClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Ask a local agent to make the Issue Assignment Board polling reliable" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  await waitFor(() => expect(proposals).toHaveLength(1));
  expect(proposals[0]).toMatchObject({
    acceptance_criteria: ["Ask a local agent to make the Issue Assignment Board polling reliable"],
    proposed_agent: "gemma4-12b",
    originating_message_id: "console-natural-task-000001",
  });
  expect(generateConsoleResponse).not.toHaveBeenCalled();
  expect(
    await screen.findByText(
      /Workstation action: Mission Commander requested Propose coding task: Ask a local agent to make the Issue Assignment Board polling reliable/,
    ),
  ).toBeVisible();

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Tell a subagent to test and fix the Issue Assignment Board polling recovery" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
  await waitFor(() => expect(proposals).toHaveLength(2));
  expect(proposals[1]).toMatchObject({
    acceptance_criteria: ["Tell a subagent to test and fix the Issue Assignment Board polling recovery"],
    proposed_agent: "gemma4-12b",
  });

  const explicitWorkPrompts = [
    "Ask a subagent to fix the polling",
    "Have a local agent repair the broken tests",
    "Tell the coding agent to handle the broken polling",
    "Call a subagent to handle this",
    "Use a local agent to patch the broken tests",
    "Send a coding agent to work on the failing tests",
    "Delegate to a subagent to take care of this issue",
    "Get a local agent to fix the polling",
    "Spin up a subagent to resolve the bug",
    "Please fix the release seam polling with a subagent",
    "Investigate why polling stalls with a subagent!!!",
    "Ask a local agent to inspect and repair the failing layout",
    "Please ask a subagent to fix the polling",
    "Please have a local agent repair the broken tests",
    "Please delegate to a subagent to investigate the polling failure",
  ];
  for (const [index, prompt] of explicitWorkPrompts.entries()) {
    fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
      target: { value: prompt },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    await waitFor(() => expect(proposals).toHaveLength(index + 3));
    expect(proposals[index + 2]).toMatchObject({
      acceptance_criteria: [prompt],
      proposed_agent: "gemma4-12b",
    });
  }
  const explicitRemediationPrompts = [
    "Please fix the failing tests",
    "Can you repair the broken build?",
    "Implement the polling feature",
    "Refactor the frontend component",
    "Patch the bug",
  ];
  for (const [index, prompt] of explicitRemediationPrompts.entries()) {
    fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
      target: { value: prompt },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    await waitFor(() => expect(proposals).toHaveLength(index + 18));
    expect(proposals[index + 17]).toMatchObject({
      acceptance_criteria: [prompt],
      proposed_agent: "gemma4-12b",
    });
  }
  expect(generateConsoleResponse).not.toHaveBeenCalled();
});

test("requires the canonical delegation Mission, scope, and goal to match the authorized prompt", () => {
  const request: AdHocDelegationProposalRequest = {
    correlation_id: "exact-boundary-1",
    expected_revision: 4,
    source: "agent-console",
    scope_kind: "issue-slice",
    scope_target: "ISS-01",
    scope_label: "Restore workspace session",
    mission_id: "command-deck",
    acceptance_criteria: ["Fix the polling"],
    allowed_paths: ["/workspace/albert"],
    command_policy: {},
    proposed_agent: "gemma4-12b",
    originating_message_id: "console-exact-1",
  };
  const item: WorkspaceQueueItem = {
    item_id: "ad-hoc-command-deck-exact-1",
    mission_id: "command-deck",
    item_type: "ad-hoc-delegation",
    status: "pending",
    source: "agent-console",
    requested_action: "Approve Ad Hoc Delegation",
    affected_boundary: "ad-hoc-delegation",
    consequence: "Approval launches bounded work.",
    issue_id: "ADHOC-000001",
    proposed_changes: {
      scope: {
        kind: "issue-slice",
        target_id: "ISS-01",
        label: "Restore workspace session",
        mission_id: "command-deck",
      },
      acceptance_criteria: ["Fix the polling"],
      allowed_paths: ["/workspace/albert"],
      command_policy: {},
      proposed_agent: "gemma4-12b",
      originating_message_id: "console-exact-1",
      goal: "Please fix the polling",
    },
  };

  expect(isExactAdHocDelegationBoundary(item, request, "Please fix the polling")).toBe(true);
  expect(isExactAdHocDelegationBoundary(
    { ...item, mission_id: "background-mission" },
    request,
    "Please fix the polling",
  )).toBe(false);
  expect(isExactAdHocDelegationBoundary(
    {
      ...item,
      proposed_changes: {
        ...item.proposed_changes,
        scope: {
          ...(item.proposed_changes.scope as Record<string, unknown>),
          mission_id: "background-mission",
        },
      },
    },
    request,
    "Please fix the polling",
  )).toBe(false);
  expect(isExactAdHocDelegationBoundary(item, request, "Altered goal")).toBe(false);

  const workingDirectoryRequest: AdHocDelegationProposalRequest = {
    ...request,
    scope_kind: "working-directory",
    scope_target: "/workspace/albert",
    scope_label: "albert",
  };
  const workingDirectoryItem: WorkspaceQueueItem = {
    ...item,
    proposed_changes: {
      ...item.proposed_changes,
      scope: {
        kind: "working-directory",
        target_id: "/workspace/albert",
        label: "albert",
        mission_id: null,
      },
    },
  };
  expect(isExactAdHocDelegationBoundary(
    workingDirectoryItem,
    workingDirectoryRequest,
    "Please fix the polling",
  )).toBe(true);
});

test("waits for the in-flight capability catalog before delegating an early coding prompt", async () => {
  let resolveCapabilities!: (
    result: Awaited<ReturnType<NonNullable<WorkspaceClient["loadAgentCapabilities"]>>>,
  ) => void;
  const capabilityPending = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["loadAgentCapabilities"]>>>
  >((resolve) => {
    resolveCapabilities = resolve;
  });
  const proposals: unknown[] = [];
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadAgentCapabilities: async () => capabilityPending,
        appendConsoleMessage: async (request) => ({
          kind: "message",
          message: {
            message_id: "console-early-task-000001",
            sequence: 1,
            role: "user",
            content: request.content,
            scope: snapshot.conversation_scope,
            outcome: "proposed",
            source: "mission-commander",
          },
        }),
        submitAdHocDelegationProposal: async (request) => {
          proposals.push(request);
          return {
            kind: "acknowledged",
            acknowledgement: {
              correlation_id: request.correlation_id,
              outcome: "acknowledged",
              revision: 1,
              item_id: "ad-hoc-command-deck-early",
              item_status: "pending",
              effect_summary: "Early task is pending approval.",
              session_id: null,
            },
          };
        },
      }}
    />,
  );
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "/task Fix the early capability race" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
  expect(proposals).toHaveLength(0);

  await act(async () => {
    resolveCapabilities({
      kind: "capabilities",
      catalog: {
        schema_version: 1,
        default_agent_id: "qwen3-14b",
        commands: [],
        skills: [],
        agents: [
          {
            id: "qwen3-14b",
            role: "frontier",
            provider: "ollama",
            runner: "ollama",
            model: "qwen3:14b",
            routing: "controller",
            availability: "available",
            availability_reason: "",
            assignable: false,
            delegate_only: false,
            requires_approval: false,
          },
          {
            id: "gemma4-12b",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "gemma4:12b",
            routing: "worker",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: false,
            requires_approval: false,
          },
        ],
      },
    });
  });

  await waitFor(() => expect(proposals).toHaveLength(1));
  expect(proposals[0]).toMatchObject({
    acceptance_criteria: ["Fix the early capability race"],
    proposed_agent: "gemma4-12b",
    originating_message_id: "console-early-task-000001",
  });
  expect(screen.queryByText(/No available assignable non-delegate worker/)).not.toBeInTheDocument();
});

test("automatically approves and dispatches an exactly bounded subagent coding request", async () => {
  const proposals: unknown[] = [];
  const decisions: unknown[] = [];
  const runs: unknown[] = [];
  let exposeExpandedBoundary = false;
  let appendedMessages = 0;
  const generateConsoleResponse = vi.fn();
  let latestProposalRequest: AdHocDelegationProposalRequest | null = null;
  let latestPromptContent = "";
  const itemId = "ad-hoc-delegation-command-deck-000004";
  const requestContent =
    "Can you ask a subagent to fix the broken Issue Assignment Board polling?";
  const taskSnapshot: WorkspaceSnapshot = {
    ...snapshot,
    conversation_scope: {
      kind: "working-directory",
      target_id: "/workspace/albert",
      label: "albert",
    },
  };
  const queueItem: WorkspaceQueueItem = {
    item_id: itemId,
    mission_id: "command-deck",
    item_type: "ad-hoc-delegation",
    status: "pending",
    source: "agent-console",
    requested_action: "Approve Ad Hoc Delegation",
    affected_boundary: "ad-hoc-delegation",
    consequence: "Approval launches bounded coding work.",
    issue_id: "ADHOC-000004",
    proposed_changes: {
      scope: {
        kind: taskSnapshot.conversation_scope.kind,
        target_id: taskSnapshot.conversation_scope.target_id,
        label: taskSnapshot.conversation_scope.label,
        mission_id: null,
      },
      acceptance_criteria: [requestContent],
      allowed_paths: ["/workspace/albert"],
      command_policy: {},
      proposed_agent: "gemma4-12b",
      originating_message_id: "console-natural-task-000004",
      goal: requestContent,
    },
  };
  const taskClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot: taskSnapshot }),
    loadAgentCapabilities: async () => ({
      kind: "capabilities",
      catalog: {
        schema_version: 1,
        default_agent_id: "qwen3-14b",
        commands: [],
        skills: [],
        agents: [
          {
            id: "qwen3-14b",
            role: "frontier",
            provider: "ollama",
            runner: "ollama",
            model: "qwen3:14b",
            routing: "controller",
            availability: "available",
            availability_reason: "",
            assignable: false,
            delegate_only: false,
            requires_approval: false,
          },
          {
            id: "deepseek-delegate",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "deepseek-r1:14b",
            routing: "worker",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: true,
            requires_approval: true,
          },
          {
            id: "gemma4-12b",
            role: "local-agent",
            provider: "ollama",
            runner: "ollama",
            model: "gemma4:12b",
            routing: "worker",
            availability: "available",
            availability_reason: "",
            assignable: true,
            delegate_only: false,
            requires_approval: false,
          },
        ],
      },
    }),
    appendConsoleMessage: async (request) => {
      appendedMessages += 1;
      latestPromptContent = request.content;
      return {
        kind: "message",
        message: {
          message_id: `console-natural-task-${String(appendedMessages + 3).padStart(6, "0")}`,
          sequence: appendedMessages,
          role: "user",
          content: request.content,
          scope: taskSnapshot.conversation_scope,
          outcome: "proposed",
          source: "mission-commander",
        },
      };
    },
    generateConsoleResponse,
    submitAdHocDelegationProposal: async (request) => {
      proposals.push(request);
      latestProposalRequest = request;
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 4,
          item_id: itemId,
          item_status: "pending",
          effect_summary: "Ad Hoc Delegation ADHOC-000004 is pending approval.",
          session_id: null,
        },
      };
    },
    loadWorkspaceQueue: async () => ({
      kind: "workspace-queue",
      projection: {
        schema_version: 1,
        revision: 4,
        items: [
          {
            ...queueItem,
            proposal_correlation_id:
              latestProposalRequest?.correlation_id ??
              "chat-task-console-natural-task-000004-4",
            proposed_changes: {
              ...queueItem.proposed_changes,
              acceptance_criteria:
                latestProposalRequest?.acceptance_criteria ?? queueItem.proposed_changes.acceptance_criteria,
              allowed_paths: exposeExpandedBoundary
                ? ["/workspace/albert", "/etc"]
                : latestProposalRequest?.allowed_paths ?? queueItem.proposed_changes.allowed_paths,
              command_policy:
                latestProposalRequest?.command_policy ?? queueItem.proposed_changes.command_policy,
              proposed_agent:
                latestProposalRequest?.proposed_agent ?? queueItem.proposed_changes.proposed_agent,
              originating_message_id:
                latestProposalRequest?.originating_message_id ??
                queueItem.proposed_changes.originating_message_id,
              goal: latestPromptContent || requestContent,
            },
          },
        ],
        groups: [],
      },
    }),
    submitWorkspaceQueueDecision: async (request) => {
      decisions.push(request);
      const sessionId = decisions.length === 1
        ? "session-ADHOC-000004"
        : `session-ADHOC-000004-${decisions.length}`;
      return {
        kind: "acknowledged",
        acknowledgement: {
          correlation_id: request.correlation_id,
          outcome: "acknowledged",
          revision: 5,
          item_id: itemId,
          item_status: "approved",
          effect_summary: `Approved ADHOC-000004 and queued ${sessionId}.`,
          session_id: sessionId,
        },
      };
    },
    runWorkstationSession: async (request) => {
      runs.push(request);
      return {
        kind: "session-finished",
        session: {
          schema_version: 1,
          mission_id: "command-deck",
          session_id: request.session_id,
          issue_id: "ADHOC-000004",
          status: "evidence-ready",
          runner_started_at: "2026-07-11T08:00:00Z",
          runner_ended_at: "2026-07-11T08:00:01Z",
          runner_exit_status: 0,
          evidence_valid: true,
        },
      };
    },
  };
  render(<App client={taskClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: requestContent },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  await waitFor(() => expect(runs).toEqual([
    { session_id: "session-ADHOC-000004", mission_id: "command-deck" },
  ]));
  expect(proposals).toHaveLength(1);
  expect(decisions).toEqual([
    {
      correlation_id:
        "chat-task-approve-console-natural-task-000004-ad-hoc-delegation-command-deck-000004-4",
      action_type: "workspace-queue-decision",
      actor: "mission-commander",
      expected_revision: 4,
      target: { kind: "workspace-queue-item", id: itemId },
      item_id: itemId,
      decision: "approve",
      reason:
        "Mission Commander explicitly authorized this bounded coding task in Agent Console message console-natural-task-000004.",
    },
  ]);
  expect(generateConsoleResponse).not.toHaveBeenCalled();
  expect(
    await screen.findByText(
      "Coding task proposal ADHOC-000004 was recorded from Agent Console.",
    ),
  ).toBeVisible();
  expect(await screen.findByText(/session-ADHOC-000004 is queued and starting/)).toBeVisible();
  expect(await screen.findByText(/finished with status evidence-ready/)).toBeVisible();

  exposeExpandedBoundary = true;
  const assistedRequest = "Ask a subagent to investigate and fix the broken polling";
  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: assistedRequest },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  expect(
    await screen.findByText(/Automatic approval paused because the canonical proposal no longer exactly matches/),
  ).toBeVisible();
  expect(decisions).toHaveLength(1);
  expect(runs).toHaveLength(1);

  exposeExpandedBoundary = false;
  generateConsoleResponse.mockResolvedValueOnce({
    kind: "message",
    message: {
      message_id: "console-controller-route-000001",
      sequence: 7,
      role: "assistant",
      content: "I will route that implementation request to a governed Local Agent.",
      scope: taskSnapshot.conversation_scope,
      outcome: "model-commentary",
      source: "frontier-model",
    },
    route: {
      intent: "coding-task",
      task_request: "Improve polling reliability",
      acceptance_criteria: ["Polling recovers after transient failures"],
    },
  });
  const controllerDiscoveredRequest = "Let's improve the polling reliability";
  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: controllerDiscoveredRequest },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  await waitFor(() => expect(runs).toHaveLength(2));
  expect(generateConsoleResponse).toHaveBeenCalledTimes(1);
  expect(proposals.at(-1)).toMatchObject({
    acceptance_criteria: ["Polling recovers after transient failures"],
    proposed_agent: "gemma4-12b",
  });
  expect(runs[1]).toEqual({
    session_id: "session-ADHOC-000004-2",
    mission_id: "command-deck",
  });
});

test("keeps discussion in controller chat instead of creating a coding proposal", async () => {
  const messageRequests: AgentConsoleMessageRequest[] = [];
  const responseRequests: Array<{
    expected_revision: number;
    scope_kind: string;
    scope_target: string;
    scope_label: string;
    agent_id?: string;
  }> = [];
  const submitAdHocDelegationProposal = vi.fn();
  const messageClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadLaunchContext: async () => ({
      kind: "launch-context",
      context: {
        schema_version: 1,
        selected_agent: "qwen3.6-27b",
        selected_model: "qwen3.6:27b",
        starting_location: "/workspace",
        coding_workspace: "/workspace/albert",
        active_mission: "command-deck",
        phase: "workspace-ready",
        runtime_root: "/tmp/albert-runtime",
        recent_workspaces: [],
      },
    }),
    loadAgentCapabilities: async () => ({
      kind: "capabilities",
      catalog: {
        schema_version: 1,
        default_agent_id: "qwen3.6-27b",
        commands: [],
        skills: [],
        agents: [
          {
            id: "qwen3.6-27b",
            role: "frontier",
            provider: "ollama",
            runner: "ollama",
            model: "qwen3.6:27b",
            routing: "controller",
            availability: "available",
            availability_reason: "",
            assignable: false,
            delegate_only: false,
            requires_approval: false,
          },
        ],
      },
    }),
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
    generateConsoleResponse: async (request) => {
      responseRequests.push(request);
      return {
        kind: "message",
        message: {
          message_id: "console-000002",
          sequence: 2,
          role: "assistant",
          content: "I can respond as the configured controller.",
          scope: snapshot.conversation_scope,
          outcome: "model-commentary",
          source: "frontier-model",
          action_outcome: "no-action",
          action_message:
            "No action taken. Controller prose is commentary and no correlated Orchestrator receipt exists.",
        },
        route: { intent: "discussion", task_request: "", acceptance_criteria: [] },
      };
    },
    submitAdHocDelegationProposal,
  };
  render(<App client={messageClient} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Please explain how workspace restoration works." },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  expect(await screen.findByText("Please explain how workspace restoration works.")).toBeVisible();
  expect(await screen.findByText("I can respond as the configured controller.")).toBeVisible();
  const noAction = await screen.findByText(
    "No action taken. Controller prose is commentary and no correlated Orchestrator receipt exists.",
  );
  expect(noAction).toBeVisible();
  expect(noAction.closest('[data-authority="commentary"]')).not.toBeNull();
  expect(responseRequests).toEqual([
    {
      expected_revision: 4,
      message_id: "console-000001",
      scope_kind: "issue-slice",
      scope_target: "ISS-01",
      scope_label: "Restore workspace session",
      scope_mission_id: undefined,
      agent_id: "qwen3.6-27b",
    },
  ]);
  expect(messageRequests).toHaveLength(1);
  expect(submitAdHocDelegationProposal).not.toHaveBeenCalled();
});

test("renders exact receipt identity only on correlated canonical Agent Console events", async () => {
  const receiptClient: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    loadConsoleHistory: async () => ({
      kind: "history",
      history: {
        schema_version: 1,
        messages: [
          {
            message_id: "console-receipt-1",
            sequence: 1,
            role: "assistant",
            content: "Orchestrator queued the exact acknowledged session.",
            scope: snapshot.conversation_scope,
            outcome: "acknowledged",
            source: "orchestrator",
            correlation_id: "queue-approval-42",
            action_phase: "session-queued",
          },
          {
            message_id: "console-commentary-2",
            sequence: 2,
            role: "assistant",
            content: "This explanation remains controller commentary.",
            scope: snapshot.conversation_scope,
            outcome: "model-commentary",
            source: "frontier-model",
            action_outcome: "no-action",
            action_message:
              "No action taken. Controller prose is commentary and no correlated Orchestrator receipt exists.",
          },
        ],
      },
    }),
  };

  render(<App client={receiptClient} />);
  const transcript = await screen.findByRole("region", { name: "Prompt Transcript" });

  expect(
    within(transcript).getByText("Receipt queue-approval-42 · session-queued"),
  ).toBeVisible();
  const commentary = within(transcript).getByText(
    "This explanation remains controller commentary.",
  ).closest("article");
  expect(commentary).not.toBeNull();
  expect(within(commentary!).queryByText(/^Receipt /)).not.toBeInTheDocument();
});

test("routes ambiguous check prompts through the controller instead of auto-authorizing work", async () => {
  let appendedMessageCount = 0;
  let generatedMessageCount = 0;
  const submitAdHocDelegationProposal = vi.fn(async (request) => ({
    kind: "acknowledged" as const,
    acknowledgement: {
      correlation_id: request.correlation_id,
      outcome: "acknowledged" as const,
      revision: 1,
      item_id: "unexpected-ambiguous-proposal",
      item_status: "pending" as const,
      effect_summary: "This ambiguous prompt should not have been auto-proposed.",
      session_id: null,
    },
  }));
  const generateConsoleResponse = vi.fn(async () => {
    generatedMessageCount += 1;
    return {
      kind: "message" as const,
      message: {
        message_id: `console-ambiguous-response-${generatedMessageCount}`,
        sequence: generatedMessageCount * 2,
        role: "assistant" as const,
        content: "The design question stays in discussion until you request a concrete change.",
        scope: snapshot.conversation_scope,
        outcome: "model-commentary" as const,
        source: "frontier-model" as const,
      },
      route: { intent: "discussion" as const, task_request: "", acceptance_criteria: [] },
    };
  });
  const catalog = {
    schema_version: 1 as const,
    default_agent_id: "qwen3-14b",
    commands: [],
    skills: [],
    agents: [
      {
        id: "qwen3-14b",
        role: "frontier",
        provider: "ollama",
        runner: "ollama",
        model: "qwen3:14b",
        routing: "controller",
        availability: "available" as const,
        availability_reason: "",
        assignable: false,
        delegate_only: false,
        requires_approval: false,
      },
      {
        id: "gemma4-12b",
        role: "local-agent",
        provider: "ollama",
        runner: "ollama",
        model: "gemma4:12b",
        routing: "worker",
        availability: "available" as const,
        availability_reason: "",
        assignable: true,
        delegate_only: false,
        requires_approval: false,
      },
    ],
  };
  render(
    <App
      client={{
        loadSnapshot: async () => ({ kind: "ready", snapshot }),
        loadAgentCapabilities: async () => ({ kind: "capabilities", catalog }),
        appendConsoleMessage: async (request) => {
          appendedMessageCount += 1;
          return {
            kind: "message",
            message: {
              message_id: `console-ambiguous-prompt-${appendedMessageCount}`,
              sequence: appendedMessageCount * 2 - 1,
              role: "user",
              content: request.content,
              scope: snapshot.conversation_scope,
              outcome: "proposed",
              source: "mission-commander",
            },
          };
        },
        generateConsoleResponse,
        submitAdHocDelegationProposal,
      }}
    />,
  );
  await waitFor(() =>
    expect(screen.getByRole("combobox", { name: "Controller model" })).toHaveValue(
      "qwen3-14b",
    ),
  );

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Check whether the tests are well structured before we change code." },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  expect(
    await screen.findByText(
      "The design question stays in discussion until you request a concrete change.",
    ),
  ).toBeVisible();
  expect(generateConsoleResponse).toHaveBeenCalledTimes(1);
  expect(submitAdHocDelegationProposal).not.toHaveBeenCalled();

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Can you take a look at the tests before we change code?" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
  await waitFor(() => expect(generateConsoleResponse).toHaveBeenCalledTimes(2));
  expect(submitAdHocDelegationProposal).not.toHaveBeenCalled();

  const additionalDiscussionPrompts = [
    "Can you explain why this bug is hard to investigate?",
    "Please write an explanation of how workspace restoration works.",
    "Make sense of this polling architecture for me.",
    "Check whether it is safe to fix the build.",
    "Please ask a subagent to explain how polling works.",
    "Please ask a subagent whether we should fix the polling.",
    "Investigate why polling stalls with a subagent if needed.",
    "Investigate why polling stalls with a subagent-ish helper.",
    "Investigate why polling stalls without a subagent.",
  ];
  for (const [index, prompt] of additionalDiscussionPrompts.entries()) {
    fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
      target: { value: prompt },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    await waitFor(() =>
      expect(generateConsoleResponse).toHaveBeenCalledTimes(index + 3),
    );
  }
  expect(submitAdHocDelegationProposal).not.toHaveBeenCalled();
});

test("echoes the user turn immediately while durable persistence is still pending", async () => {
  let resolveAppend!: (result: Awaited<ReturnType<NonNullable<WorkspaceClient["appendConsoleMessage"]>>>) => void;
  const appendPending = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["appendConsoleMessage"]>>>
  >((resolve) => {
    resolveAppend = resolve;
  });
  const appendConsoleMessage = vi.fn(async () => appendPending);
  const client: WorkspaceClient = {
    loadSnapshot: async () => ({ kind: "ready", snapshot }),
    appendConsoleMessage,
  };
  render(<App client={client} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Inspect the failing layout without freezing the console" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));

  expect(
    within(screen.getByRole("region", { name: "Prompt Transcript" })).getByText(
      "Inspect the failing layout without freezing the console",
    ),
  ).toBeVisible();
  expect(screen.getByRole("status", { name: "Message status" })).toHaveTextContent(/saving/i);

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Second prompt while the first save is pending" },
  });
  fireEvent.keyDown(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    key: "Enter",
    shiftKey: false,
  });
  expect(appendConsoleMessage).toHaveBeenCalledTimes(1);

  resolveAppend({
    kind: "message",
    message: {
      message_id: "console-000001",
      sequence: 1,
      role: "user",
      content: "Inspect the failing layout without freezing the console",
      scope: snapshot.conversation_scope,
      outcome: "proposed",
      source: "mission-commander",
    },
  });
  await waitFor(() =>
    expect(screen.queryByRole("status", { name: "Message status" })).not.toBeInTheDocument(),
  );
});

test("caps pasted prompts at the shared durable Agent Console input boundary", async () => {
  render(<App client={client} />);
  const composer = await screen.findByRole("textbox", { name: "Message Alfredo" });
  expect(composer).toHaveAttribute("maxlength", "16000");
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

  await openContextInspector();
  fireEvent.change(screen.getByRole("combobox", { name: "Conversation Scope" }), {
    target: { value: selection },
  });
  fireEvent.click(screen.getByRole("button", { name: "Apply target" }));

  await waitFor(() => expectConversationScopeValue(selection));
  closeContextInspector();
  expectPromptScope(expectedScope.label);
  expect(scopeRequests).toEqual([
    {
      correlation_id: `conversation-scope-${expectedScope.kind}-${expectedScope.target_id}-4`,
      action_type: "conversation-scope-change",
      actor: "mission-commander",
      expected_revision: 4,
      target: { kind: "conversation-scope", id: expectedScope.target_id },
      scope_kind: expectedScope.kind,
      scope_target: expectedScope.target_id,
      scope_label: expectedScope.label,
    },
  ]);
});
