import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  createLocalhostWorkspaceClient,
  createWorkspaceClient,
  TauriWorkspaceClient,
} from "./workspace-client";
import type { WorkspaceSnapshot } from "./contracts";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));

const snapshot: WorkspaceSnapshot = {
  schema_version: 1,
  revision: 1,
  workspace_session: {
    id: "workspace-command-deck",
    workspace_path: "/workspace/albert",
    status: "ready",
  },
  active_mission: {
    id: "command-deck",
    title: "Command Deck Mission",
    issue_count: 1,
  },
  conversation_scope: {
    kind: "working-directory",
    target_id: "/workspace/albert",
    label: "albert",
  },
  operations_view: "mission-board",
  mission_board: {
    prd_title: "Command Deck Mission",
    issue_count: 1,
    ordered_issue_ids: ["ISS-01"],
    ready_issue_ids: ["ISS-01"],
    approved_issue_ids: [],
  },
};

test("loads a ready canonical snapshot through the Tauri command", async () => {
  vi.mocked(invoke).mockResolvedValueOnce(snapshot);

  const result = await new TauriWorkspaceClient().loadSnapshot();

  expect(invoke).toHaveBeenCalledWith("workspace_snapshot");
  expect(result).toEqual({ kind: "ready", snapshot });
});

test("an ordinary browser remains fail-closed without invoking IPC or fetch", async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  const invokeCallsBefore = vi.mocked(invoke).mock.calls.length;
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);

  try {
    const result = await createWorkspaceClient().loadSnapshot();

    expect(vi.mocked(invoke).mock.calls).toHaveLength(invokeCallsBefore);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(result).toEqual({
      kind: "startup-failure",
      message:
        "The browser preview has no Alfredo desktop bridge. Start the managed workstation from the repository root.",
      recoverable: false,
    });
  } finally {
    vi.unstubAllGlobals();
    vi.mocked(isTauri).mockReturnValue(true);
  }
});

test("native IPC wins over an injected localhost bridge without fetching", async () => {
  const fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);
  vi.stubGlobal("__ALFREDO_LOCALHOST_BRIDGE__", {
    endpoint: "/__alfredo/invoke",
    token: "localhost-secret",
  });
  vi.mocked(isTauri).mockReturnValue(true);
  vi.mocked(invoke).mockResolvedValueOnce(snapshot);

  try {
    const result = await createWorkspaceClient().loadSnapshot();

    expect(result).toEqual({ kind: "ready", snapshot });
    expect(invoke).toHaveBeenCalledWith("workspace_snapshot");
    expect(fetchMock).not.toHaveBeenCalled();
  } finally {
    vi.unstubAllGlobals();
  }
});

test("an explicit localhost bridge transports a typed successful response", async () => {
  const invokeCallsBefore = vi.mocked(invoke).mock.calls.length;
  const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
    const request = JSON.parse(String(init?.body)) as {
      id: string;
      command: string;
      args?: Record<string, unknown>;
    };
    expect(request.command).toBe("workspace_snapshot");
    expect(request.args).toEqual({});
    return new Response(
      JSON.stringify({ id: request.id, ok: true, value: snapshot }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });
  vi.stubGlobal("fetch", fetchMock);
  vi.stubGlobal("__ALFREDO_LOCALHOST_BRIDGE__", {
    endpoint: "/__alfredo/invoke",
    token: "localhost-secret",
  });
  vi.mocked(isTauri).mockReturnValue(false);

  try {
    const client = createWorkspaceClient();
    const result = await client.loadSnapshot();

    expect(result).toEqual({ kind: "ready", snapshot });
    expect(vi.mocked(invoke).mock.calls).toHaveLength(invokeCallsBefore);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      "/__alfredo/invoke",
      expect.objectContaining({
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-Alfredo-Bridge-Token": "localhost-secret",
        },
      }),
    );

    const performanceResult = await client.recordPerformanceMark?.({
      stage: "S2",
      boundary: "end",
      clock: "native",
      monotonic_ns: "",
      clock_id: "",
      detail: { outcome: "pass" },
    });
    expect(performanceResult).toEqual({ recorded: false });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  } finally {
    vi.unstubAllGlobals();
    vi.mocked(isTauri).mockReturnValue(true);
  }
});

test("the localhost bridge preserves a typed backend error", async () => {
  const fetchMock = vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
    const request = JSON.parse(String(init?.body)) as { id: string };
    return new Response(
      JSON.stringify({
        id: request.id,
        ok: false,
        error: {
          code: "persistence-read-failure",
          message: "Workspace preferences are corrupt",
          recoverable: true,
        },
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });

  const result = await createLocalhostWorkspaceClient(
    { endpoint: "/__alfredo/invoke", token: "localhost-secret" },
    fetchMock,
  ).loadSnapshot();

  expect(result).toEqual({
    kind: "persistence-read-failure",
    message: "Workspace preferences are corrupt",
    recoverable: true,
  });
});

test("the localhost bridge rejects a mismatched response correlation", async () => {
  const fetchMock = vi.fn(async () =>
    new Response(
      JSON.stringify({ id: "different-request", ok: true, value: snapshot }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    ),
  );

  const result = await createLocalhostWorkspaceClient(
    { endpoint: "/__alfredo/invoke", token: "localhost-secret" },
    fetchMock,
  ).loadSnapshot();

  expect(result).toEqual({
    kind: "contract-failure",
    message: "The localhost bridge response correlation did not match the request.",
    recoverable: false,
  });
});

test("the localhost bridge maps malformed responses and fetch failures structurally", async () => {
  const malformedResult = await createLocalhostWorkspaceClient(
    { endpoint: "/__alfredo/invoke", token: "localhost-secret" },
    vi.fn(async () =>
      new Response(JSON.stringify({ id: "missing-ok" }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    ),
  ).loadSnapshot();
  const startupResult = await createLocalhostWorkspaceClient(
    { endpoint: "/__alfredo/invoke", token: "localhost-secret" },
    vi.fn(async () => {
      throw new TypeError("fetch failed");
    }),
  ).loadSnapshot();

  expect(malformedResult).toEqual({
    kind: "contract-failure",
    message: "The localhost bridge returned a malformed response envelope.",
    recoverable: false,
  });
  expect(startupResult).toEqual({
    kind: "startup-failure",
    message: "The localhost bridge request failed: fetch failed",
    recoverable: true,
  });
});

test("records a process-local performance boundary through the native bridge", async () => {
  const request = {
    stage: "S4" as const,
    boundary: "start" as const,
    clock: "native" as const,
    monotonic_ns: "",
    clock_id: "",
    detail: { outcome: "pass" },
  };
  vi.mocked(invoke).mockResolvedValueOnce({ recorded: true });

  const result = await new TauriWorkspaceClient().recordPerformanceMark(request);

  expect(invoke).toHaveBeenCalledWith("performance_mark", { request });
  expect(result).toEqual({ recorded: true });
});

test("preserves a structured Coding Workspace selection failure from Tauri", async () => {
  const request = {
    correlation_id: "workspace-select-unsafe-1",
    workspace_path: "/opt/alfredo/backend",
    selection_mode: "existing" as const,
  };
  vi.mocked(invoke).mockRejectedValueOnce({
    code: "workspace-unsafe",
    message: "The selected repository overlaps an Alfredo backend root.",
    recoverable: true,
  });

  const result = await new TauriWorkspaceClient().selectCodingWorkspace(request);

  expect(invoke).toHaveBeenCalledWith("coding_workspace_select", { request });
  expect(result).toEqual({
    kind: "selection-failure",
    code: "workspace-unsafe",
    message: "The selected repository overlaps an Alfredo backend root.",
    recoverable: true,
  });
});

test("submits an explicit Mission choice through Tauri", async () => {
  const request = {
    correlation_id: "mission-choice-1",
    expected_revision: 1,
    choice: "resume" as const,
    mission_id: "existing",
    mission_title: "",
  };
  const acknowledgement = {
    schema_version: 1 as const,
    correlation_id: request.correlation_id,
    outcome: "acknowledged" as const,
    coding_workspace: "/workspace/albert",
    choice: request.choice,
    active_mission: "existing",
    revision: 2,
    replayed: false,
    missions: [{ id: "existing", title: "Existing Mission" }],
    message: "Existing Mission resumed.",
  };
  vi.mocked(invoke).mockResolvedValueOnce(acknowledgement);

  const result = await new TauriWorkspaceClient().chooseMission(request);

  expect(invoke).toHaveBeenCalledWith("mission_choice", { request });
  expect(result).toEqual({ kind: "acknowledged", acknowledgement });
});

test("loads the installed coding-agent capability catalog through Tauri", async () => {
  const catalog = {
    schema_version: 1 as const,
    default_agent_id: "qwen3-14b",
    skills: [
      {
        name: "diagnose",
        description: "Diagnose hard bugs.",
        source: "/workspace/.agents/skills/diagnose/SKILL.md",
        invocation: "/use diagnose",
      },
    ],
    commands: [
      {
        name: "/run",
        usage: "/run <command>",
        description: "Run a governed command.",
        category: "execution",
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
      },
    ],
  };
  vi.mocked(invoke).mockResolvedValueOnce(catalog);

  const result = await new TauriWorkspaceClient().loadAgentCapabilities();

  expect(invoke).toHaveBeenCalledWith("agent_capabilities");
  expect(result).toEqual({ kind: "capabilities", catalog });
});

test("preserves a structured persistence failure from the Tauri bridge", async () => {
  vi.mocked(invoke).mockRejectedValueOnce({
    code: "persistence-read-failure",
    message: "Workspace preferences are corrupt",
    recoverable: true,
  });

  const result = await new TauriWorkspaceClient().loadSnapshot();

  expect(result).toEqual({
    kind: "persistence-read-failure",
    message: "Workspace preferences are corrupt",
    recoverable: true,
  });
});

test("loads ordered updates after the acknowledged revision", async () => {
  const batch = {
    after_revision: 1,
    current_revision: 2,
    events: [
      {
        event_id: "workspace-2-activity",
        correlation_id: "activity",
        revision: 2,
        kind: "workspace-preferences-updated" as const,
        active_mission_id: "command-deck",
        conversation_scope: snapshot.conversation_scope,
        operations_view: "activity",
      },
    ],
  };
  vi.mocked(invoke).mockResolvedValueOnce(batch);

  const result = await new TauriWorkspaceClient().loadUpdates(1);

  expect(invoke).toHaveBeenCalledWith("workspace_updates", { afterRevision: 1 });
  expect(result).toEqual({ kind: "updates", batch });
});

test("submits a correlated action and preserves its acknowledgement", async () => {
  const acknowledgement = {
    correlation_id: "activity-1",
    outcome: "acknowledged" as const,
    revision: 2,
  };
  vi.mocked(invoke).mockResolvedValueOnce(acknowledgement);
  const action = {
    correlation_id: "activity-1",
    expected_revision: 1,
    operations_view: "activity",
  };

  const result = await new TauriWorkspaceClient().submitAction(action);

  expect(invoke).toHaveBeenCalledWith("workspace_action", { action });
  expect(result).toEqual({ kind: "acknowledged", acknowledgement });
});

test("maps stale bridge rejection separately from invalid rejection", async () => {
  vi.mocked(invoke).mockRejectedValueOnce({
    code: "stale-action",
    message: "Expected revision 1 but current revision is 2",
    recoverable: true,
    current_revision: 2,
  });

  const result = await new TauriWorkspaceClient().submitAction({
    correlation_id: "review-stale",
    expected_revision: 1,
    operations_view: "review-workspace",
  });

  expect(result).toEqual({
    kind: "stale",
    code: "stale-action",
    message: "Expected revision 1 but current revision is 2",
    current_revision: 2,
  });
});

test("loads typed Agent Console history through Tauri", async () => {
  const history = {
    schema_version: 1 as const,
    messages: [
      {
        message_id: "console-000001",
        sequence: 1,
        role: "assistant" as const,
        content: "Persisted guidance",
        scope: snapshot.conversation_scope,
        outcome: "model-commentary" as const,
        source: "frontier-model",
      },
    ],
  };
  vi.mocked(invoke).mockResolvedValueOnce(history);

  const result = await new TauriWorkspaceClient().loadConsoleHistory();

  expect(invoke).toHaveBeenCalledWith("agent_console_history");
  expect(result).toEqual({ kind: "history", history });
});

test("submits explicit Conversation Scope through Tauri", async () => {
  const acknowledgement = {
    correlation_id: "scope-mission-4",
    outcome: "acknowledged" as const,
    revision: 5,
  };
  const scope = {
    correlation_id: "scope-mission-4",
    action_type: "conversation-scope-change" as const,
    actor: "mission-commander" as const,
    expected_revision: 4,
    target: { kind: "conversation-scope" as const, id: "command-deck" },
    scope_kind: "mission" as const,
    scope_target: "command-deck",
    scope_label: "Command Deck Mission",
  };
  vi.mocked(invoke).mockResolvedValueOnce(acknowledgement);

  const result = await new TauriWorkspaceClient().changeScope(scope);

  expect(invoke).toHaveBeenCalledWith("workspace_scope", { scope });
  expect(result).toEqual({ kind: "acknowledged", acknowledgement });
});

test("submits Active Mission switch through Tauri", async () => {
  const acknowledgement = {
    correlation_id: "active-mission-background-4",
    outcome: "acknowledged" as const,
    revision: 5,
  };
  const request = {
    correlation_id: "active-mission-background-4",
    expected_revision: 4,
    active_mission_id: "background-mission",
  };
  vi.mocked(invoke).mockResolvedValueOnce(acknowledgement);

  const result = await new TauriWorkspaceClient().switchMission(request);

  expect(invoke).toHaveBeenCalledWith("workspace_mission_switch", { request });
  expect(result).toEqual({ kind: "acknowledged", acknowledgement });
});

test("appends explicitly scoped Agent Console message through Tauri", async () => {
  const request = {
    role: "user" as const,
    content: "Explain the mission",
    outcome: "proposed" as const,
    source: "mission-commander",
    expected_revision: 4,
    scope_kind: "issue-slice" as const,
    scope_target: "ISS-01",
    scope_label: "Restore workspace session",
  };
  const message = {
    message_id: "console-000001",
    sequence: 1,
    role: request.role,
    content: request.content,
    scope: snapshot.conversation_scope,
    outcome: request.outcome,
    source: request.source,
  };
  vi.mocked(invoke).mockResolvedValueOnce(message);

  const result = await new TauriWorkspaceClient().appendConsoleMessage(request);

  expect(invoke).toHaveBeenCalledWith("agent_console_message", { message: request });
  expect(result).toEqual({ kind: "message", message });
});

test("generates Agent Console controller response through Tauri", async () => {
  const request = {
    expected_revision: 4,
    message_id: "console-000001",
    scope_kind: "issue-slice" as const,
    scope_target: "ISS-01",
    scope_label: "Restore workspace session",
    agent_id: "qwen3.6-27b",
  };
  const message = {
    message_id: "console-000002",
    sequence: 2,
    role: "assistant" as const,
    content: "Controller response",
    scope: snapshot.conversation_scope,
    outcome: "model-commentary" as const,
    source: "frontier-model",
  };
  const route = {
    intent: "coding-task" as const,
    task_request: "Fix workspace polling.",
    acceptance_criteria: ["Polling recovers after a transient failure."],
  };
  vi.mocked(invoke).mockResolvedValueOnce({ message, route });

  const result = await new TauriWorkspaceClient().generateConsoleResponse(request);

  expect(invoke).toHaveBeenCalledWith("agent_console_response", { request });
  expect(result).toEqual({ kind: "message", message, route });
});

test("loads the typed Working Context projection through Tauri", async () => {
  const projection = {
    schema_version: 1 as const,
    revision: 1,
    scope: snapshot.conversation_scope,
    content_character_count: 240,
    sources: [
      {
        source_id: "shared-context:working-directory:/workspace/albert",
        kind: "shared-context" as const,
        label: "Shared Context — albert",
        content: "Mission: Command Deck Mission.",
        governed: true,
        eligible: false,
        disposition: "required" as const,
      },
    ],
  };
  vi.mocked(invoke).mockResolvedValueOnce(projection);

  const result = await new TauriWorkspaceClient().loadWorkingContext();

  expect(invoke).toHaveBeenCalledWith("working_context");
  expect(result).toEqual({ kind: "working-context", projection });
});

test("submits eligible Working Context curation through Tauri", async () => {
  const request = {
    source_id: "message:console-000001",
    disposition: "pinned" as const,
    expected_context_revision: 1,
  };
  const acknowledgement = { outcome: "acknowledged" as const, revision: 2 };
  vi.mocked(invoke).mockResolvedValueOnce(acknowledgement);

  const result = await new TauriWorkspaceClient().curateWorkingContext(request);

  expect(invoke).toHaveBeenCalledWith("working_context_curate", { request });
  expect(result).toEqual({ kind: "acknowledged", acknowledgement });
});

test("loads the typed Review Workspace projection through Tauri", async () => {
  const projection = {
    schema_version: 1 as const,
    revision: 4,
    mission_id: "command-deck",
    items: [
      {
        mission_id: "command-deck",
        issue_id: "ISS-01",
        issue_title: "Review evidence",
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
          proposed_context_updates: "Document review flow.",
          artifact_links: ["app-local://evidence/session-ISS-01-1"],
        },
        visibility_limitations: [],
      },
    ],
  };
  vi.mocked(invoke).mockResolvedValueOnce(projection);

  const result = await new TauriWorkspaceClient().loadReviewWorkspace();

  expect(invoke).toHaveBeenCalledWith("review_workspace");
  expect(result).toEqual({ kind: "review-workspace", projection });
});

test("loads the filtered Activity Journal projection through Tauri", async () => {
  const filters = {
    search: "evidence",
    mission_id: "command-deck",
    actor: "mission-commander" as const,
    action_type: "review-decision",
    started_at: "2026-06-26T10:00:00Z",
    ended_at: "2026-06-26T11:00:00Z",
  };
  const projection = {
    schema_version: 1 as const,
    revision: 1,
    entries: [
      {
        entry_id: "activity-000001",
        sequence: 1,
        recorded_at: "2026-06-26T10:15:00Z",
        actor: "mission-commander" as const,
        action_type: "review-decision",
        summary: "Mission Commander recorded Review Workspace decision Approved.",
        affected_entities: [
          {
            entity_type: "issue-slice",
            entity_id: "ISS-01",
            label: "Restore Workspace Session",
            href: "app-local://missions/command-deck/issues/ISS-01",
          },
        ],
        evidence_links: ["app-local://evidence/session-ISS-01-1"],
        correlation_id: "review-activity-cli-1",
      },
    ],
  };
  vi.mocked(invoke).mockResolvedValueOnce(projection);

  const result = await new TauriWorkspaceClient().loadActivityJournal(filters);

  expect(invoke).toHaveBeenCalledWith("activity_journal", { filters });
  expect(result).toEqual({ kind: "activity-journal", projection });
});

test("loads the governed Shell Terminal projection through Tauri", async () => {
  const projection = {
    schema_version: 1 as const,
    revision: 2,
    commands: [
      {
        command_id: "terminal-command-000001",
        correlation_id: "terminal-client-1",
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
    grants: [
      {
        grant_id: "path-grant-000001",
        correlation_id: "path-grant-client-1",
        request_id: "path-grant-request-000001",
        path: "/external/docs",
        access_level: "read" as const,
        duration_seconds: 900,
        granted_by: "mission-commander" as const,
        granted_at: "2026-06-27T08:00:00Z",
        expires_at: "2026-06-27T08:15:00Z",
      },
    ],
    path_grant_requests: [
      {
        request_id: "path-grant-request-000001",
        correlation_id: "terminal-client-1",
        mission_id: "command-deck",
        path: "/external/docs",
        access_level: "read" as const,
        duration_seconds: 900,
        requester: "mission-commander",
        requested_at: "2026-06-27T07:59:00Z",
        reason: "External documentation requires an Additional Path Grant.",
        affected_action: "python3 -m unittest --help",
        status: "granted" as const,
      },
    ],
  };
  vi.mocked(invoke).mockResolvedValueOnce(projection);

  const result = await new TauriWorkspaceClient().loadShellTerminal();

  expect(invoke).toHaveBeenCalledWith("shell_terminal");
  expect(result).toEqual({ kind: "shell-terminal", projection });
});

test("submits a governed Shell Terminal command through Tauri", async () => {
  const request = {
    correlation_id: "terminal-client-submit-1",
    command: "python3 -m unittest --help",
    working_directory: "/workspace/albert",
    requested_paths: [] as string[],
    requester: "mission-commander",
    access_level: "read" as const,
  };
  const commandResult = {
    command_id: "terminal-command-000001",
    correlation_id: request.correlation_id,
    classification: "auto-allowed" as const,
    status: "completed" as const,
    exit_code: 0,
    stdout: "usage: python3 -m unittest",
    stderr: "",
  };
  vi.mocked(invoke).mockResolvedValueOnce(commandResult);

  const result = await new TauriWorkspaceClient().submitShellTerminalCommand(request);

  expect(invoke).toHaveBeenCalledWith("shell_terminal_submit", { request });
  expect(result).toEqual({ kind: "command-result", result: commandResult });
});

test("approves a pending Shell Terminal command through Tauri", async () => {
  const request = {
    command_id: "terminal-command-000002",
    decision: "approve" as const,
    actor: "mission-commander" as const,
    reason: "Approved for the requested repository task.",
  };
  const commandResult = {
    command_id: request.command_id,
    correlation_id: "terminal-client-human-1",
    classification: "human-required" as const,
    status: "completed" as const,
    exit_code: 0,
    stdout: "human approved\n",
    stderr: "",
  };
  vi.mocked(invoke).mockResolvedValueOnce(commandResult);

  const result = await new TauriWorkspaceClient().decideShellTerminalCommand(request);

  expect(invoke).toHaveBeenCalledWith("shell_terminal_decision", { request });
  expect(result).toEqual({ kind: "command-result", result: commandResult });
});

test("creates an Additional Path Grant through Tauri", async () => {
  const request = {
    correlation_id: "path-grant-client-2",
    request_id: "path-grant-request-000002",
    expected_revision: 3,
    path: "/external/docs",
    access_level: "write" as const,
    duration_seconds: 900,
    requester: "mission-commander" as const,
  };
  const grant = {
    grant_id: "path-grant-000002",
    correlation_id: request.correlation_id,
    request_id: request.request_id,
    path: request.path,
    access_level: request.access_level,
    duration_seconds: request.duration_seconds,
    granted_by: "mission-commander" as const,
    granted_at: "2026-06-29T12:00:00Z",
    expires_at: "2026-06-29T12:15:00Z",
  };
  vi.mocked(invoke).mockResolvedValueOnce(grant);

  const result = await new TauriWorkspaceClient().createAdditionalPathGrant(request);

  expect(invoke).toHaveBeenCalledWith("additional_path_grant_create", { request });
  expect(result).toEqual({ kind: "path-grant", grant });
});

test("denies a contextual Additional Path Grant request through Tauri", async () => {
  const request = {
    correlation_id: "path-grant-denial-client-1",
    request_id: "contextual-grant-client-1",
    expected_revision: 4,
    path: "/external/docs",
    access_level: "read" as const,
    duration_seconds: 300,
    requester: "mission-commander" as const,
    reason: "The blocked command requested external documentation.",
    affected_action: "python3 docs/check.py",
  };
  const denial = {
    denial_id: "path-grant-denial-000001",
    correlation_id: request.correlation_id,
    request_id: request.request_id,
    path: request.path,
    access_level: request.access_level,
    duration_seconds: request.duration_seconds,
    denied_by: "mission-commander" as const,
    denied_at: "2026-07-11T10:30:00Z",
    reason: request.reason,
    affected_action: request.affected_action,
  };
  vi.mocked(invoke).mockResolvedValueOnce(denial);

  const result = await new TauriWorkspaceClient().denyAdditionalPathGrant(request);

  expect(invoke).toHaveBeenCalledWith("additional_path_grant_deny", { request });
  expect(result).toEqual({ kind: "path-grant-denied", denial });
});

test("loads the typed Workspace Queue projection through Tauri", async () => {
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
  const projection = {
    schema_version: 1 as const,
    revision: 2,
    items: [item],
    groups: [
      {
        group_id: "issue-change-proposal:command-deck",
        item_type: "issue-change-proposal" as const,
        mission_id: "command-deck",
        item_count: 1,
        items: [item],
      },
    ],
  };
  vi.mocked(invoke).mockResolvedValueOnce(projection);

  const result = await new TauriWorkspaceClient().loadWorkspaceQueue();

  expect(invoke).toHaveBeenCalledWith("workspace_queue");
  expect(result).toEqual({ kind: "workspace-queue", projection });
});

test("loads the typed Mission Draft projection through Tauri", async () => {
  const projection = {
    schema_version: 1 as const,
    revision: 2,
    drafts: [
      {
        draft_id: "mission-draft-command-deck-000001",
        mission_id: "command-deck",
        status: "draft" as const,
        proposed_goal: "Create a focused Command Deck follow-up mission.",
        included_ad_hoc_work: [
          {
            work_id: "ADHOC-000001",
            source: "agent-console",
            status: "pending" as const,
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
  vi.mocked(invoke).mockResolvedValueOnce(projection);

  const result = await new TauriWorkspaceClient().loadMissionDrafts();

  expect(invoke).toHaveBeenCalledWith("mission_drafts");
  expect(result).toEqual({ kind: "mission-drafts", projection });
});

test("submits a Mission Draft confirmation through Tauri", async () => {
  const request = {
    correlation_id: "mission-draft-confirm-1",
    expected_revision: 2,
    draft_id: "mission-draft-command-deck-000001",
    decision: "confirm" as const,
    reason: "Mission Commander confirmed the draft.",
  };
  const acknowledgement = {
    correlation_id: request.correlation_id,
    outcome: "acknowledged" as const,
    revision: 3,
    draft_id: request.draft_id,
    draft_status: "confirmed" as const,
    effect_summary: "Mission Draft confirmed as accepted Issue Slice ISS-02.",
    accepted_issue_id: "ISS-02",
  };
  vi.mocked(invoke).mockResolvedValueOnce(acknowledgement);

  const result = await new TauriWorkspaceClient().submitMissionDraftDecision(request);

  expect(invoke).toHaveBeenCalledWith("mission_draft_decision", { request });
  expect(result).toEqual({ kind: "acknowledged", acknowledgement });
});

test("submits Mission Draft creation through Tauri", async () => {
  const request = {
    correlation_id: "mission-draft-create-1",
    expected_revision: 4,
    proposed_goal: "Turn selected ad hoc work into a focused mission.",
    selected_ad_hoc_ids: ["ADHOC-000001"],
    excluded_ad_hoc_ids: ["ADHOC-000002"],
    new_work_items: ["Add a durable confirmation path."],
    dependencies: ["Issue 10 approvals remain authoritative."],
    unresolved_decisions: ["Choose final queue grouping."],
    mission_id: "command-deck",
  };
  const acknowledgement = {
    correlation_id: request.correlation_id,
    outcome: "acknowledged" as const,
    revision: 5,
    draft_id: "mission-draft-command-deck-000001",
    draft_status: "draft" as const,
    effect_summary: "Mission Draft created; accepted Mission state is unchanged.",
    accepted_issue_id: "",
  };
  vi.mocked(invoke).mockResolvedValueOnce(acknowledgement);

  const result = await new TauriWorkspaceClient().submitMissionDraftCreate(request);

  expect(invoke).toHaveBeenCalledWith("mission_draft_create", { request });
  expect(result).toEqual({ kind: "acknowledged", acknowledgement });
});

test("submits a Workspace Queue decision through Tauri", async () => {
  const request = {
    correlation_id: "queue-reject-2",
    expected_revision: 2,
    item_id: "issue-change-command-deck-ISS-01-000001",
    decision: "reject" as const,
    reason: "Keep the accepted boundary unchanged.",
  };
  const acknowledgement = {
    correlation_id: request.correlation_id,
    outcome: "acknowledged" as const,
    revision: 3,
    item_id: request.item_id,
    item_status: "rejected" as const,
    effect_summary: "Rejected; accepted Mission state is unchanged.",
  };
  vi.mocked(invoke).mockResolvedValueOnce(acknowledgement);

  const result = await new TauriWorkspaceClient().submitWorkspaceQueueDecision(request);

  expect(invoke).toHaveBeenCalledWith("workspace_queue_decision", { request });
  expect(result).toEqual({ kind: "acknowledged", acknowledgement });
});

test("submits a Workstation action through Tauri", async () => {
  const request = {
    correlation_id: "workstation-launch-1",
    action_type: "issue-launch" as const,
    actor: "mission-commander" as const,
    expected_revision: 2,
    target: { kind: "issue-slice" as const, id: "ISS-01" },
    issue_id: "ISS-01",
    allowed_paths: ["src"],
  };
  const acknowledgement = {
    correlation_id: request.correlation_id,
    outcome: "acknowledged" as const,
    revision: 3,
    action_type: "issue-launch" as const,
    issue_id: "ISS-01",
    session_id: "session-ISS-01-1",
    effect_summary: "Orchestrator launched ISS-01 as session-ISS-01-1.",
  };
  vi.mocked(invoke).mockResolvedValueOnce(acknowledgement);

  const result = await new TauriWorkspaceClient().submitWorkstationAction(request);

  expect(invoke).toHaveBeenCalledWith("workstation_action", { request });
  expect(result).toEqual({ kind: "acknowledged", acknowledgement });
});

test("maps stale Workstation actions separately from rejected actions", async () => {
  vi.mocked(invoke).mockRejectedValueOnce({
    code: "stale-action",
    message: "Workspace action expected revision 2, but the current revision is 3.",
    recoverable: true,
    current_revision: 3,
  });

  const result = await new TauriWorkspaceClient().submitWorkstationAction({
    correlation_id: "workstation-launch-stale-1",
    action_type: "issue-launch",
    actor: "mission-commander",
    expected_revision: 2,
    target: { kind: "issue-slice", id: "ISS-01" },
    issue_id: "ISS-01",
  });

  expect(result).toEqual({
    kind: "stale",
    code: "stale-action",
    message: "Workspace action expected revision 2, but the current revision is 3.",
    current_revision: 3,
  });
});

test("runs an acknowledged queued workstation session through the isolated bridge command", async () => {
  const request = { session_id: "session-ISS-01-1", mission_id: "command-deck" };
  const session = {
    schema_version: 1 as const,
    mission_id: "command-deck",
    session_id: request.session_id,
    issue_id: "ISS-01",
    status: "evidence-ready",
    runner_started_at: "2026-07-10T08:00:00Z",
    runner_ended_at: "2026-07-10T08:00:01Z",
    runner_exit_status: 0,
    evidence_valid: true,
  };
  vi.mocked(invoke).mockResolvedValueOnce(session);

  const result = await new TauriWorkspaceClient().runWorkstationSession(request);

  expect(invoke).toHaveBeenCalledWith("workstation_session_run", { request });
  expect(result).toEqual({ kind: "session-finished", session });
});

test("loads a bounded session artifact through the typed Tauri command", async () => {
  const request = {
    mission_id: "command-deck",
    session_id: "session-ISS-01-1",
    artifact_ref:
      "app-local://missions/command-deck/sessions/session-ISS-01-1/artifacts/review_diff/review.diff",
  };
  const artifact = {
    schema_version: 1 as const,
    mission_id: request.mission_id,
    session_id: request.session_id,
    artifact_id: "review_diff",
    label: "Review diff",
    media_type: "text/x-diff",
    content: "--- a/app.py\n+++ b/app.py\n+fixed\n",
    byte_count: 36,
    content_limit_bytes: 128000,
    truncated: false,
  };
  vi.mocked(invoke).mockResolvedValueOnce(artifact);

  const result = await new TauriWorkspaceClient().loadSessionArtifact(request);

  expect(invoke).toHaveBeenCalledWith("session_artifact", { request });
  expect(result).toEqual({ kind: "session-artifact", artifact });
  expect(result).not.toHaveProperty("artifact.path");
});

test("preserves a structured bounded artifact read failure for inline retry", async () => {
  vi.mocked(invoke).mockRejectedValueOnce({
    code: "session-artifact-not-found",
    message: "The evidence reference is not registered for this Local Agent session.",
    recoverable: true,
  });

  const result = await new TauriWorkspaceClient().loadSessionArtifact({
    mission_id: "command-deck",
    session_id: "session-ISS-01-1",
    artifact_ref: "app-local://evidence/missing",
  });

  expect(result).toEqual({
    kind: "session-artifact-failure",
    code: "session-artifact-not-found",
    message: "The evidence reference is not registered for this Local Agent session.",
    recoverable: true,
  });
});

test("submits a Review Workspace decision through Tauri", async () => {
  const request = {
    correlation_id: "review-accept-4",
    action_type: "review-decision" as const,
    actor: "mission-commander" as const,
    expected_revision: 4,
    target: { kind: "agent-session" as const, id: "session-ISS-01-1" },
    session_id: "session-ISS-01-1",
    decision: "accept" as const,
    reason: "Evidence satisfies the Issue Slice.",
  };
  const acknowledgement = {
    correlation_id: request.correlation_id,
    outcome: "acknowledged" as const,
    revision: 5,
    issue_id: "ISS-01",
    session_id: request.session_id,
    review_outcome: "Approved",
    next_action: "prepare-pr",
    issue_lifecycle: "Complete",
    effect_summary: "Issue Slice becomes Complete and PR-ready; it is not marked merged.",
  };
  vi.mocked(invoke).mockResolvedValueOnce(acknowledgement);

  const result = await new TauriWorkspaceClient().submitReviewDecision(request);

  expect(invoke).toHaveBeenCalledWith("review_decision", { request });
  expect(result).toEqual({ kind: "acknowledged", acknowledgement });
});

test("maps incomplete Review Workspace decisions as backend rejections", async () => {
  vi.mocked(invoke).mockRejectedValueOnce({
    code: "evidence-incomplete",
    message: "Evidence Package is missing: changed_files",
    recoverable: true,
  });

  const result = await new TauriWorkspaceClient().submitReviewDecision({
    correlation_id: "review-accept-incomplete-4",
    action_type: "review-decision",
    actor: "mission-commander",
    expected_revision: 4,
    target: { kind: "agent-session", id: "session-ISS-01-1" },
    session_id: "session-ISS-01-1",
    decision: "accept",
    reason: "Looks fine.",
  });

  expect(result).toEqual({
    kind: "rejected",
    code: "evidence-incomplete",
    message: "Evidence Package is missing: changed_files",
  });
});
