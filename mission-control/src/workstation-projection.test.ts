import type { WorkspaceSnapshot } from "./contracts";
import { projectWorkstationCards } from "./workstation-projection";

const baseSnapshot: WorkspaceSnapshot = {
  schema_version: 1,
  revision: 12,
  workspace_session: {
    id: "workspace-command-deck",
    workspace_path: "/workspace/albert",
    status: "ready",
  },
  active_mission: {
    id: "command-deck",
    title: "Command Deck Mission",
    issue_count: 2,
  },
  conversation_scope: {
    kind: "mission",
    target_id: "command-deck",
    label: "Command Deck Mission",
  },
  operations_view: "mission-board",
  mission_board: {
    prd_title: "Command Deck Mission",
    issue_count: 2,
    ordered_issue_ids: ["ISS-01", "ISS-02"],
    ready_issue_ids: ["ISS-01"],
    approved_issue_ids: ["ISS-01"],
    issue_slices: [
      {
        issue_id: "ISS-01",
        title: "Build prompt shell",
        lifecycle: "Approved",
        progress: "Runner streaming edits",
        launch_eligible: false,
        blockers: [],
        accepted_boundary: {
          what_to_build: "Build the prompt-dominant shell.",
          acceptance_criteria: ["Prompt shell renders."],
          evidence_requirements: ["Tests pass."],
          source_path: ".agent/issues/22-build-prompt-dominant-workstation-shell.md",
        },
        sessions: [
          {
            session_id: "session-ISS-01-1",
            assigned_agent: "qwen-coder-local",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            status: "launched",
            stale: false,
            disconnected: true,
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
          operation_status: "streaming",
          failure: "",
        },
        evidence: {
          state: "missing",
          changed_files: ["mission-control/src/App.tsx", "mission-control/src/styles.css"],
          commands_run: ["npm test -- workstation-projection.test.ts"],
          test_results: "No evidence package recorded.",
          risks: "None recorded.",
          artifact_links: [],
        },
        working_context_sources: [],
      },
      {
        issue_id: "ISS-02",
        title: "Review evidence",
        lifecycle: "Complete",
        progress: "Evidence accepted and PR-ready",
        launch_eligible: false,
        blockers: [],
        accepted_boundary: {
          what_to_build: "Review evidence packages.",
          acceptance_criteria: ["Evidence accepted."],
          evidence_requirements: ["Review decision recorded."],
          source_path: ".agent/issues/08-review-evidence-packages.md",
        },
        sessions: [
          {
            session_id: "session-ISS-02-1",
            assigned_agent: "frontier-reviewer",
            role: "frontier-reviewer",
            provider: "remote",
            model: "frontier-reviewer",
            status: "reviewed",
            stale: false,
            disconnected: false,
            operation_status: "completed",
            failure: "",
          },
        ],
        provenance: {
          role: "frontier-reviewer",
          provider: "remote",
          model: "frontier-reviewer",
        },
        model_assignment: {
          agent_id: "frontier-reviewer",
          role: "frontier-reviewer",
          provider: "remote",
          model: "frontier-reviewer",
          availability: "available",
          availability_reason: "",
          operation_status: "completed",
          failure: "",
        },
        evidence: {
          state: "accepted",
          changed_files: ["docs/review.md"],
          commands_run: [],
          test_results: "Review tests passed.",
          risks: "None recorded.",
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
      issue_count: 2,
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
        {
          session_id: "session-ISS-02-1",
          issue_id: "ISS-02",
          assigned_agent: "frontier-reviewer",
          status: "reviewed",
          role: "frontier-reviewer",
          provider: "remote",
          model: "frontier-reviewer",
        },
      ],
      attention: [
        {
          attention_id: "delegation-command-deck-ISS-03",
          mission_id: "command-deck",
          kind: "delegation-approval",
          label: "ISS-03 delegation approval required",
          queue_link: "workspace-queue#delegation-command-deck-ISS-03",
        },
      ],
    },
  ],
};

test("projects compact live workstation cards from canonical snapshot state", () => {
  const projection = projectWorkstationCards(baseSnapshot);

  expect(projection.revision).toBe(12);
  expect(projection.groups.map((group) => group.id)).toEqual(["active", "done"]);
  expect(projection.groups[0].cards.map((card) => card.id)).toEqual([
    "attention:delegation-command-deck-ISS-03",
    "session:session-ISS-01-1",
  ]);

  const waiting = projection.groups[0].cards[0];
  expect(waiting).toMatchObject({
    name: "ISS-03 delegation approval required",
    status: "waiting-approval",
    phase: "Approval",
    approvalBlockers: ["ISS-03 delegation approval required"],
    nextAction: "Open Workspace Queue",
    acceptedRevision: 12,
  });

  const running = projection.groups[0].cards[1];
  expect(running).toMatchObject({
    name: "qwen-coder-local",
    model: "qwen3.6:27b",
    role: "local-agent",
    currentTask: "Build prompt shell",
    status: "running",
    phase: "streaming",
    progress: "Runner streaming edits",
    filesTouched: 2,
    latestCommandOrTest: "npm test -- workstation-projection.test.ts",
    nextAction: "Runner streaming edits",
    acceptedRevision: 12,
  });

  expect(projection.groups[1].cards[0]).toMatchObject({
    name: "frontier-reviewer",
    status: "done",
    phase: "completed",
    latestCommandOrTest: "Review tests passed.",
  });
});

test("keeps frontend-only pending intent separate from accepted workstation state", () => {
  const projection = projectWorkstationCards(baseSnapshot, {
    pendingIntent: {
      id: "launch-ISS-99",
      label: "Launch ISS-99",
      expectedRevision: 12,
    },
  });

  expect(projection.pendingIntent).toEqual({
    id: "launch-ISS-99",
    label: "Launch ISS-99",
    expectedRevision: 12,
  });
  expect(projection.groups.flatMap((group) => group.cards).map((card) => card.currentTask)).not.toContain(
    "Launch ISS-99",
  );
  expect(projection.groups.flatMap((group) => group.cards).every((card) => card.acceptedRevision === 12)).toBe(
    true,
  );
});

test("exposes waiting approval card decisions only from pending queue projection", () => {
  const pendingItem = {
    item_id: "delegation-command-deck-ISS-03",
    mission_id: "command-deck",
    item_type: "ad-hoc-delegation" as const,
    status: "pending" as const,
    source: "agent-console",
    requested_action: "Approve delegated work",
    affected_boundary: "launch-boundary",
    consequence: "Approval will launch the local agent session.",
    issue_id: "ADHOC-000001",
    proposed_changes: {},
  };
  const projection = projectWorkstationCards(baseSnapshot, {
    workspaceQueue: {
      schema_version: 1,
      revision: 4,
      items: [pendingItem],
      groups: [
        {
          group_id: "ad-hoc-delegation:command-deck",
          item_type: "ad-hoc-delegation",
          mission_id: "command-deck",
          item_count: 1,
          items: [pendingItem],
        },
      ],
    },
  });

  const waiting = projection.groups[0].cards[0];
  expect(waiting.detail.governedActions).toEqual([
    {
      label: "Approve",
      target: "workspace-queue",
      requiresReason: false,
      actionType: "workspace-queue-decision",
      actor: "mission-commander",
      itemId: "delegation-command-deck-ISS-03",
      decision: "approve",
      targetIdentity: {
        kind: "workspace-queue-item",
        id: "delegation-command-deck-ISS-03",
      },
    },
    {
      label: "Reject",
      target: "workspace-queue",
      requiresReason: true,
      actionType: "workspace-queue-decision",
      actor: "mission-commander",
      itemId: "delegation-command-deck-ISS-03",
      decision: "reject",
      targetIdentity: {
        kind: "workspace-queue-item",
        id: "delegation-command-deck-ISS-03",
      },
    },
    {
      label: "Defer",
      target: "workspace-queue",
      requiresReason: true,
      actionType: "workspace-queue-decision",
      actor: "mission-commander",
      itemId: "delegation-command-deck-ISS-03",
      decision: "defer",
      targetIdentity: {
        kind: "workspace-queue-item",
        id: "delegation-command-deck-ISS-03",
      },
    },
  ]);

  const approvedProjection = projectWorkstationCards(baseSnapshot, {
    workspaceQueue: {
      schema_version: 1,
      revision: 5,
      items: [{ ...pendingItem, status: "approved" }],
      groups: [],
    },
  });

  expect(approvedProjection.groups[0].cards[0].detail.governedActions).toEqual([
    {
      label: "Open Workspace Queue",
      target: "workspace-queue",
      requiresReason: false,
    },
  ]);
});

test("describes the broader governed workstation action family with typed metadata", () => {
  const projection = projectWorkstationCards({
    ...baseSnapshot,
    revision: 20,
    mission_board: {
      ...baseSnapshot.mission_board,
      ordered_issue_ids: ["ISS-01", "ISS-02", "ISS-04"],
      ready_issue_ids: ["ISS-04"],
      issue_slices: [
        ...(baseSnapshot.mission_board.issue_slices ?? []),
        {
          issue_id: "ISS-04",
          title: "Launch guarded worker",
          lifecycle: "Ready",
          progress: "Ready for Mission Commander launch",
          launch_eligible: true,
          blockers: [],
          accepted_boundary: {
            what_to_build: "Launch guarded worker.",
            acceptance_criteria: ["Launch uses Orchestrator validation."],
            evidence_requirements: ["Session evidence is recorded."],
            source_path: ".scratch/command-deck/issues/04-launch-guarded-worker.md",
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
            risks: "None recorded.",
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
            status: "running",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
          {
            session_id: "session-ISS-02-1",
            issue_id: "ISS-02",
            assigned_agent: "frontier-reviewer",
            status: "evidence-ready",
            role: "frontier-reviewer",
            provider: "remote",
            model: "frontier-reviewer",
          },
        ],
        attention: [],
      },
    ],
  });

  const cards = projection.groups.flatMap((group) => group.cards);
  const launchCard = cards.find((card) => card.issueId === "ISS-04");
  expect(launchCard?.detail.governedActions).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        actionType: "issue-launch",
        actor: "mission-commander",
        issueId: "ISS-04",
        expectedRevision: 20,
        targetIdentity: { kind: "issue-slice", id: "ISS-04" },
      }),
      expect.objectContaining({
        actionType: "model-assignment-change",
        actor: "mission-commander",
        issueId: "ISS-04",
        expectedRevision: 20,
        targetIdentity: { kind: "issue-slice", id: "ISS-04" },
      }),
    ]),
  );

  const runningCard = cards.find((card) => card.sessionId === "session-ISS-01-1");
  expect(runningCard?.detail.governedActions).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        actionType: "session-cancel",
        sessionId: "session-ISS-01-1",
        targetIdentity: { kind: "agent-session", id: "session-ISS-01-1" },
      }),
    ]),
  );

  const reviewCard = cards.find((card) => card.sessionId === "session-ISS-02-1");
  expect(reviewCard?.detail.governedActions).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        actionType: "review-decision",
        reviewDecision: "accept",
        sessionId: "session-ISS-02-1",
      }),
      expect.objectContaining({
        actionType: "review-decision",
        reviewDecision: "repair",
        requiresReason: true,
      }),
      expect.objectContaining({
        actionType: "review-decision",
        reviewDecision: "escalate-human",
      }),
    ]),
  );
});

test("projects cancelled sessions as done instead of active work", () => {
  const projection = projectWorkstationCards({
    ...baseSnapshot,
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
            status: "cancelled",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
        ],
        attention: [],
      },
    ],
  });

  const card = projection.groups.flatMap((group) => group.cards).find((item) => item.sessionId === "session-ISS-01-1");

  expect(card?.status).toBe("done");
  expect(card?.nextAction).toBe("Review accepted evidence");
  expect(card?.detail.governedActions).toEqual([
    { label: "Open Activity", target: "activity", requiresReason: false },
  ]);
});

test("projects expanded operational detail from canonical issue and session evidence", () => {
  const projection = projectWorkstationCards({
    ...baseSnapshot,
    mission_board: {
      ...baseSnapshot.mission_board,
      issue_slices: baseSnapshot.mission_board.issue_slices?.map((issue) =>
        issue.issue_id === "ISS-01"
          ? {
              ...issue,
              evidence: {
                ...issue.evidence,
                artifact_links: ["app-local://evidence/session-ISS-01-1"],
                risks: "Review generated CSS responsiveness.",
              },
            }
          : issue,
      ),
    },
  });

  const card = projection.groups
    .flatMap((group) => group.cards)
    .find((item) => item.sessionId === "session-ISS-01-1");

  expect(card?.detail).toMatchObject({
    originatingSessionId: "session-ISS-01-1",
    issueId: "ISS-01",
    toolActivity: [
      {
        kind: "command-summary",
        label: "Command",
        summary: "npm test -- workstation-projection.test.ts",
      },
      {
        kind: "operation-summary",
        label: "Provider operation",
        summary: "streaming",
      },
    ],
    filesTouched: [
      { path: "mission-control/src/App.tsx", status: "touched" },
      { path: "mission-control/src/styles.css", status: "touched" },
    ],
    diffs: [
      {
        label: "Diff mission-control/src/App.tsx",
        path: "mission-control/src/App.tsx",
        href: "app-local://diffs/session-ISS-01-1?path=mission-control%2Fsrc%2FApp.tsx",
        sessionId: "session-ISS-01-1",
      },
      {
        label: "Diff mission-control/src/styles.css",
        path: "mission-control/src/styles.css",
        href: "app-local://diffs/session-ISS-01-1?path=mission-control%2Fsrc%2Fstyles.css",
        sessionId: "session-ISS-01-1",
      },
    ],
    evidenceLinks: [
      {
        label: "Evidence Package session-ISS-01-1",
        href: "app-local://evidence/session-ISS-01-1",
        sessionId: "session-ISS-01-1",
      },
    ],
    terminalExcerpts: [
      {
        label: "Command summary",
        excerpt: "npm test -- workstation-projection.test.ts",
        sessionId: "session-ISS-01-1",
      },
      {
        label: "Test summary",
        excerpt: "No evidence package recorded.",
        sessionId: "session-ISS-01-1",
      },
    ],
    reviewState: {
      evidenceState: "missing",
      lifecycle: "Approved",
      risks: "Review generated CSS responsiveness.",
      reviewReady: false,
    },
  });
  expect(card?.detail.governedActions.map((action) => action.label)).toEqual([
    "Cancel session",
    "Monitor active work",
  ]);
});

test("updates accepted cards only when the canonical snapshot advances", () => {
  const pending = projectWorkstationCards(baseSnapshot, {
    pendingIntent: {
      id: "launch-ISS-01",
      label: "Launch ISS-01",
      expectedRevision: 13,
    },
  });

  expect(pending.pendingIntent?.expectedRevision).toBe(13);
  expect(pending.groups.flatMap((group) => group.cards).map((card) => card.acceptedRevision)).toEqual([
    12,
    12,
    12,
  ]);

  const acknowledgedSnapshot: WorkspaceSnapshot = {
    ...baseSnapshot,
    revision: 13,
    mission_board: {
      ...baseSnapshot.mission_board,
      issue_slices: baseSnapshot.mission_board.issue_slices?.map((issue) =>
        issue.issue_id === "ISS-01"
          ? {
              ...issue,
              progress: "Evidence package ready for review",
              sessions: issue.sessions.map((session) => ({
                ...session,
                status: "evidence-ready",
                disconnected: false,
                operation_status: "completed",
              })),
              evidence: {
                ...issue.evidence,
                commands_run: ["npm test -- --run"],
                test_results: "90 tests passed.",
              },
            }
          : issue,
      ),
    },
    missions: baseSnapshot.missions?.map((mission) => ({
      ...mission,
      attention: [],
      sessions: mission.sessions.map((session) =>
        session.session_id === "session-ISS-01-1"
          ? { ...session, status: "evidence-ready" }
          : session,
      ),
    })),
  };

  const acknowledged = projectWorkstationCards(acknowledgedSnapshot);
  const card = acknowledged.groups
    .flatMap((group) => group.cards)
    .find((item) => item.sessionId === "session-ISS-01-1");

  expect(acknowledged.pendingIntent).toBeNull();
  expect(card).toMatchObject({
    status: "review-ready",
    acceptedRevision: 13,
    latestCommandOrTest: "npm test -- --run",
  });
});

test("does not create live workstation cards from issue board fallback state", () => {
  const projection = projectWorkstationCards({
    ...baseSnapshot,
    missions: undefined,
  });

  expect(projection.groups).toEqual([]);
});

test("splits many live cards by mission while retaining active and done buckets", () => {
  const projection = projectWorkstationCards({
    ...baseSnapshot,
    missions: [
      ...(baseSnapshot.missions ?? []),
      {
        id: "background-mission",
        title: "Background Mission",
        issue_count: 1,
        is_active: false,
        sessions: [
          {
            session_id: "session-BG-01-1",
            issue_id: "BG-01",
            assigned_agent: "lint-subagent",
            status: "running",
            role: "subagent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
          },
        ],
        attention: [],
      },
    ],
  });

  expect(projection.groups.map((group) => group.id)).toEqual([
    "active:command-deck",
    "active:background-mission",
    "done:command-deck",
  ]);
  expect(projection.groups[1].cards[0]).toMatchObject({
    name: "lint-subagent",
    role: "subagent",
    model: "qwen2.5-coder:14b",
  });
});

test("maps canonical session states to every workstation status", () => {
  const cases = [
    ["queued", "idle", "", "thinking"],
    ["launched", "streaming", "", "running"],
    ["pending-approval", "idle", "", "waiting-approval"],
    ["needs-repair", "idle", "", "blocked"],
    ["needs-review", "idle", "", "reviewing"],
    ["evidence-ready", "completed", "", "review-ready"],
    ["reviewed", "completed", "", "done"],
    ["launched", "idle", "Provider failed.", "failed"],
  ] as const;

  for (const [sessionStatus, operationStatus, failure, expected] of cases) {
    const snapshot: WorkspaceSnapshot = {
      ...baseSnapshot,
      mission_board: {
        ...baseSnapshot.mission_board,
        issue_slices: baseSnapshot.mission_board.issue_slices?.map((issue) =>
          issue.issue_id === "ISS-01"
            ? {
                ...issue,
                sessions: issue.sessions.map((session) => ({
                  ...session,
                  status: sessionStatus,
                  operation_status: operationStatus,
                  failure,
                })),
              }
            : issue,
        ),
      },
      missions: baseSnapshot.missions?.map((mission) => ({
        ...mission,
        attention: [],
        sessions: mission.sessions
          .filter((session) => session.session_id === "session-ISS-01-1")
          .map((session) => ({ ...session, status: sessionStatus })),
      })),
    };

    const [card] = projectWorkstationCards(snapshot).groups.flatMap((group) => group.cards);

    expect(card.status).toBe(expected);
  }
});
