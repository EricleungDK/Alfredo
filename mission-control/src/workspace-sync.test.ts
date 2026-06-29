import type { WorkspaceSnapshot, WorkspaceUpdateBatch } from "./contracts";
import { applyWorkspaceUpdates } from "./workspace-sync";

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
    issue_count: 2,
  },
  conversation_scope: {
    kind: "working-directory",
    target_id: "/workspace/albert",
    label: "albert",
  },
  operations_view: "mission-board",
  mission_board: {
    prd_title: "Command Deck Mission",
    issue_count: 2,
    ordered_issue_ids: ["ISS-01", "ISS-02"],
    ready_issue_ids: ["ISS-02"],
    approved_issue_ids: [],
  },
};

test("applies a contiguous ordered batch without replacing unchanged entities", () => {
  const batch: WorkspaceUpdateBatch = {
    after_revision: 1,
    current_revision: 3,
    events: [
      {
        event_id: "workspace-2-review",
        correlation_id: "review",
        revision: 2,
        kind: "workspace-preferences-updated",
        active_mission_id: "command-deck",
        conversation_scope: {
          kind: "mission",
          target_id: "command-deck",
          label: "Command Deck Mission",
        },
        operations_view: "review-workspace",
      },
      {
        event_id: "workspace-3-activity",
        correlation_id: "activity",
        revision: 3,
        kind: "workspace-preferences-updated",
        active_mission_id: "command-deck",
        conversation_scope: {
          kind: "mission",
          target_id: "command-deck",
          label: "Command Deck Mission",
        },
        operations_view: "activity",
      },
    ],
  };

  const result = applyWorkspaceUpdates(snapshot, batch);

  expect(result.kind).toBe("applied");
  if (result.kind !== "applied") throw new Error("expected applied batch");
  expect(result.snapshot.revision).toBe(3);
  expect(result.snapshot.operations_view).toBe("activity");
  expect(result.snapshot.conversation_scope.kind).toBe("mission");
  expect(result.snapshot.workspace_session).toBe(snapshot.workspace_session);
  expect(result.snapshot.active_mission).toBe(snapshot.active_mission);
  expect(result.snapshot.mission_board).toBe(snapshot.mission_board);
});

test("requests canonical resynchronization for an invalid revision range", () => {
  const result = applyWorkspaceUpdates(snapshot, {
    after_revision: 1,
    current_revision: 0,
    events: [],
  });

  expect(result).toEqual({
    kind: "resync-required",
    reason: "Update batch is malformed or has a revision gap.",
  });
  expect(snapshot.revision).toBe(1);
  expect(snapshot.operations_view).toBe("mission-board");
});
