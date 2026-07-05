import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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

test("opens to a prompt-dominant workstation with Agent Workstations beside it", async () => {
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
  const transcript = screen.getByRole("region", { name: "Prompt Transcript" });
  expect(within(transcript).getByText("Implement the next Alfredo workstation slice.")).toBeVisible();
  expect(within(transcript).getByText(/durable and route execution/)).toBeVisible();
  expect(within(transcript).getByText("Workstation action pending: ISS-02 delegation approval required.")).toBeVisible();
  expect(within(transcript).getByText("Workstation outcome: ISS-01 is running on qwen-coder-local.")).toBeVisible();
  expect(screen.getByRole("complementary", { name: "Agent Workstations" })).toBeVisible();
  const cards = screen.getByRole("region", { name: "Workstation Cards" });
  expect(cards).toBeVisible();
  expect(screen.getByText("session-ISS-01-1")).toBeVisible();
  expect(screen.getByText("qwen3.6:27b")).toBeVisible();
  expect(within(cards).getByText("ISS-02 delegation approval required")).toBeVisible();

  const statusLine = screen.getByLabelText("Prompt status line");
  expect(within(statusLine).getByText("Connection Connected")).toBeVisible();
  expect(within(statusLine).getByText("Scope Restore workspace session")).toBeVisible();
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

test("switches distinct side-pane modes without mixing prompt and terminal drafts", async () => {
  render(<App client={client} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
    target: { value: "Keep this console draft" },
  });
  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));

  expect(screen.getByRole("region", { name: "Workstation Cards" })).toBeVisible();
  expect(screen.getByRole("region", { name: "Shell Terminal" })).toBeVisible();

  fireEvent.change(screen.getByRole("textbox", { name: "Command" }), {
    target: { value: "python3 -m unittest --help" },
  });
  fireEvent.click(screen.getByRole("tab", { name: "Workstations" }));

  expect(screen.getByRole("textbox", { name: "Message Alfredo" })).toHaveValue(
    "Keep this console draft",
  );
  expect(screen.queryByRole("region", { name: "Shell Terminal" })).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));
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

test("loads authoritative terminal metadata without reconstructing terminal bytes", async () => {
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
  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));

  const terminal = await screen.findByRole("region", { name: "Shell Terminal" });
  expect(loadShellTerminal).toHaveBeenCalledTimes(1);
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
  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));
  await waitFor(() => expect(loadShellTerminal).toHaveBeenCalledTimes(1));

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
  expect(loadShellTerminal).toHaveBeenCalledTimes(2);

  fireEvent.click(screen.getByRole("tab", { name: "Workstations" }));
  expect(screen.queryByText("usage: python3 -m unittest")).not.toBeInTheDocument();
  await act(async () => {
    fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));
  });
  expect(screen.getByLabelText("Command output")).toHaveTextContent(
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
  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));
  fireEvent.click(await screen.findByRole("button", { name: `Approve ${command.command_id}` }));

  expect(decideShellTerminalCommand).toHaveBeenCalledWith({
    command_id: command.command_id,
    decision: "approve",
    actor: "mission-commander",
    reason: "",
  });
  expect(await screen.findByLabelText("Command output")).toHaveTextContent("pushed");
  expect(loadShellTerminal).toHaveBeenCalledTimes(2);
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
  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));
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
  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));

  expect(await screen.findByText("Awaiting Frontier Model approval")).toBeVisible();
  expect(screen.queryByRole("button", { name: `Approve ${command.command_id}` })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: `Deny ${command.command_id}` })).toBeDisabled();
});

test("creates a bounded Additional Path Grant through Mission Commander authority", async () => {
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
  let created = false;
  const loadShellTerminal = vi.fn(async () => ({
    kind: "shell-terminal" as const,
    projection: {
      schema_version: 1 as const,
      revision: created ? 2 : 1,
      commands: [],
      grants: created ? [grant] : [],
    },
  }));
  const createAdditionalPathGrant = vi.fn(async () => {
    created = true;
    return { kind: "path-grant" as const, grant };
  });
  render(<App client={{ ...client, loadShellTerminal, createAdditionalPathGrant }} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });
  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));
  await waitFor(() => expect(loadShellTerminal).toHaveBeenCalledTimes(1));

  fireEvent.change(screen.getByRole("textbox", { name: "Grant path" }), {
    target: { value: "/external/docs" },
  });
  fireEvent.change(screen.getByRole("combobox", { name: "Grant access level" }), {
    target: { value: "write" },
  });
  fireEvent.change(screen.getByRole("spinbutton", { name: "Grant duration seconds" }), {
    target: { value: "900" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Create Additional Path Grant" }));

  expect(createAdditionalPathGrant).toHaveBeenCalledWith({
    correlation_id: expect.stringMatching(/^path-grant-/),
    path: "/external/docs",
    access_level: "write",
    duration_seconds: 900,
    requester: "mission-commander",
  });
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
  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));

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
  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));
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

test("switches side-pane tabs with arrow keys at constrained width", async () => {
  const originalWidth = window.innerWidth;
  try {
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 700 });
    fireEvent(window, new Event("resize"));
    render(<App client={client} />);
    await screen.findByRole("heading", { name: "Command Deck Mission" });
    fireEvent.change(screen.getByRole("textbox", { name: "Message Alfredo" }), {
      target: { value: "Preserve constrained draft" },
    });
    const agentTab = screen.getByRole("tab", { name: "Workstations" });
    agentTab.focus();
    fireEvent.keyDown(agentTab, { key: "ArrowRight" });
    expect(screen.getByRole("tab", { name: "Shell Terminal" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("region", { name: "Shell Terminal" })).toBeVisible();

    const terminalTab = screen.getByRole("tab", { name: "Shell Terminal" });
    fireEvent.keyDown(terminalTab, { key: "ArrowLeft" });
    expect(screen.getByRole("tab", { name: "Workstations" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("textbox", { name: "Message Alfredo" })).toHaveValue(
      "Preserve constrained draft",
    );
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
  expect(screen.getByRole("complementary", { name: "Agent Workstations" })).toBeVisible();
  expect(screen.getByText("Restore workspace session", { selector: ".scope-card strong" })).toBeVisible();
  expect(screen.getByText("Workspace Session workspace-command-deck")).toBeVisible();
  expect(screen.getAllByText("ISS-01")).not.toHaveLength(0);
});

test("exposes named landmarks and labelled controls in both side-pane modes", async () => {
  render(<App client={client} />);
  await screen.findByRole("heading", { name: "Command Deck Mission" });

  expect(screen.getByRole("main", { name: "Prompt Workstation" })).toBeVisible();
  expect(screen.getByRole("complementary", { name: "Agent Workstations" })).toBeVisible();
  for (const control of document.querySelectorAll("button, input, select, textarea, a[href]")) {
    expect(control).toHaveAccessibleName();
  }

  fireEvent.click(screen.getByRole("tab", { name: "Shell Terminal" }));
  expect(screen.getByRole("region", { name: "Shell Terminal" })).toBeVisible();
  expect(screen.getByRole("alert")).toHaveTextContent("Shell Terminal transport is unavailable");
  for (const control of document.querySelectorAll("button, input, select, textarea, a[href]")) {
    expect(control).toHaveAccessibleName();
  }
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
      expected_revision: 2,
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
