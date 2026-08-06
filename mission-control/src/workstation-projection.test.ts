import type { WorkspaceIssueSliceSummary, WorkspaceSnapshot } from "./contracts";
import {
  projectIssueAssignmentBoard,
  projectMissionExecutionTree,
  projectWorkstationCards,
  workstationActionConsequence,
} from "./workstation-projection";

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
        work_type: "AFK",
        tracker_status: "ready-for-agent",
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
        work_type: "AFK",
        tracker_status: "complete",
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
          last_activity_at: "2026-07-12T08:31:45+00:00",
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
          entity_id: "ADHOC-000001",
          queue_item_id: "delegation-command-deck-ISS-03",
        },
      ],
    },
  ],
};

function issueSlice(
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
      evidence_requirements: ["Projection tests pass."],
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

test("projects compact live workstation cards from canonical snapshot state", () => {
  const projection = projectWorkstationCards(baseSnapshot);

  expect(projection.revision).toBe(12);
  expect(projection.groups.map((group) => group.id)).toEqual(["active", "done"]);
  expect(projection.groups[0].cards.map((card) => card.id)).toEqual([
    "attention:command-deck:delegation-command-deck-ISS-03",
    "session:command-deck:session-ISS-01-1",
  ]);

  const waiting = projection.groups[0].cards[0];
  expect(waiting).toMatchObject({
    name: "ISS-03 delegation approval required",
    status: "waiting-approval",
    phase: "Approval",
    lastActivity: "",
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
    lastActivity: "2026-07-12T08:31:45+00:00",
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

test("projects a work-centered Mission Execution Tree from canonical work records", () => {
  const treeSnapshot: WorkspaceSnapshot = {
    ...baseSnapshot,
    revision: 19,
    mission_board: {
      ...baseSnapshot.mission_board,
      ordered_issue_ids: ["ISS-01"],
      issue_count: 1,
      issue_slices: [
        {
          ...baseSnapshot.mission_board.issue_slices![0],
          issue_id: "ISS-01",
          title: "Build prompt shell",
          lifecycle: "Blocked",
          blockers: [
            {
              issue_id: "ISS-00",
              title: "Choose the workspace",
              lifecycle: "Ready",
              satisfied: false,
            },
          ],
          sessions: [
            ...baseSnapshot.mission_board.issue_slices![0].sessions,
            {
              session_id: "session-ISS-01-2",
              assigned_agent: "repair-agent",
              role: "local-agent",
              provider: "ollama",
              model: "gemma4:12b",
              status: "queued",
              stale: false,
              disconnected: false,
              operation_status: "queued",
              failure: "",
            },
          ],
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
            session_id: "session-ISS-01-1",
            issue_id: "ISS-01",
            assigned_agent: "qwen-coder-local",
            status: "launched",
            last_activity_at: "2026-07-12T08:31:45+00:00",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
            task_title: "Build prompt shell",
            operation_status: "streaming",
            failure: "",
            changed_files: ["src/App.tsx"],
            commands_run: ["npm test"],
            test_results: "Tests are running.",
            risks: "",
            artifact_links: [],
          },
          {
            session_id: "session-ISS-01-2",
            issue_id: "ISS-01",
            assigned_agent: "repair-agent",
            status: "queued",
            last_activity_at: "",
            role: "local-agent",
            provider: "ollama",
            model: "gemma4:12b",
            task_title: "Repair prompt shell",
            operation_status: "queued",
            failure: "",
            changed_files: [],
            commands_run: [],
            test_results: "",
            risks: "",
            artifact_links: [],
            work_kind: "issue-slice",
            parent_session_id: "session-ISS-01-1",
          },
          {
            session_id: "session-ADHOC-000001-1",
            issue_id: "ADHOC-000001",
            assigned_agent: "docs-agent",
            status: "running",
            last_activity_at: "2026-07-12T08:35:45+00:00",
            role: "local-agent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
            task_title: "Inspect prompt accessibility",
            operation_status: "streaming",
            failure: "",
            changed_files: ["docs/console.md"],
            commands_run: ["npm test -- accessibility"],
            test_results: "Accessibility checks are running.",
            risks: "",
            artifact_links: [],
            work_kind: "ad-hoc-delegation",
          },
        ],
        attention: [
          {
            attention_id: "ad-hoc-delegation-command-deck-000001",
            mission_id: "command-deck",
            kind: "ad-hoc-delegation",
            label: "ADHOC-000001 Ad Hoc Delegation pending",
            queue_link: "workspace-queue#ad-hoc-delegation-command-deck-000001",
            entity_id: "ADHOC-000001",
            queue_item_id: "ad-hoc-delegation-command-deck-000001",
          },
        ],
      },
    ],
  };

  const projection = projectMissionExecutionTree(treeSnapshot);
  const node = (id: string) => projection.nodes.find((candidate) => candidate.id === id);

  expect(projection.counts).toEqual({
    issue_slices: 1,
    ad_hoc_delegations: 1,
    local_agent_sessions: 3,
    repairs: 1,
    blockers: 1,
    evidence_packages: 0,
  });
  expect(projection.root_id).toBe("mission:command-deck");
  expect(node("mission:command-deck")).toMatchObject({
    kind: "mission",
    title: "Command Deck Mission",
    child_ids: ["issue:command-deck:ISS-01", "ad-hoc:command-deck:ADHOC-000001"],
  });
  expect(node("issue:command-deck:ISS-01")).toMatchObject({
    kind: "issue-slice",
    identity: "ISS-01",
    title: "Build prompt shell",
    state: "blocked",
    child_ids: [
      "session:command-deck:session-ISS-01-1",
    ],
  });
  expect(node("ad-hoc:command-deck:ADHOC-000001")).toMatchObject({
    kind: "ad-hoc-delegation",
    identity: "ADHOC-000001",
    title: "Inspect prompt accessibility",
    child_ids: ["session:command-deck:session-ADHOC-000001-1"],
  });
  expect(node("session:command-deck:session-ISS-01-2")).toMatchObject({
    kind: "agent-session",
    state: "queued",
    lineage: "repair",
    shape: "repair",
    parent_id: "session:command-deck:session-ISS-01-1",
    parent_session_id: "session-ISS-01-1",
  });
  expect(node("session:command-deck:session-ISS-01-1")).toMatchObject({
    child_ids: ["session:command-deck:session-ISS-01-2"],
  });
  expect(node("session:command-deck:session-ADHOC-000001-1")).toMatchObject({
    kind: "agent-session",
    state: "working",
    parent_id: "ad-hoc:command-deck:ADHOC-000001",
    identity: "session-ADHOC-000001-1",
  });
});

test("fails safe by surfacing cyclic repair sessions as root-owned nodes", () => {
  const snapshot: WorkspaceSnapshot = {
    ...baseSnapshot,
    mission_board: {
      ...baseSnapshot.mission_board,
      ordered_issue_ids: ["ISS-01"],
      issue_count: 1,
      issue_slices: [
        {
          ...baseSnapshot.mission_board.issue_slices![0],
          sessions: [],
        },
      ],
    },
    missions: [
      {
        ...baseSnapshot.missions![0],
        issue_count: 1,
        sessions: [
          {
            session_id: "session-cycle-a",
            issue_id: "ISS-01",
            assigned_agent: "repair-a",
            status: "queued",
            parent_session_id: "session-cycle-b",
          },
          {
            session_id: "session-cycle-b",
            issue_id: "ISS-01",
            assigned_agent: "repair-b",
            status: "failed",
            parent_session_id: "session-cycle-a",
          },
        ],
        attention: [],
      },
    ],
  };

  const projection = projectMissionExecutionTree(snapshot);
  const node = (id: string) => projection.nodes.find((candidate) => candidate.id === id);

  expect(node("issue:command-deck:ISS-01")?.child_ids).toEqual([
    "session:command-deck:session-cycle-a",
    "session:command-deck:session-cycle-b",
  ]);
  expect(node("session:command-deck:session-cycle-a")).toMatchObject({
    parent_id: "issue:command-deck:ISS-01",
    lineage: "root",
    state: "queued",
  });
  expect(node("session:command-deck:session-cycle-b")).toMatchObject({
    parent_id: "issue:command-deck:ISS-01",
    lineage: "root",
    state: "failed",
  });
  expect(node("session:command-deck:session-cycle-a")?.child_ids).toEqual([]);
  expect(node("session:command-deck:session-cycle-b")?.child_ids).toEqual([]);
});

test("uses typed Ad Hoc identity instead of display labels or issue-id patterns", () => {
  const snapshot: WorkspaceSnapshot = {
    ...baseSnapshot,
    mission_board: {
      ...baseSnapshot.mission_board,
      ordered_issue_ids: [],
      issue_count: 0,
      issue_slices: [],
    },
    missions: [
      {
        ...baseSnapshot.missions![0],
        issue_count: 0,
        sessions: [
          {
            session_id: "session-ADHOC-LOOKALIKE-1",
            issue_id: "ADHOC-LOOKALIKE",
            assigned_agent: "issue-worker",
            status: "queued",
            work_kind: "issue-slice",
          },
        ],
        attention: [
          {
            attention_id: "attention-canonical-ad-hoc",
            mission_id: "command-deck",
            kind: "ad-hoc-delegation",
            label: "A display label unrelated to identity",
            queue_link: "workspace-queue#not-the-entity",
            entity_id: "ADHOC-CANONICAL",
            queue_item_id: "queue-canonical",
          },
        ],
      },
    ],
  };

  const projection = projectMissionExecutionTree(snapshot);

  expect(projection.nodes.map((node) => node.id)).toContain(
    "ad-hoc:command-deck:ADHOC-CANONICAL",
  );
  expect(projection.nodes.map((node) => node.id)).not.toContain(
    "ad-hoc:command-deck:A display label unrelated to identity",
  );
  expect(projection.nodes.map((node) => node.id)).not.toContain(
    "ad-hoc:command-deck:ADHOC-LOOKALIKE",
  );
  expect(projection.nodes.map((node) => node.id)).toContain(
    "session:command-deck:session-ADHOC-LOOKALIKE-1",
  );
});

test("describes governed action consequences independently of recovery guidance", () => {
  const action = projectWorkstationCards(baseSnapshot)
    .groups
    .flatMap((group) => group.cards)
    .find((card) => card.sessionId === "session-ISS-01-1")
    ?.detail.governedActions.find((candidate) => candidate.actionType === "session-cancel");

  expect(action).toBeDefined();
  expect(workstationActionConsequence(action!)).toContain("records cancellation");
  expect(workstationActionConsequence(action!)).not.toContain("recovery");
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

test("projects canonical review repair launches for Issue Slice and Ad Hoc sessions", () => {
  const projection = projectWorkstationCards({
    ...baseSnapshot,
    revision: 21,
    mission_board: {
      ...baseSnapshot.mission_board,
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
        sessions: [
          {
            session_id: "session-ISS-REPAIR-1",
            issue_id: "ISS-REPAIR",
            assigned_agent: "issue-worker",
            status: "evidence-ready",
            task_title: "Repair accepted issue work",
            review_outcome: "Needs repair",
            review_next_action: "same-local-agent-repair",
            repair_action_available: true,
          },
          {
            session_id: "session-ADHOC-000001-1",
            issue_id: "ADHOC-000001",
            assigned_agent: "ad-hoc-worker",
            status: "failed",
            task_title: "Repair bounded ad hoc work",
            review_outcome: "Needs repair",
            review_next_action: "same-local-agent-repair",
            repair_action_available: true,
          },
          {
            session_id: "session-ISS-FAILED-1",
            issue_id: "ISS-FAILED",
            assigned_agent: "ordinary-worker",
            status: "failed",
            task_title: "Retry ordinary failed work",
            repair_action_available: false,
          },
        ],
        attention: [],
      },
    ],
  });
  const cards = projection.groups.flatMap((group) => group.cards);

  for (const sessionId of ["session-ISS-REPAIR-1", "session-ADHOC-000001-1"]) {
    const action = cards
      .find((card) => card.sessionId === sessionId)
      ?.detail.governedActions.find((candidate) => candidate.actionType === "issue-retry");
    expect(action).toMatchObject({
      label: "Launch repair",
      actionType: "issue-retry",
      requiresReason: false,
      missionId: "command-deck",
      sessionId,
      expectedRevision: 21,
      targetIdentity: { kind: "agent-session", id: sessionId },
    });
  }

  expect(
    cards
      .find((card) => card.sessionId === "session-ISS-FAILED-1")
      ?.detail.governedActions.find((candidate) => candidate.actionType === "issue-retry"),
  ).toMatchObject({
    label: "Retry",
    requiresReason: true,
  });
});

test("projects issue assignment rows from accepted mission and session state", () => {
  const boardSnapshot: WorkspaceSnapshot = {
    ...baseSnapshot,
    revision: 18,
    conversation_scope: {
      kind: "issue-slice",
      target_id: "ISS-DONE",
      label: "Accepted evidence",
    },
    mission_board: {
      ...baseSnapshot.mission_board,
      issue_count: 7,
      ordered_issue_ids: [
        "ISS-READY",
        "ISS-BLOCKED",
        "ISS-ACTIVE",
        "ISS-REVIEW",
        "ISS-DONE",
        "ISS-MERGED",
        "ISS-FAILED",
      ],
      ready_issue_ids: ["ISS-READY"],
      approved_issue_ids: [
        "ISS-READY",
        "ISS-BLOCKED",
        "ISS-ACTIVE",
        "ISS-REVIEW",
        "ISS-DONE",
        "ISS-FAILED",
      ],
      issue_slices: [
        issueSlice({
          issue_id: "ISS-READY",
          title: "Unassigned ready work",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: true,
        }),
        issueSlice({
          issue_id: "ISS-BLOCKED",
          title: "Blocked dependency work",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          progress: "Waiting on release seam",
          blockers: [
            {
              issue_id: "ISS-000",
              title: "Release seam",
              lifecycle: "Ready",
              satisfied: false,
            },
          ],
        }),
        issueSlice({
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
        issueSlice({
          issue_id: "ISS-REVIEW",
          title: "Evidence ready",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          progress: "Evidence package ready",
          sessions: [
            {
              session_id: "session-ISS-REVIEW-1",
              assigned_agent: "review-subagent",
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
        }),
        issueSlice({
          issue_id: "ISS-DONE",
          title: "Accepted evidence",
          work_type: "AFK",
          tracker_status: "complete",
          lifecycle: "Complete",
          progress: "Evidence accepted and PR-ready",
          evidence: {
            state: "accepted",
            changed_files: [],
            commands_run: [],
            test_results: "Review accepted.",
            risks: "None recorded.",
            artifact_links: [],
          },
        }),
        issueSlice({
          issue_id: "ISS-MERGED",
          title: "Merged evidence",
          work_type: "AFK",
          tracker_status: "merged",
          lifecycle: "Merged",
          progress: "PR merged upstream",
          evidence: {
            state: "accepted",
            changed_files: [],
            commands_run: [],
            test_results: "Review accepted.",
            risks: "None recorded.",
            artifact_links: [],
          },
        }),
        issueSlice({
          issue_id: "ISS-FAILED",
          title: "Provider failed",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          progress: "Provider failure recorded",
          sessions: [
            {
              session_id: "session-ISS-FAILED-1",
              assigned_agent: "repair-agent",
              role: "local-agent",
              provider: "ollama",
              model: "gemma4:12b",
              status: "failed",
              stale: false,
              disconnected: false,
              operation_status: "failed",
              failure: "Provider exited before evidence.",
            },
          ],
        }),
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 7,
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
          {
            session_id: "session-ISS-REVIEW-1",
            issue_id: "ISS-REVIEW",
            assigned_agent: "review-subagent",
            status: "evidence-ready",
            role: "subagent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
          },
          {
            session_id: "session-ISS-FAILED-1",
            issue_id: "ISS-FAILED",
            assigned_agent: "repair-agent",
            status: "failed",
            role: "local-agent",
            provider: "ollama",
            model: "gemma4:12b",
          },
        ],
        attention: [],
      },
    ],
  };

  const board = projectIssueAssignmentBoard(boardSnapshot);

  expect(board.revision).toBe(18);
  expect(board.rows.map((row) => row.issueId)).toEqual([
    "ISS-READY",
    "ISS-BLOCKED",
    "ISS-ACTIVE",
    "ISS-REVIEW",
    "ISS-FAILED",
  ]);
  expect(
    board.rows.map((row) => ({
      issueId: row.issueId,
      owner: row.owner,
      assignmentState: row.assignmentState,
      state: row.state,
      blockerState: row.blockerState,
      workstationSessionId: row.workstationSessionId,
      workstationAgent: row.workstationAgent,
      scopeDisabledReason: row.scopeDisabledReason,
    })),
  ).toEqual([
    {
      issueId: "ISS-READY",
      owner: "Unassigned",
      assignmentState: "unassigned",
      state: "unassigned-ready",
      blockerState: "clear",
      workstationSessionId: null,
      workstationAgent: null,
      scopeDisabledReason: null,
    },
    {
      issueId: "ISS-BLOCKED",
      owner: "Unassigned",
      assignmentState: "unassigned",
      state: "blocked",
      blockerState: "blocked",
      workstationSessionId: null,
      workstationAgent: null,
      scopeDisabledReason: null,
    },
    {
      issueId: "ISS-ACTIVE",
      owner: "qwen-coder-local",
      assignmentState: "active",
      state: "active",
      blockerState: "clear",
      workstationSessionId: "session-ISS-ACTIVE-1",
      workstationAgent: "qwen-coder-local",
      scopeDisabledReason: null,
    },
    {
      issueId: "ISS-REVIEW",
      owner: "review-subagent",
      assignmentState: "active",
      state: "review-ready",
      blockerState: "clear",
      workstationSessionId: "session-ISS-REVIEW-1",
      workstationAgent: "review-subagent",
      scopeDisabledReason: null,
    },
    {
      issueId: "ISS-FAILED",
      owner: "repair-agent",
      assignmentState: "active",
      state: "failed",
      blockerState: "clear",
      workstationSessionId: "session-ISS-FAILED-1",
      workstationAgent: "repair-agent",
      scopeDisabledReason: null,
    },
  ]);
  expect(board.rows[1].blockerSummaries).toEqual(["ISS-000 Ready open - Release seam"]);
});

test("keeps only active AFK work in the Issue Assignment Board", () => {
  const activeAfk = {
    ...issueSlice({
      issue_id: "ISS-22",
      title: "Finish responsive workstation shell",
      tracker_status: "ready-for-agent",
      lifecycle: "Needs review",
    }),
    work_type: "AFK",
  } as WorkspaceIssueSliceSummary;
  const completedAfk = {
    ...issueSlice({
      issue_id: "ISS-20",
      title: "Ship package entrypoint",
      tracker_status: "complete",
      lifecycle: "Merged",
      sessions: [
        {
          session_id: "session-ISS-20-stale",
          assigned_agent: "stale-worker",
          role: "local-agent",
          provider: "ollama",
          model: "stale-model",
          status: "running",
          stale: true,
          disconnected: true,
          operation_status: "streaming",
          failure: "",
        },
      ],
    }),
    work_type: "AFK",
  } as WorkspaceIssueSliceSummary;
  const manualReview = {
    ...issueSlice({
      issue_id: "ISS-28",
      title: "Validate accessibility with a human",
      tracker_status: "ready-for-human",
      lifecycle: "Needs human review",
    }),
    work_type: "HITL",
  } as WorkspaceIssueSliceSummary;
  const mergedAfk = {
    ...issueSlice({
      issue_id: "ISS-29",
      title: "Verify release seam",
      tracker_status: "merged",
      lifecycle: "Merged",
    }),
    work_type: "AFK",
  } as WorkspaceIssueSliceSummary;
  const missingWorkType = issueSlice({
    issue_id: "ISS-30",
    title: "Unknown execution ownership",
    tracker_status: "ready-for-agent",
    lifecycle: "Needs review",
  });
  const missingTrackerStatus = issueSlice({
    issue_id: "ISS-32",
    title: "Unknown tracker lifecycle",
    work_type: "AFK",
    lifecycle: "Approved",
    sessions: [
      {
        session_id: "session-ISS-32-1",
        assigned_agent: "unknown-lifecycle-worker",
        role: "local-agent",
        provider: "ollama",
        model: "unknown-model",
        status: "running",
        stale: false,
        disconnected: false,
        operation_status: "streaming",
        failure: "",
      },
    ],
  });

  const board = projectIssueAssignmentBoard({
    ...baseSnapshot,
    mission_board: {
      ...baseSnapshot.mission_board,
      issue_count: 7,
      ordered_issue_ids: ["ISS-20", "ISS-22", "ISS-28", "ISS-29", "ISS-30", "ISS-31", "ISS-32"],
      ready_issue_ids: ["ISS-31"],
      approved_issue_ids: [],
      issue_slices: [
        completedAfk,
        activeAfk,
        manualReview,
        mergedAfk,
        missingWorkType,
        missingTrackerStatus,
      ],
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: 7,
        is_active: true,
        sessions: [],
        attention: [],
      },
    ],
  });

  expect(board.rows.map((row) => row.issueId)).toEqual(["ISS-22"]);
});

test("Issue Assignment Board follows the latest retry session", () => {
  const oldSession = {
    session_id: "session-ISS-RETRY-2",
    assigned_agent: "old-worker",
    role: "local-agent",
    provider: "ollama",
    model: "gemma4:12b",
    status: "failed",
    stale: false,
    disconnected: false,
    operation_status: "failed",
    failure: "First attempt failed.",
  } as const;
  const currentSession = {
    ...oldSession,
    session_id: "session-ISS-RETRY-10",
    assigned_agent: "repair-worker",
    model: "gemma4:26b",
    status: "running",
    operation_status: "streaming",
    failure: "",
  } as const;
  const [row] = projectIssueAssignmentBoard({
    ...baseSnapshot,
    mission_board: {
      ...baseSnapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-RETRY"],
      ready_issue_ids: [],
      approved_issue_ids: ["ISS-RETRY"],
      issue_slices: [
        issueSlice({
          issue_id: "ISS-RETRY",
          title: "Repair current implementation",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          sessions: [oldSession, currentSession],
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
          { ...oldSession, issue_id: "ISS-RETRY" },
          { ...currentSession, issue_id: "ISS-RETRY" },
        ],
        attention: [],
      },
    ],
  }).rows;

  expect(row).toMatchObject({
    issueId: "ISS-RETRY",
    owner: "repair-worker",
    assignmentState: "active",
    state: "active",
    workstationSessionId: "session-ISS-RETRY-10",
    workstationAgent: "repair-worker",
  });
});

test("projects Issue Assignment Board governed actions only for canonical unassigned ready work", () => {
  const board = projectIssueAssignmentBoard({
    ...baseSnapshot,
    revision: 21,
    mission_board: {
      ...baseSnapshot.mission_board,
      ordered_issue_ids: ["ISS-READY", "ISS-BLOCKED", "ISS-ACTIVE"],
      ready_issue_ids: ["ISS-READY"],
      approved_issue_ids: ["ISS-READY", "ISS-BLOCKED", "ISS-ACTIVE"],
      issue_slices: [
        issueSlice({
          issue_id: "ISS-READY",
          title: "Launchable issue",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: true,
        }),
        issueSlice({
          issue_id: "ISS-BLOCKED",
          title: "Blocked issue",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: false,
          blockers: [
            {
              issue_id: "ISS-000",
              title: "Release seam",
              lifecycle: "Ready",
              satisfied: false,
            },
          ],
        }),
        issueSlice({
          issue_id: "ISS-ACTIVE",
          title: "Active issue",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: false,
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
  });

  expect(board.rows.find((row) => row.issueId === "ISS-READY")?.governedActions).toEqual([
    expect.objectContaining({
      label: "Launch",
      actionType: "issue-launch",
      actor: "mission-commander",
      issueId: "ISS-READY",
      expectedRevision: 21,
      targetIdentity: { kind: "issue-slice", id: "ISS-READY" },
    }),
    expect.objectContaining({
      label: "Assign model",
      actionType: "model-assignment-change",
      actor: "mission-commander",
      issueId: "ISS-READY",
      requiresReason: true,
      expectedRevision: 21,
      targetIdentity: { kind: "issue-slice", id: "ISS-READY" },
    }),
  ]);
  expect(board.rows.find((row) => row.issueId === "ISS-BLOCKED")?.governedActions).toEqual([]);
  expect(board.rows.find((row) => row.issueId === "ISS-ACTIVE")?.governedActions).toEqual([]);
});

test("offers agent-ready approval but never approves human-ready or blocked review rows", () => {
  const board = projectIssueAssignmentBoard({
    ...baseSnapshot,
    revision: 25,
    mission_board: {
      ...baseSnapshot.mission_board,
      ordered_issue_ids: ["ISS-AGENT", "ISS-HUMAN", "ISS-BLOCKED"],
      ready_issue_ids: [],
      approved_issue_ids: [],
      issue_slices: [
        issueSlice({
          issue_id: "ISS-AGENT",
          title: "Agent review work",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          lifecycle: "Needs review",
        }),
        issueSlice({
          issue_id: "ISS-HUMAN",
          title: "Human acceptance work",
          work_type: "HITL",
          tracker_status: "ready-for-human",
          lifecycle: "Needs review",
        }),
        issueSlice({
          issue_id: "ISS-BLOCKED",
          title: "Blocked agent review work",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          lifecycle: "Needs review",
          blockers: [
            {
              issue_id: "ISS-ROOT",
              title: "Root dependency",
              lifecycle: "Needs review",
              satisfied: false,
            },
          ],
        }),
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
  });

  expect(board.rows.find((row) => row.issueId === "ISS-AGENT")).toMatchObject({
    state: "needs-review",
    readinessState: "Needs review",
    governedActions: [
      expect.objectContaining({
        label: "Approve for launch",
        actionType: "issue-approve",
        requiresReason: false,
        issueId: "ISS-AGENT",
        expectedRevision: 25,
      }),
    ],
  });
  expect(board.rows.find((row) => row.issueId === "ISS-HUMAN")).toBeUndefined();
  expect(board.rows.find((row) => row.issueId === "ISS-BLOCKED")?.governedActions).toEqual([]);
});

test("keeps model reassignment available for approved agent-ready work with a stale agent", () => {
  const staleAssignment = issueSlice({
    issue_id: "ISS-STALE-AGENT",
    title: "Replace legacy worker",
    work_type: "AFK",
    tracker_status: "ready-for-agent",
    lifecycle: "Approved",
    launch_eligible: false,
    model_assignment: {
      agent_id: "qwen-coder-local-1",
      role: "local-agent",
      provider: "ollama",
      model: "qwen2.5-coder:14b",
      availability: "unavailable",
      availability_reason: "Agent is absent from the current registry.",
      operation_status: "idle",
      failure: "",
    },
  });
  const board = projectIssueAssignmentBoard({
    ...baseSnapshot,
    revision: 26,
    mission_board: {
      ...baseSnapshot.mission_board,
      ordered_issue_ids: ["ISS-STALE-AGENT"],
      ready_issue_ids: [],
      approved_issue_ids: ["ISS-STALE-AGENT"],
      issue_slices: [staleAssignment],
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
  });

  expect(board.rows[0]).toMatchObject({
    owner: "qwen-coder-local-1",
    assignmentState: "assigned",
    governedActions: [
      expect.objectContaining({
        label: "Change model assignment",
        actionType: "model-assignment-change",
        requiresReason: true,
      }),
    ],
  });
});

test("keeps a ready model-assigned issue launchable until a workstation session exists", () => {
  const assignedReadyIssue = issueSlice({
    issue_id: "ISS-READY",
    title: "Ready assigned work",
    work_type: "AFK",
    tracker_status: "ready-for-agent",
    launch_eligible: true,
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
  });
  const snapshot: WorkspaceSnapshot = {
    ...baseSnapshot,
    revision: 22,
    mission_board: {
      ...baseSnapshot.mission_board,
      issue_count: 1,
      ordered_issue_ids: ["ISS-READY"],
      ready_issue_ids: ["ISS-READY"],
      approved_issue_ids: ["ISS-READY"],
      issue_slices: [assignedReadyIssue],
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

  const [row] = projectIssueAssignmentBoard(snapshot).rows;

  expect(row).toMatchObject({
    issueId: "ISS-READY",
    owner: "qwen-coder-local",
    assignmentState: "assigned",
    state: "assigned",
    workstationSessionId: null,
    workstationAgent: null,
    workstationStatus: null,
  });
  expect(row.governedActions).toEqual([
    expect.objectContaining({
      label: "Launch",
      actionType: "issue-launch",
      issueId: "ISS-READY",
      expectedRevision: 22,
    }),
    expect.objectContaining({
      label: "Change model assignment",
      actionType: "model-assignment-change",
      issueId: "ISS-READY",
      expectedRevision: 22,
    }),
  ]);
  expect(projectWorkstationCards(snapshot).groups).toEqual([]);
});

test("keeps issue assignment linkage qualified to the active mission", () => {
  const duplicateIssueSnapshot: WorkspaceSnapshot = {
    ...baseSnapshot,
    active_mission: {
      id: "active-mission",
      title: "Active Mission",
      issue_count: 1,
    },
    conversation_scope: {
      kind: "issue-slice",
      target_id: "ISS-01",
      label: "Background issue",
      mission_id: "background-mission",
    },
    mission_board: {
      ...baseSnapshot.mission_board,
      prd_title: "Active Mission",
      issue_count: 1,
      ordered_issue_ids: ["ISS-01"],
      ready_issue_ids: ["ISS-01"],
      approved_issue_ids: ["ISS-01"],
      issue_slices: [
        issueSlice({
          issue_id: "ISS-01",
          title: "Active unassigned issue",
          work_type: "AFK",
          tracker_status: "ready-for-agent",
          launch_eligible: true,
        }),
      ],
    },
    missions: [
      {
        id: "background-mission",
        title: "Background Mission",
        issue_count: 1,
        is_active: false,
        sessions: [
          {
            session_id: "session-background-ISS-01-1",
            issue_id: "ISS-01",
            assigned_agent: "background-agent",
            status: "launched",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
        ],
        attention: [],
      },
      {
        id: "active-mission",
        title: "Active Mission",
        issue_count: 1,
        is_active: true,
        sessions: [],
        attention: [],
      },
    ],
  };

  const board = projectIssueAssignmentBoard(duplicateIssueSnapshot);

  expect(board.rows[0]).toMatchObject({
    issueId: "ISS-01",
    title: "Active unassigned issue",
    owner: "Unassigned",
    assignmentState: "unassigned",
    state: "unassigned-ready",
    workstationSessionId: null,
    workstationAgent: null,
    scopeDisabledReason: null,
    scope: {
      mission_id: "active-mission",
    },
  });
});

test("keeps duplicate issue-id workstation details isolated by mission identity", () => {
  const activeIssue = issueSlice({
    issue_id: "ISS-01",
    title: "Active mission implementation",
    progress: "Active mission runner streaming",
    sessions: [
      {
        session_id: "session-active-ISS-01-1",
        assigned_agent: "active-agent",
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
    evidence: {
      state: "missing",
      changed_files: ["active-mission.ts"],
      commands_run: ["npm test -- active-mission"],
      test_results: "No evidence package recorded.",
      risks: "None recorded.",
      artifact_links: [],
    },
  });
  const snapshot: WorkspaceSnapshot = {
    ...baseSnapshot,
    active_mission: {
      id: "active-mission",
      title: "Active Mission",
      issue_count: 1,
    },
    mission_board: {
      ...baseSnapshot.mission_board,
      prd_title: "Active Mission",
      issue_count: 1,
      ordered_issue_ids: ["ISS-01"],
      ready_issue_ids: [],
      approved_issue_ids: ["ISS-01"],
      issue_slices: [activeIssue],
    },
    missions: [
      {
        id: "active-mission",
        title: "Active Mission",
        issue_count: 1,
        is_active: true,
        sessions: [
          {
            session_id: "session-active-ISS-01-1",
            issue_id: "ISS-01",
            assigned_agent: "active-agent",
            status: "launched",
            role: "local-agent",
            provider: "ollama",
            model: "qwen3.6:27b",
          },
        ],
        attention: [],
      },
      {
        id: "background-mission",
        title: "Background Mission",
        issue_count: 1,
        is_active: false,
        sessions: [
          {
            session_id: "session-background-ISS-01-1",
            issue_id: "ADHOC-000001",
            assigned_agent: "background-agent",
            status: "evidence-ready",
            role: "subagent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
            task_title: "Repair background polling from the originating prompt",
            operation_status: "evidence-ready",
            failure: "",
            changed_files: ["src/background-polling.ts"],
            commands_run: ["npm test -- background-polling"],
            test_results: "Background polling tests passed.",
            risks: "None recorded.",
            artifact_links: [
              "app-local://missions/background-mission/sessions/session-background-ISS-01-1/artifacts/review_diff/review.diff",
            ],
          },
        ],
        attention: [],
      },
    ],
  };

  const cards = projectWorkstationCards(snapshot).groups.flatMap((group) => group.cards);
  const activeCard = cards.find((card) => card.sessionId === "session-active-ISS-01-1");
  const backgroundCard = cards.find((card) => card.sessionId === "session-background-ISS-01-1");

  expect(cards).toHaveLength(2);
  expect(activeCard).toMatchObject({
    missionId: "active-mission",
    currentTask: "Active mission implementation",
    filesTouched: 1,
    latestCommandOrTest: "npm test -- active-mission",
  });
  expect(backgroundCard).toMatchObject({
    missionId: "background-mission",
    issueId: "ADHOC-000001",
    currentTask: "Repair background polling from the originating prompt",
    status: "review-ready",
    filesTouched: 1,
    latestCommandOrTest: "npm test -- background-polling",
  });
  expect(backgroundCard?.detail.filesTouched).toEqual([
    { path: "src/background-polling.ts", status: "touched" },
  ]);
  expect(backgroundCard?.detail.diffs).toEqual([
    expect.objectContaining({
      path: "src/background-polling.ts",
      href: "app-local://missions/background-mission/sessions/session-background-ISS-01-1/artifacts/review_diff/review.diff",
      missionId: "background-mission",
      sessionId: "session-background-ISS-01-1",
    }),
  ]);
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
      missionId: "command-deck",
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
      missionId: "command-deck",
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
      missionId: "command-deck",
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
  const snapshot: WorkspaceSnapshot = {
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
          work_type: "AFK",
          tracker_status: "ready-for-agent",
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
  };
  const projection = projectWorkstationCards(snapshot);
  const board = projectIssueAssignmentBoard(snapshot);

  const cards = projection.groups.flatMap((group) => group.cards);
  expect(cards.find((card) => card.issueId === "ISS-04")).toBeUndefined();
  const launchRow = board.rows.find((row) => row.issueId === "ISS-04");
  expect(launchRow?.governedActions).toEqual(
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

test.each(["cancelled", "canceled"] as const)(
  "keeps %s sessions out of accepted completion projections",
  (cancellationStatus) => {
    const cancelledIssue = issueSlice({
      issue_id: "ISS-CANCELLED",
      title: "Cancelled implementation",
      work_type: "AFK",
      tracker_status: "ready-for-agent",
      lifecycle: "Complete",
      sessions: [
        {
          session_id: "session-ISS-CANCELLED-1",
          assigned_agent: "qwen-coder-local",
          role: "local-agent",
          provider: "ollama",
          model: "qwen3.6:27b",
          status: cancellationStatus,
          stale: false,
          disconnected: false,
          operation_status: cancellationStatus,
          failure: "",
        },
      ],
      evidence: {
        state: "accepted",
        changed_files: ["mission-control/src/workstation-projection.ts"],
        commands_run: [],
        test_results: "Prior evidence was accepted before this session was cancelled.",
        risks: "None recorded.",
        artifact_links: [],
      },
    });
    const cancelledSnapshot: WorkspaceSnapshot = {
      ...baseSnapshot,
      mission_board: {
        ...baseSnapshot.mission_board,
        issue_count: 1,
        ordered_issue_ids: ["ISS-CANCELLED"],
        ready_issue_ids: [],
        approved_issue_ids: ["ISS-CANCELLED"],
        issue_slices: [cancelledIssue],
      },
      missions: [
        {
          id: "command-deck",
          title: "Command Deck Mission",
          issue_count: 1,
          is_active: true,
          sessions: [
            {
              session_id: "session-ISS-CANCELLED-1",
              issue_id: "ISS-CANCELLED",
              assigned_agent: "qwen-coder-local",
              status: cancellationStatus,
              role: "local-agent",
              provider: "ollama",
              model: "qwen3.6:27b",
            },
          ],
          attention: [],
        },
      ],
    };

    const card = projectWorkstationCards(cancelledSnapshot).groups
      .flatMap((group) => group.cards)
      .find((item) => item.sessionId === "session-ISS-CANCELLED-1");
    const [row] = projectIssueAssignmentBoard(cancelledSnapshot).rows;

    expect(card).toMatchObject({
      status: "failed",
      nextAction: "Inspect failure evidence",
    });
    expect(row).toMatchObject({
      state: "failed",
      readinessState: "Failed",
    });
  },
);

test("projects expanded operational detail from canonical issue and session evidence", () => {
  const reviewDiffRef =
    "app-local://missions/command-deck/sessions/session-ISS-01-1/artifacts/review_diff/review.diff";
  const evidenceRef = "artifact://evidence/session-ISS-01-1";
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
                artifact_links: [
                  reviewDiffRef,
                  "runtime/evidence/session-ISS-01-1.json",
                  evidenceRef,
                ],
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
        href: reviewDiffRef,
        missionId: "command-deck",
        cardId: "session:command-deck:session-ISS-01-1",
        sessionId: "session-ISS-01-1",
      },
      {
        label: "Diff mission-control/src/styles.css",
        path: "mission-control/src/styles.css",
        href: reviewDiffRef,
        missionId: "command-deck",
        cardId: "session:command-deck:session-ISS-01-1",
        sessionId: "session-ISS-01-1",
      },
    ],
    evidenceLinks: [
      {
        label: "Review diff session-ISS-01-1",
        href: reviewDiffRef,
        sessionId: "session-ISS-01-1",
      },
      {
        label: "Evidence Package session-ISS-01-1",
        href: evidenceRef,
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

test("qualifies colliding mission-local session identities and actions by Mission", () => {
  const sharedSessionId = "session-ISS-01-1";
  const projection = projectWorkstationCards({
    ...baseSnapshot,
    missions: [
      ...(baseSnapshot.missions ?? []).map((mission) => ({
        ...mission,
        attention: [],
        sessions: mission.sessions
          .filter((session) => session.issue_id === "ISS-01")
          .map((session) => ({ ...session, session_id: sharedSessionId })),
      })),
      {
        id: "background-mission",
        title: "Background Mission",
        issue_count: 1,
        is_active: false,
        sessions: [
          {
            session_id: sharedSessionId,
            issue_id: "ISS-01",
            assigned_agent: "background-agent",
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

  const cards = projection.groups.flatMap((group) => group.cards);
  const collidingCards = cards.filter((card) => card.sessionId === sharedSessionId);
  expect(collidingCards.map((card) => card.id)).toEqual([
    "session:command-deck:session-ISS-01-1",
    "session:background-mission:session-ISS-01-1",
  ]);
  expect(
    collidingCards.map((card) =>
      card.detail.governedActions.find((action) => action.actionType === "session-cancel")
        ?.missionId,
    ),
  ).toEqual(["command-deck", "background-mission"]);
});

test("maps canonical session states to every workstation status", () => {
  const cases = [
    ["queued", "idle", "", "queued"],
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

test("sorts blocked and waiting approval cards above routine active work while preserving distinct states", () => {
  const statuses = [
    ["session-running", "ISS-RUN", "runner-agent", "launched", "streaming", "", "running"],
    ["session-blocked", "ISS-BLOCK", "blocked-agent", "needs-repair", "idle", "", "blocked"],
    ["session-idle", "ISS-IDLE", "idle-agent", "queued", "idle", "", "queued"],
    ["session-reviewing", "ISS-REVIEW", "review-agent", "needs-review", "idle", "", "reviewing"],
    ["session-ready", "ISS-READY", "evidence-agent", "evidence-ready", "completed", "", "review-ready"],
    ["session-failed", "ISS-FAIL", "failed-agent", "launched", "idle", "Provider failed.", "failed"],
  ] as const;
  const snapshot: WorkspaceSnapshot = {
    ...baseSnapshot,
    mission_board: {
      ...baseSnapshot.mission_board,
      issue_slices: statuses.map(([sessionId, issueId, agent, sessionStatus, operationStatus, failure]) => ({
        issue_id: issueId,
        title: `Work ${issueId}`,
        lifecycle: "Approved",
        progress: `${agent} progress`,
        launch_eligible: false,
        blockers: [],
        accepted_boundary: {
          what_to_build: `Work ${issueId}.`,
          acceptance_criteria: ["Card projects state."],
          evidence_requirements: ["Projection test."],
          source_path: `.scratch/issues/${issueId}.md`,
        },
        sessions: [
          {
            session_id: sessionId,
            assigned_agent: agent,
            role: "subagent",
            provider: "ollama",
            model: "qwen2.5-coder:14b",
            status: sessionStatus,
            stale: false,
            disconnected: false,
            operation_status: operationStatus,
            failure,
          },
        ],
        provenance: {
          role: "subagent",
          provider: "ollama",
          model: "qwen2.5-coder:14b",
        },
        model_assignment: {
          agent_id: agent,
          role: "subagent",
          provider: "ollama",
          model: "qwen2.5-coder:14b",
          availability: "available",
          availability_reason: "",
          operation_status: operationStatus,
          failure,
        },
        evidence: {
          state: operationStatus === "completed" ? "complete" : "missing",
          changed_files: [],
          commands_run: [`npm test -- ${issueId}`],
          test_results: "",
          risks: "",
          artifact_links: operationStatus === "completed" ? [`app-local://evidence/${sessionId}`] : [],
        },
        working_context_sources: [],
      })),
    },
    missions: [
      {
        id: "command-deck",
        title: "Command Deck Mission",
        issue_count: statuses.length,
        is_active: true,
        attention: [
          {
            attention_id: "queue-ISS-APPROVAL",
            mission_id: "command-deck",
            kind: "delegation-approval",
            label: "ISS-APPROVAL delegation approval required",
            queue_link: "workspace-queue#queue-ISS-APPROVAL",
            entity_id: "ISS-APPROVAL",
            queue_item_id: "",
          },
        ],
        sessions: statuses.map(([sessionId, issueId, agent, sessionStatus]) => ({
          session_id: sessionId,
          issue_id: issueId,
          assigned_agent: agent,
          status: sessionStatus,
          role: "subagent",
          provider: "ollama",
          model: "qwen2.5-coder:14b",
        })),
      },
    ],
  };

  const cards = projectWorkstationCards(snapshot).groups.flatMap((group) => group.cards);

  expect(cards.map((card) => card.status)).toEqual([
    "waiting-approval",
    "blocked",
    "failed",
    "reviewing",
    "queued",
    "running",
    "review-ready",
  ]);
  expect(cards.map((card) => [card.status, card.tone])).toEqual([
    ["waiting-approval", "attention"],
    ["blocked", "attention"],
    ["failed", "failed"],
    ["reviewing", "active"],
    ["queued", "active"],
    ["running", "active"],
    ["review-ready", "active"],
  ]);
});
