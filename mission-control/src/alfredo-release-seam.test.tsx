/// <reference types="node" />
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { App } from "./App";
import type { WorkspaceClient } from "./workspace-client";
import type {
  ActivityJournalFilters,
  AlfredoLaunchContext,
  AgentConsoleHistory,
  WorkspaceActionRequest,
  WorkstationActionRequest,
} from "./contracts";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(projectRoot, "..");

function alfredoBinPath() {
  const packageJson = JSON.parse(readFileSync(resolve(projectRoot, "package.json"), "utf8"));
  return resolve(projectRoot, packageJson.bin.alfredo);
}

function writeReleaseTracker(root: string) {
  const workspace = resolve(root, "workspace");
  const tracker = resolve(workspace, ".scratch", "alfredo-agent-workstation");
  const issues = resolve(tracker, "issues");
  const agentConfig = resolve(root, "agents.json");
  mkdirSync(issues, { recursive: true });
  writeFileSync(resolve(tracker, "PRD.md"), "# Alfredo Release Seam\n", "utf8");
  writeFileSync(
    resolve(issues, "01-release-seam.md"),
    [
      "Status: ready-for-agent",
      "Type: AFK",
      "Suggested agent: fake-local",
      "Assigned agent: fake-local",
      "",
      "## Parent",
      "",
      "PRD.md",
      "",
      "## What to build",
      "",
      "Verify the complete Alfredo launch-to-workstation release seam.",
      "",
      "## Acceptance criteria",
      "",
      "- [ ] Release seam remains visible and governed.",
      "",
      "## Blocked by",
      "",
      "None - can start immediately",
      "",
    ].join("\n"),
    "utf8",
  );
  writeFileSync(
    agentConfig,
    `${JSON.stringify(
      {
        agents: [
          {
            id: "fake-controller",
            role: "frontier",
            provider: "test",
            runner: "fake",
            model: "deterministic-controller",
            routing: "controller",
          },
          {
            id: "fake-local",
            role: "local-agent",
            provider: "test",
            runner: "fake",
            model: "deterministic-fake",
          },
        ],
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  return { workspace, tracker, issues, agentConfig };
}

function createBackendClient(options: {
  launchContext: AlfredoLaunchContext;
  workspace: string;
  tracker: string;
  issues: string;
  runtimeRoot: string;
  agentConfig: string;
  workstationActions: WorkstationActionRequest[];
  viewActions: WorkspaceActionRequest[];
  activityLoads: ActivityJournalFilters[];
}): WorkspaceClient {
  const backendJson = (args: readonly string[]) => {
    const result = spawnSync("python3", ["-m", "albert_mvp", ...args], {
      cwd: repositoryRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        PYTHONPATH: process.env.PYTHONPATH
          ? `${repositoryRoot}:${process.env.PYTHONPATH}`
          : repositoryRoot,
      },
    });
    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    return JSON.parse(result.stdout);
  };
  const common = [
    "--target-repo",
    options.workspace,
    "--tracker-dir",
    options.tracker,
    "--issues-dir",
    options.issues,
    "--runtime-root",
    options.runtimeRoot,
    "--mission-id",
    "alfredo-release",
    "--agent-config",
    options.agentConfig,
  ];

  return {
    loadLaunchContext: async () => ({ kind: "launch-context", context: options.launchContext }),
    loadAgentCapabilities: async () => ({
      kind: "capabilities",
      catalog: backendJson(["agent-capabilities", ...common]),
    }),
    loadSnapshot: async () => {
      const snapshot = backendJson(["workspace-snapshot", ...common]);
      return { kind: snapshot.workspace_session.status === "empty" ? "empty" : "ready", snapshot };
    },
    loadConsoleHistory: async () => ({
      kind: "history",
      history: backendJson(["agent-console-history", ...common]) as AgentConsoleHistory,
    }),
    appendConsoleMessage: async (request) => ({
      kind: "message",
      message: backendJson([
        "agent-console-message",
        ...common,
        "--role",
        request.role,
        "--content",
        request.content,
        "--outcome",
        request.outcome,
        "--source",
        request.source,
        "--expected-revision",
        String(request.expected_revision),
        "--scope-kind",
        request.scope_kind,
        "--scope-target",
        request.scope_target,
        "--scope-label",
        request.scope_label,
        ...(request.scope_mission_id
          ? ["--scope-mission-id", request.scope_mission_id]
          : []),
      ]),
    }),
    loadWorkspaceQueue: async () => ({
      kind: "workspace-queue",
      projection: backendJson(["workspace-queue", ...common]),
    }),
    submitAdHocDelegationProposal: async (request) => {
      const args = [
        "ad-hoc-delegation-proposal",
        ...common,
        "--correlation-id",
        request.correlation_id,
        "--expected-revision",
        String(request.expected_revision),
        "--source",
        request.source,
        "--scope-kind",
        request.scope_kind,
        "--scope-target",
        request.scope_target,
        "--scope-label",
        request.scope_label,
        "--proposed-agent",
        request.proposed_agent,
        "--originating-message-id",
        request.originating_message_id,
      ];
      request.acceptance_criteria.forEach((criterion) => {
        args.push("--acceptance-criterion", criterion);
      });
      request.allowed_paths.forEach((path) => {
        args.push("--allowed-path", path);
      });
      Object.entries(request.command_policy).forEach(([command, policy]) => {
        args.push("--command-policy", `${command}=${policy}`);
      });
      if (request.mission_id) args.push("--queue-mission-id", request.mission_id);
      return { kind: "acknowledged", acknowledgement: backendJson(args) };
    },
    submitWorkspaceQueueDecision: async (request) => {
      const args: string[] = [
        "workspace-queue-decision",
        ...common,
        "--correlation-id",
        request.correlation_id,
        "--expected-queue-revision",
        String(request.expected_revision),
        "--item-id",
        request.item_id,
        "--decision",
        request.decision,
        "--reason",
        request.reason ?? "",
      ];
      if (request.action_type) args.push("--action-type", request.action_type);
      if (request.actor) args.push("--actor", request.actor);
      if (request.target) {
        args.push("--target-kind", request.target.kind, "--target-id", request.target.id);
      }
      return { kind: "acknowledged", acknowledgement: backendJson(args) };
    },
    submitWorkstationAction: async (request) => {
      options.workstationActions.push(request);
      const acknowledgement = backendJson([
        "workstation-action",
        ...common,
        "--correlation-id",
        request.correlation_id,
        "--expected-revision",
        String(request.expected_revision),
        "--action-type",
        request.action_type,
        "--actor",
        request.actor,
        "--target-kind",
        request.target.kind,
        "--target-id",
        request.target.id,
        "--issue-id",
        request.issue_id ?? "",
        "--session-id",
        request.session_id ?? "",
      ]);
      return { kind: "acknowledged", acknowledgement };
    },
    runWorkstationSession: async (request) => ({
      kind: "session-finished",
      session: backendJson([
        "workstation-session-run",
        ...common,
        "--session-id",
        request.session_id,
        ...(request.mission_id
          ? ["--session-mission-id", request.mission_id]
          : []),
      ]),
    }),
    submitAction: async (request) => {
      options.viewActions.push(request);
      const acknowledgement = backendJson([
        "workspace-action",
        ...common,
        "--correlation-id",
        request.correlation_id,
        "--expected-revision",
        String(request.expected_revision),
        "--operations-view",
        request.operations_view,
      ]);
      return { kind: "acknowledged", acknowledgement };
    },
    loadUpdates: async (afterRevision) => ({
      kind: "updates",
      batch: backendJson([
        "workspace-updates",
        ...common,
        "--after-revision",
        String(afterRevision),
      ]),
    }),
    loadActivityJournal: async (filters = {}) => {
      options.activityLoads.push(filters);
      const args = ["activity-journal", ...common];
      if (filters.mission_id) args.push("--activity-mission-id", filters.mission_id);
      if (filters.actor) args.push("--actor", filters.actor);
      if (filters.action_type) args.push("--action-type", filters.action_type);
      if (filters.search) args.push("--search", filters.search);
      return {
        kind: "activity-journal",
        projection: backendJson(args),
      };
    },
  };
}

test("release seam covers launch intent, workstation action acknowledgement, journal, and restart restore", async () => {
  window.localStorage.clear();
  const root = mkdtempSync(resolve(tmpdir(), "alfredo-release-seam-"));
  const trackedSessionId = "session-ISS-01-2";

  try {
    const runtimeRoot = resolve(root, "runtime");
    const { workspace, tracker, issues, agentConfig } = writeReleaseTracker(root);
    const launch = spawnSync(process.execPath, [alfredoBinPath(), "--agent", "fake-controller"], {
      cwd: workspace,
      encoding: "utf8",
      env: {
        ...process.env,
        ALFREDO_AGENT_CONFIG: agentConfig,
        ALFREDO_DESKTOP_DRY_RUN: "1",
        ALFREDO_RUNTIME_ROOT: runtimeRoot,
      },
    });
    expect(launch.stderr).toBe("");
    expect(launch.status).toBe(0);
    const launchPlan = JSON.parse(launch.stdout);
    expect(launchPlan).toMatchObject({
      product: "Alfredo",
      launch: "workstation",
      selected_agent: "fake-controller",
      selected_model: "deterministic-controller",
      starting_location: workspace,
      workspace_selection: {
        schema_version: 1,
        phase: "selection-required",
        starting_location: workspace,
        coding_workspace: null,
        active_mission: null,
      },
      runtime_root: runtimeRoot,
      recent_workspaces: [],
    });
    expect(launchPlan).not.toHaveProperty("selected_workspace");
    expect(launchPlan.preflight.map((check: { name: string }) => check.name)).toEqual([
      "product_install",
      "node_runtime",
      "npm_runtime",
      "desktop_shell",
      "backend_process",
      "sandbox_runtime",
      "starting_location_access",
      "writable_runtime",
      "ollama",
      "required_model",
    ]);

    const common = [
      "--target-repo",
      workspace,
      "--tracker-dir",
      tracker,
      "--issues-dir",
      issues,
      "--runtime-root",
      runtimeRoot,
      "--mission-id",
      "alfredo-release",
      "--agent-config",
      agentConfig,
    ];
    const runBackendText = (args: readonly string[]) => {
      const result = spawnSync("python3", ["-m", "albert_mvp", ...args], {
        cwd: repositoryRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          PYTHONPATH: process.env.PYTHONPATH
            ? `${repositoryRoot}:${process.env.PYTHONPATH}`
            : repositoryRoot,
        },
      });
      expect(result.stderr).toBe("");
      expect(result.status).toBe(0);
      return result.stdout;
    };
    const runBackend = (args: readonly string[]) => JSON.parse(runBackendText(args));
    runBackend([
      "agent-console-message",
      ...common,
      "--role",
      "user",
      "--content",
      "Launch the release seam verification.",
      "--outcome",
      "proposed",
      "--source",
      "mission-commander",
      "--expected-revision",
      "1",
      "--scope-kind",
      "working-directory",
      "--scope-target",
      workspace,
      "--scope-label",
      "workspace",
    ]);
    runBackend([
      "agent-console-message",
      ...common,
      "--role",
      "assistant",
      "--content",
      "I will keep the action governed and wait for Orchestrator acknowledgement.",
      "--outcome",
      "model-commentary",
      "--source",
      "frontier-model",
      "--expected-revision",
      "1",
      "--scope-kind",
      "working-directory",
      "--scope-target",
      workspace,
      "--scope-label",
      "workspace",
    ]);

    const workstationActions: WorkstationActionRequest[] = [];
    const viewActions: WorkspaceActionRequest[] = [];
    const activityLoads: ActivityJournalFilters[] = [];
    const client = createBackendClient({
      launchContext: {
        schema_version: 1,
        selected_agent: launchPlan.selected_agent,
        selected_model: launchPlan.selected_model,
        starting_location: workspace,
        coding_workspace: workspace,
        active_mission: "release-smoke",
        phase: "workspace-ready",
        runtime_root: launchPlan.runtime_root,
        recent_workspaces: launchPlan.recent_workspaces,
      },
      workspace,
      tracker,
      issues,
      runtimeRoot,
      agentConfig,
      workstationActions,
      viewActions,
      activityLoads,
    });

    const first = render(<App client={client} />);
    expect(await screen.findByRole("main", { name: "Prompt Workstation" })).toBeVisible();
    const statusLine = screen.getByLabelText("Prompt status line");
    const controllerPicker = screen.getByRole("combobox", { name: "Controller model" });
    expect(controllerPicker).toHaveValue("fake-controller");
    expect((controllerPicker as HTMLSelectElement).selectedOptions[0]).toHaveTextContent(
      "fake-controller · deterministic-controller",
    );
    expect(statusLine).toHaveTextContent("Workspace workspace");
    expect(statusLine).not.toHaveTextContent(/Runtime|recent workspaces/);
    expect(screen.getByRole("textbox", { name: "Message Alfredo" })).toBeVisible();
    expect(screen.getByText("Launch the release seam verification.")).toBeVisible();
    expect(screen.getByText(/keep the action governed/)).toBeVisible();

    fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
      target: { value: "Please fix the release seam polling with a subagent" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    expect(
      await screen.findByText("Please fix the release seam polling with a subagent"),
    ).toBeVisible();
    await waitFor(() => {
      expect(screen.getAllByText("session-ADHOC-000001-1").length).toBeGreaterThan(0);
    }, { timeout: 10_000 });
    expect(
      await screen.findByText(
        "Mission Commander approved coding task ADHOC-000001.",
      ),
    ).toBeVisible();
    expect(
      screen.getByText(
        "Orchestrator queued coding task ADHOC-000001 as session-ADHOC-000001-1 on fake-local.",
      ),
    ).toBeVisible();
    expect(
      await screen.findByText(
        "Workstation outcome: ADHOC-000001 is evidence-ready on fake-local.",
      ),
    ).toBeVisible();
    fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
      target: { value: "While that runs, explain the documentation layout." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    const immediateTranscript = screen.getByRole("region", { name: "Prompt Transcript" });
    const immediateOrderedTurns = [
      await within(immediateTranscript).findByText(
        "Please fix the release seam polling with a subagent",
      ),
      within(immediateTranscript).getByText(
        "Coding task proposal ADHOC-000001 was recorded from Agent Console.",
      ),
      within(immediateTranscript).getByText(
        "Mission Commander approved coding task ADHOC-000001.",
      ),
      within(immediateTranscript).getByText(
        "Orchestrator queued coding task ADHOC-000001 as session-ADHOC-000001-1 on fake-local.",
      ),
      await within(immediateTranscript).findByText(
        "While that runs, explain the documentation layout.",
      ),
    ].map((node) => node.closest("[data-timeline-key]"));
    expect(immediateOrderedTurns.every(Boolean)).toBe(true);
    for (let index = 0; index < immediateOrderedTurns.length - 1; index += 1) {
      expect(
        immediateOrderedTurns[index]!.compareDocumentPosition(
          immediateOrderedTurns[index + 1]!,
        ) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    }

    const assignmentBoard = screen.getByRole("table", { name: "Issue Assignment Board" });
    expect(within(assignmentBoard).getByText("Release Seam")).toBeVisible();
    fireEvent.click(
      await within(assignmentBoard).findByRole("button", { name: "Approve for launch ISS-01" }),
    );

    await waitFor(() => expect(workstationActions[0]).toEqual({
      correlation_id: "workstation-issue-approve-alfredo-release-ISS-01-1",
      action_type: "issue-approve",
      actor: "mission-commander",
      expected_revision: 1,
      target: { kind: "issue-slice", id: "ISS-01" },
      mission_id: "alfredo-release",
      issue_id: "ISS-01",
      session_id: undefined,
      agent_id: undefined,
      reason: undefined,
      allowed_paths: [],
      command_policy: {},
    }));
    expect(
      await within(screen.getByRole("region", { name: "Prompt Transcript" })).findByText(
        /approved ISS-01 for governed Local Agent launch/,
      ),
    ).toBeVisible();
    fireEvent.click(
      await within(assignmentBoard).findByRole("button", { name: "Launch ISS-01" }),
    );

    await waitFor(() =>
      expect(workstationActions).toEqual([
        {
          correlation_id: "workstation-issue-approve-alfredo-release-ISS-01-1",
          action_type: "issue-approve",
          actor: "mission-commander",
          expected_revision: 1,
          target: { kind: "issue-slice", id: "ISS-01" },
          mission_id: "alfredo-release",
          issue_id: "ISS-01",
          session_id: undefined,
          agent_id: undefined,
          reason: undefined,
          allowed_paths: [],
          command_policy: {},
        },
        {
          correlation_id: "workstation-issue-launch-alfredo-release-ISS-01-2",
          action_type: "issue-launch",
          actor: "mission-commander",
          expected_revision: 2,
          target: { kind: "issue-slice", id: "ISS-01" },
          mission_id: "alfredo-release",
          issue_id: "ISS-01",
          session_id: undefined,
          agent_id: undefined,
          reason: undefined,
          allowed_paths: [],
          command_policy: {},
        },
      ]),
    );
    expect(await screen.findByText(/Workstation action: Mission Commander requested Launch ISS-01/)).toBeVisible();
    expect(await screen.findByText(/Orchestrator accepted workstation action: Orchestrator queued ISS-01/)).toBeVisible();
    expect(screen.getAllByText(trackedSessionId).length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Inspect assignment ISS-01" }));
    expect(screen.getByRole("region", { name: "Issue Assignment Detail" })).toHaveTextContent(
      "Release Seam",
    );

    fireEvent.click(screen.getByRole("button", { name: "Open detail views" }));
    fireEvent.click(screen.getByRole("button", { name: "Activity" }));
    const journal = await screen.findByRole("region", { name: "Activity Journal" });
    expect(within(journal).getByText(`Orchestrator queued ISS-01 as ${trackedSessionId}.`)).toBeVisible();
    expect(
      within(journal).getByText(
        `Local Agent fake-local submitted validated evidence for ${trackedSessionId}.`,
      ),
    ).toBeVisible();
    expect(within(journal).getAllByText("issue-slice / ISS-01")).toHaveLength(3);
    expect(viewActions).toEqual([
      {
        correlation_id: "operations-view-activity-3",
        expected_revision: 3,
        operations_view: "activity",
      },
    ]);
    expect(activityLoads.length).toBeGreaterThan(0);

    first.unmount();
    render(<App client={client} />);
    const restoredTranscript = await screen.findByRole("region", { name: "Prompt Transcript" });
    expect(
      await within(restoredTranscript).findByText(
        "Coding task proposal ADHOC-000001 was recorded from Agent Console.",
      ),
    ).toBeVisible();
    expect(
      within(restoredTranscript).getByText(
        "Mission Commander approved coding task ADHOC-000001.",
      ),
    ).toBeVisible();
    expect(
      within(restoredTranscript).getByText(
        "Orchestrator queued coding task ADHOC-000001 as session-ADHOC-000001-1 on fake-local.",
      ),
    ).toBeVisible();
    const restoredOrderedTurns = [
      within(restoredTranscript).getByText(
        "Please fix the release seam polling with a subagent",
      ),
      within(restoredTranscript).getByText(
        "Coding task proposal ADHOC-000001 was recorded from Agent Console.",
      ),
      within(restoredTranscript).getByText(
        "Mission Commander approved coding task ADHOC-000001.",
      ),
      within(restoredTranscript).getByText(
        "Orchestrator queued coding task ADHOC-000001 as session-ADHOC-000001-1 on fake-local.",
      ),
      within(restoredTranscript).getByText(
        "While that runs, explain the documentation layout.",
      ),
    ].map((node) => node.closest("[data-timeline-key]"));
    expect(restoredOrderedTurns.every(Boolean)).toBe(true);
    for (let index = 0; index < restoredOrderedTurns.length - 1; index += 1) {
      expect(
        restoredOrderedTurns[index]!.compareDocumentPosition(restoredOrderedTurns[index + 1]!) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    }
    expect(within(restoredTranscript).getByText("Workstation action: Mission Commander requested issue launch for ISS-01.")).toBeVisible();
    expect(within(restoredTranscript).getByText(`Orchestrator accepted workstation action: Orchestrator queued ISS-01 as ${trackedSessionId}.`)).toBeVisible();
    expect(screen.getByRole("button", { name: "Open detail views" })).toBeVisible();
    expect(screen.queryByRole("region", { name: "Activity Journal" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open detail views" }));
    const restoredJournal = await screen.findByRole("region", { name: "Activity Journal" });
    expect(within(restoredJournal).getByText(`Orchestrator queued ISS-01 as ${trackedSessionId}.`)).toBeVisible();
    const restoredAssignmentDetail = screen.getByRole("region", { name: "Issue Assignment Detail" });
    expect(restoredAssignmentDetail).toHaveTextContent("ISS-01");
    expect(restoredAssignmentDetail).toHaveTextContent("Release Seam");
    expect(screen.getAllByText(trackedSessionId).length).toBeGreaterThan(0);
    expect(screen.getByLabelText("Prompt status line")).toHaveTextContent("Workspace workspace");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}, 15_000);
