/// <reference types="node" />
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
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
  const startingLocation = resolve(root, "projects");
  const workspace = resolve(startingLocation, "workspace");
  const tracker = resolve(workspace, ".agent", "issues");
  const issues = tracker;
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
    resolve(issues, "02-runner-recovery.md"),
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
      "Recover one exact dead-owner runner boundary and retire its result.",
      "",
      "## Acceptance criteria",
      "",
      "- [ ] The same session resumes once after an exact supervision receipt.",
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
  const repository = spawnSync("git", ["init", "--quiet", workspace], {
    encoding: "utf8",
  });
  expect(repository.stderr).toBe("");
  expect(repository.status).toBe(0);
  const baseline = spawnSync(
    "git",
    [
      "-C",
      workspace,
      "-c",
      "user.name=Alfredo Release",
      "-c",
      "user.email=release@example.invalid",
      "add",
      ".",
    ],
    { encoding: "utf8" },
  );
  expect(baseline.stderr).toBe("");
  expect(baseline.status).toBe(0);
  const commit = spawnSync(
    "git",
    [
      "-C",
      workspace,
      "-c",
      "user.name=Alfredo Release",
      "-c",
      "user.email=release@example.invalid",
      "commit",
      "--quiet",
      "-m",
      "release fixture",
    ],
    { encoding: "utf8" },
  );
  expect(commit.stderr).toBe("");
  expect(commit.status).toBe(0);
  return { startingLocation, workspace, tracker, issues, agentConfig };
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
    "agent-issues",
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
    generateConsoleResponse: async (request) => {
      const response = backendJson([
        "agent-console-response",
        ...common,
        "--message-id",
        request.message_id,
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
        ...(request.agent_id ? ["--agent-id", request.agent_id] : []),
      ]);
      return {
        kind: "message",
        message: response.message,
        route: response.route,
        wayfinder: response.wayfinder,
      };
    },
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
    loadReviewWorkspace: async () => ({
      kind: "review-workspace",
      projection: backendJson(["review-workspace", ...common]),
    }),
    submitReviewDecision: async (request) => ({
      kind: "acknowledged",
      acknowledgement: backendJson([
        "review-decision",
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
        "--session-id",
        request.session_id,
        ...(request.mission_id
          ? ["--review-mission-id", request.mission_id]
          : []),
        "--decision",
        request.decision,
        "--reason",
        request.reason,
        ...(request.failure_type ? ["--failure-type", request.failure_type] : []),
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
  const root = realpathSync(mkdtempSync(resolve(tmpdir(), "alfredo-release-seam-")));
  const trackedSessionId = "session-ISS-01-2";

  try {
    const runtimeRoot = resolve(root, "runtime");
    const { startingLocation, workspace, tracker, issues, agentConfig } =
      writeReleaseTracker(root);
    const launch = spawnSync(process.execPath, [alfredoBinPath(), "--agent", "fake-controller"], {
      cwd: startingLocation,
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
      starting_location: startingLocation,
      workspace_selection: {
        schema_version: 1,
        phase: "selection-required",
        starting_location: startingLocation,
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
      "agent-issues",
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
    type ReleaseMissionSession = {
      session_id: string;
      status: string;
      session_revision?: number;
      retirement_phase?: string;
      retirement_blocked_reason?: string;
      retirement_record?: { payload_disposition?: string } | null;
      supervision_receipt_id?: string;
      supervision_outcome?: string;
      automatic_recovery_count?: number;
    };
    const missionSession = (
      snapshot: { missions: readonly { sessions: readonly ReleaseMissionSession[] }[] },
      sessionId: string,
    ) => snapshot.missions.flatMap((mission) => mission.sessions)
      .find((session) => session.session_id === sessionId);
    const selectedWorkspace = runBackend([
      "coding-workspace-select",
      "--starting-location",
      startingLocation,
      "--workspace-path",
      workspace,
      "--selection-mode",
      "existing",
      "--runtime-root",
      runtimeRoot,
      "--correlation-id",
      "release-workspace-select",
    ]);
    expect(selectedWorkspace).toMatchObject({
      schema_version: 1,
      coding_workspace: workspace,
      active_mission: null,
    });
    const missionOptions = runBackend([
      "mission-options",
      "--starting-location",
      startingLocation,
      "--coding-workspace",
      workspace,
      "--runtime-root",
      runtimeRoot,
    ]);
    expect(missionOptions.missions).toEqual([
      { id: "agent-issues", title: "Alfredo Release Seam" },
    ]);
    const missionChoiceArgs = [
      "mission-choice",
      "--starting-location",
      startingLocation,
      "--coding-workspace",
      workspace,
      "--runtime-root",
      runtimeRoot,
      "--correlation-id",
      "release-mission-resume",
      "--expected-revision",
      "1",
      "--choice",
      "resume",
      "--mission-id",
      "agent-issues",
    ];
    expect(runBackend(missionChoiceArgs)).toMatchObject({
      outcome: "acknowledged",
      active_mission: "agent-issues",
      replayed: false,
    });
    expect(runBackend(missionChoiceArgs)).toMatchObject({
      outcome: "acknowledged",
      active_mission: "agent-issues",
      replayed: true,
    });
    const restoredJourney = runBackend([
      "workspace-context",
      "--starting-location",
      startingLocation,
      "--runtime-root",
      runtimeRoot,
    ]);
    expect(restoredJourney).toMatchObject({
      phase: "workspace-ready",
      coding_workspace: workspace,
      active_mission: "agent-issues",
    });
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
        starting_location: startingLocation,
        coding_workspace: workspace,
        active_mission: "agent-issues",
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

    const composer = screen.getByRole("textbox", { name: "Message Alfredo" });
    fireEvent.change(composer, {
      target: { value: "Start a new project for governed release planning." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    const wayfinderRoute = await screen.findByRole("status", { name: "Wayfinder route" });
    expect(wayfinderRoute).toHaveTextContent("Wayfinder / Chart mode");
    expect(wayfinderRoute).toHaveTextContent("Shared Understanding Gate pending");

    fireEvent.change(composer, {
      target: { value: "/wayfinder confirm" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    await waitFor(() => {
      expect(screen.getByRole("status", { name: "Wayfinder route" })).toHaveTextContent(
        "Shared Understanding Gate open",
      );
    });
    expect(
      await screen.findByText(/Mission Commander receipt opened the Shared Understanding Gate/),
    ).toBeVisible();

    fireEvent.change(composer, {
      target: { value: "Please fix the release seam polling with a subagent" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send prompt" }));
    expect(
      await screen.findByText("Please fix the release seam polling with a subagent"),
    ).toBeVisible();
    await waitFor(() => {
      const tree = screen.getByRole("region", { name: "Mission Execution Tree" });
      expect(
        within(tree).getByRole("treeitem", {
          name: /Local Agent session session-ADHOC-000001-1/,
        }),
      ).toBeVisible();
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
        "Local Agent fake-local submitted validated evidence for session-ADHOC-000001-1.",
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
      correlation_id: "workstation-issue-approve-agent-issues-ISS-01-1",
      action_type: "issue-approve",
      actor: "mission-commander",
      expected_revision: 1,
      target: { kind: "issue-slice", id: "ISS-01" },
      mission_id: "agent-issues",
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
          correlation_id: "workstation-issue-approve-agent-issues-ISS-01-1",
          action_type: "issue-approve",
          actor: "mission-commander",
          expected_revision: 1,
          target: { kind: "issue-slice", id: "ISS-01" },
          mission_id: "agent-issues",
          issue_id: "ISS-01",
          session_id: undefined,
          agent_id: undefined,
          reason: undefined,
          allowed_paths: [],
          command_policy: {},
        },
        {
          correlation_id: "workstation-issue-launch-agent-issues-ISS-01-2",
          action_type: "issue-launch",
          actor: "mission-commander",
          expected_revision: 2,
          target: { kind: "issue-slice", id: "ISS-01" },
          mission_id: "agent-issues",
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
    expect(
      within(screen.getByRole("region", { name: "Mission Execution Tree" })).getByRole(
        "treeitem",
        { name: new RegExp(`Local Agent session ${trackedSessionId}`) },
      ),
    ).toBeVisible();
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

    fireEvent.click(screen.getByRole("button", { name: "Review" }));
    const reviewWorkspace = await screen.findByRole("region", { name: "Review Workspace" });
    expect(within(reviewWorkspace).getByText(trackedSessionId)).toBeVisible();
    fireEvent.click(
      within(reviewWorkspace).getByRole("button", { name: `Accept ${trackedSessionId}` }),
    );
    expect(
      await screen.findByText(
        /Orchestrator accepted workstation action: Issue Slice becomes Complete and PR-ready/,
      ),
    ).toBeVisible();
    fireEvent.click(
      within(reviewWorkspace).getByRole("button", {
        name: "Accept session-ADHOC-000001-1",
      }),
    );
    expect(await within(reviewWorkspace).findByText("No evidence awaiting review")).toBeVisible();

    const reviewedSnapshot = runBackend(["workspace-snapshot", ...common]);
    expect(
      missionSession(reviewedSnapshot, trackedSessionId)?.retirement_blocked_reason,
    ).toBe("");
    expect(missionSession(reviewedSnapshot, trackedSessionId)).toMatchObject({
      status: "reviewed",
      retirement_phase: "retired",
      retirement_record: { payload_disposition: "retained" },
    });

    const crashApproval = runBackend([
      "workstation-action",
      ...common,
      "--correlation-id",
      "release-recovery-approve",
      "--expected-revision",
      String(reviewedSnapshot.revision),
      "--action-type",
      "issue-approve",
      "--actor",
      "mission-commander",
      "--target-kind",
      "issue-slice",
      "--target-id",
      "ISS-02",
      "--issue-id",
      "ISS-02",
    ]);
    const crashLaunch = runBackend([
      "workstation-action",
      ...common,
      "--correlation-id",
      "release-recovery-launch",
      "--expected-revision",
      String(crashApproval.revision),
      "--action-type",
      "issue-launch",
      "--actor",
      "mission-commander",
      "--target-kind",
      "issue-slice",
      "--target-id",
      "ISS-02",
      "--issue-id",
      "ISS-02",
      "--allowed-path",
      "FAKE_AGENT_RESULT.md",
    ]);
    const recoverySessionId = crashLaunch.session_id as string;
    expect(recoverySessionId).toBe("session-ISS-02-3");
    const crashFixture = spawnSync(
      "python3",
      [
        resolve(projectRoot, "scripts", "prepare-release-crash-fixture.py"),
        ...common,
        "--session-id",
        recoverySessionId,
      ],
      {
        cwd: repositoryRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          PYTHONPATH: process.env.PYTHONPATH
            ? `${repositoryRoot}:${process.env.PYTHONPATH}`
            : repositoryRoot,
        },
      },
    );
    expect(crashFixture.stderr).toBe("");
    expect(crashFixture.status).toBe(0);
    const crashedBoundary = JSON.parse(crashFixture.stdout) as {
      revision: number;
      runner_operation_id: string;
      runner_process_pid: number;
      worktree_identity: string;
    };
    expect(crashedBoundary.runner_operation_id).toMatch(/^runner-operation:/);
    expect(crashedBoundary.worktree_identity).toMatch(/^managed-git:/);
    const observationArgs = [
      "runner-observe",
      ...common,
      "--source-id",
      "release-seam-observer",
      "--source-incarnation",
      "release-seam-boot",
      "--sequence",
      "1",
      "--observation-mission-id",
      "agent-issues",
      "--session-id",
      recoverySessionId,
      "--session-revision",
      String(crashedBoundary.revision),
      "--runner-operation-id",
      crashedBoundary.runner_operation_id,
      "--owner-signal",
      "absent",
      "--process-group-signal",
      "absent",
      "--worktree-identity",
      crashedBoundary.worktree_identity,
      "--result-signal",
      "absent",
    ];
    const recoveryReceipt = runBackend(observationArgs);
    expect(recoveryReceipt).toMatchObject({
      outcome: "recovered",
      effect: "recover-same-session",
      session_id: recoverySessionId,
    });
    expect(runBackend(observationArgs)).toEqual(recoveryReceipt);
    const recoveredSnapshot = runBackend(["workspace-snapshot", ...common]);
    expect(missionSession(recoveredSnapshot, recoverySessionId)).toMatchObject({
      status: "queued",
      supervision_receipt_id: recoveryReceipt.receipt_id,
      supervision_outcome: "recovered",
      automatic_recovery_count: 1,
      retirement_phase: "active",
    });

    first.unmount();
    const recoveredRender = render(<App client={client} />);
    const recoveredTree = await screen.findByRole("region", { name: "Mission Execution Tree" });
    const recoveredTreeItem = within(recoveredTree).getByRole("treeitem", {
      name: new RegExp(`Local Agent session ${recoverySessionId}`),
    });
    fireEvent.click(recoveredTreeItem);
    const recoveredInspector = await screen.findByRole("region", {
      name: `${recoverySessionId} execution inspector`,
    });
    expect(within(recoveredInspector).getByText(/Supervision receipt:/)).toHaveTextContent(
      "recovered",
    );
    expect(within(recoveredInspector).getByText("Automatic recovery: 1 of 1 used")).toBeVisible();

    let recoveryEvidenceSnapshot = runBackend(["workspace-snapshot", ...common]);
    await waitFor(() => {
      recoveryEvidenceSnapshot = runBackend(["workspace-snapshot", ...common]);
      expect(missionSession(recoveryEvidenceSnapshot, recoverySessionId)?.status).toBe(
        "evidence-ready",
      );
    }, { timeout: 10_000 });
    const recoveryEvidence = missionSession(recoveryEvidenceSnapshot, recoverySessionId);
    expect(recoveryEvidence?.session_revision).toBeTypeOf("number");
    runBackend([
      "review-decision",
      ...common,
      "--correlation-id",
      "release-recovery-review",
      "--expected-revision",
      String(recoveryEvidence!.session_revision),
      "--action-type",
      "review-decision",
      "--actor",
      "mission-commander",
      "--target-kind",
      "agent-session",
      "--target-id",
      recoverySessionId,
      "--session-id",
      recoverySessionId,
      "--review-mission-id",
      "agent-issues",
      "--decision",
      "accept",
      "--reason",
      "The exact recovered session produced valid evidence.",
    ]);
    const retiredRecoverySnapshot = runBackend(["workspace-snapshot", ...common]);
    expect(missionSession(retiredRecoverySnapshot, recoverySessionId)).toMatchObject({
      status: "reviewed",
      supervision_outcome: "recovered",
      automatic_recovery_count: 1,
      retirement_phase: "retired",
      retirement_record: { payload_disposition: "retained" },
    });

    recoveredRender.unmount();
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
    const restoredTree = screen.getByRole("region", { name: "Mission Execution Tree" });
    const restoredSession = within(restoredTree).getByRole("treeitem", {
      name: new RegExp(`Local Agent session ${trackedSessionId}`),
    });
    expect(restoredSession).toBeVisible();
    fireEvent.click(restoredSession);
    const retirementInspector = await screen.findByRole("region", {
      name: `${trackedSessionId} execution inspector`,
    });
    expect(
      within(retirementInspector).getByRole("heading", { name: "Retirement Record" }),
    ).toBeVisible();
    expect(within(retirementInspector).getByText("Disposition").parentElement).toHaveTextContent(
      "Retained",
    );
    expect(screen.getByLabelText("Prompt status line")).toHaveTextContent("Workspace workspace");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}, 75_000);
