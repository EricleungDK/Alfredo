import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { App } from "./App";
import type { WorkspaceClient } from "./workspace-client";

type SelectionRequest = Parameters<NonNullable<WorkspaceClient["selectCodingWorkspace"]>>[0];
type MissionRequest = Parameters<NonNullable<WorkspaceClient["chooseMission"]>>[0];

const selectionRequiredContext = {
  schema_version: 1 as const,
  selected_agent: "qwen3-14b",
  selected_model: "qwen3:14b",
  starting_location: "/home/mission-commander/projects",
  coding_workspace: null,
  active_mission: null,
  phase: "selection-required" as const,
  runtime_root: "/home/mission-commander/.alfredo/runtime",
  recent_workspaces: [],
};

function snapshotMustRemainBlocked() {
  return vi.fn(async () => {
    throw new Error("workspace snapshot must remain blocked before Mission selection");
  });
}

test("prefills a safe child path below the Starting Location", async () => {
  render(
    <App
      client={{
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: selectionRequiredContext,
        }),
        loadSnapshot: snapshotMustRemainBlocked(),
      }}
    />,
  );

  const workspacePath = await screen.findByRole("textbox", {
    name: "Coding Workspace path",
  });
  await waitFor(() =>
    expect(workspacePath).toHaveValue("/home/mission-commander/projects/workspace"),
  );
  expect(screen.getByRole("button", { name: "Create new repository" })).toBeEnabled();
});

test("keeps Coding Workspace selection pending until the Orchestrator acknowledges it", async () => {
  let acknowledgeSelection!: (
    value: Awaited<ReturnType<NonNullable<WorkspaceClient["selectCodingWorkspace"]>>>,
  ) => void;
  const pendingSelection = new Promise<
    Awaited<ReturnType<NonNullable<WorkspaceClient["selectCodingWorkspace"]>>>
  >((resolve) => {
    acknowledgeSelection = resolve;
  });
  const loadSnapshot = snapshotMustRemainBlocked();
  const selectCodingWorkspace = vi.fn(async (_request: SelectionRequest) => pendingSelection);

  render(
    <App
      client={{
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: selectionRequiredContext,
        }),
        loadSnapshot,
        selectCodingWorkspace,
      }}
    />,
  );

  const agentConsole = await screen.findByRole("main", { name: "Agent Console" });
  expect(within(agentConsole).getByText("Starting Location")).toBeVisible();
  expect(within(agentConsole).getByText(selectionRequiredContext.starting_location)).toBeVisible();
  expect(loadSnapshot).not.toHaveBeenCalled();

  fireEvent.change(screen.getByRole("textbox", { name: "Coding Workspace path" }), {
    target: { value: "/home/mission-commander/projects/acknowledged" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Choose existing repository" }));

  expect(await screen.findByText("Waiting for Orchestrator acknowledgement.")).toBeVisible();
  expect(screen.queryByText("Mission selection required")).not.toBeInTheDocument();
  const request = selectCodingWorkspace.mock.calls[0][0];

  acknowledgeSelection({
    kind: "acknowledged",
    acknowledgement: {
      schema_version: 1,
      correlation_id: request.correlation_id,
      outcome: "acknowledged",
      starting_location: selectionRequiredContext.starting_location,
      coding_workspace: "/home/mission-commander/projects/acknowledged",
      selection_mode: "existing",
      active_mission: null,
      replayed: false,
      message: "Coding Workspace acknowledged by the Orchestrator; no Mission has been selected.",
    },
  });

  expect(await screen.findByText("Mission selection required")).toBeVisible();
  expect(loadSnapshot).not.toHaveBeenCalled();
});

test("requires explicit Resume Mission or Start New Mission choices before loading work", async () => {
  const missionChoiceContext = {
    ...selectionRequiredContext,
    coding_workspace: "/home/mission-commander/projects/acknowledged",
    phase: "mission-choice-required" as const,
    revision: 1,
    known_missions: [{ id: "existing", title: "Existing Mission" }],
  };
  const loadSnapshot = snapshotMustRemainBlocked();
  const requests: MissionRequest[] = [];
  const chooseMission = vi.fn(async (request: MissionRequest) => {
    requests.push(request);
    return {
      kind: "mission-choice-failure" as const,
      code: "mission-not-found",
      message: "The requested Mission is not known for this Coding Workspace.",
      recoverable: true,
    };
  });

  render(
    <App
      client={{
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: missionChoiceContext,
        }),
        loadSnapshot,
        chooseMission,
      }}
    />,
  );

  expect(await screen.findByText("Mission selection required")).toBeVisible();
  expect(screen.getByRole("button", { name: /Resume Mission/ })).toBeVisible();
  expect(screen.getByRole("button", { name: "Start New Mission" })).toBeVisible();
  expect(loadSnapshot).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: /Resume Mission/ }));
  expect(await screen.findByRole("alert")).toHaveTextContent("mission-not-found");
  expect(requests[0]).toMatchObject({
    expected_revision: 1,
    choice: "resume",
    mission_id: "existing",
  });

  fireEvent.change(screen.getByRole("textbox", { name: "New Mission title" }), {
    target: { value: "Modernization" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Start New Mission" }));
  expect(requests[1]).toMatchObject({
    expected_revision: 1,
    choice: "new",
    mission_id: "modernization",
    mission_title: "Modernization",
  });
  expect(await screen.findByRole("alert")).toHaveTextContent("mission-not-found");
  expect(loadSnapshot).not.toHaveBeenCalled();
});

test("rejects an acknowledgement for a different Mission without leaving the gate", async () => {
  const missionChoiceContext = {
    ...selectionRequiredContext,
    coding_workspace: "/home/mission-commander/projects/acknowledged",
    phase: "mission-choice-required" as const,
    revision: 1,
    known_missions: [{ id: "existing", title: "Existing Mission" }],
  };
  const loadSnapshot = vi.fn(async () => ({
    kind: "contract-failure" as const,
    message: "snapshot must remain blocked for a mismatched Mission acknowledgement",
    recoverable: false,
  }));
  const chooseMission = vi.fn(async (request: MissionRequest) => ({
    kind: "acknowledged" as const,
    acknowledgement: {
      schema_version: 1 as const,
      correlation_id: request.correlation_id,
      outcome: "acknowledged" as const,
      coding_workspace: missionChoiceContext.coding_workspace,
      choice: request.choice,
      active_mission: "different-mission",
      revision: 2,
      replayed: false,
      missions: [{ id: "different-mission", title: "Different Mission" }],
      message: "A different Mission was selected.",
    },
  }));

  render(
    <App
      client={{
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: missionChoiceContext,
        }),
        loadSnapshot,
        chooseMission,
      }}
    />,
  );

  fireEvent.click(await screen.findByRole("button", { name: /Resume Mission/ }));

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "invalid-mission-acknowledgement",
  );
  expect(screen.getByText("Mission selection required")).toBeVisible();
  expect(loadSnapshot).not.toHaveBeenCalled();
});

test.each([
  { choice: "resume" as const, title: "", expectedMissionId: "existing" },
  { choice: "new" as const, title: "Modernization", expectedMissionId: "modernization" },
])("loads canonical work only after an acknowledged $choice choice", async ({
  choice,
  title,
  expectedMissionId,
}) => {
  const missionChoiceContext = {
    ...selectionRequiredContext,
    coding_workspace: "/home/mission-commander/projects/acknowledged",
    phase: "mission-choice-required" as const,
    revision: 1,
    known_missions: [{ id: "existing", title: "Existing Mission" }],
  };
  const loadSnapshot = vi.fn(() => new Promise<never>(() => undefined));
  const chooseMission = vi.fn(async (request: MissionRequest) => ({
    kind: "acknowledged" as const,
    acknowledgement: {
      schema_version: 1 as const,
      correlation_id: request.correlation_id,
      outcome: "acknowledged" as const,
      coding_workspace: missionChoiceContext.coding_workspace,
      choice: request.choice,
      active_mission: request.mission_id,
      revision: 2,
      replayed: false,
      missions: [
        { id: "existing", title: "Existing Mission" },
        ...(request.choice === "new"
          ? [{ id: request.mission_id, title: request.mission_title ?? "" }]
          : []),
      ],
      message: request.choice === "resume" ? "Existing Mission resumed." : "New Mission started.",
    },
  }));

  render(
    <App
      client={{
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: missionChoiceContext,
        }),
        loadSnapshot,
        chooseMission,
      }}
    />,
  );

  if (choice === "new") {
    fireEvent.change(await screen.findByRole("textbox", { name: "New Mission title" }), {
      target: { value: title },
    });
    fireEvent.click(screen.getByRole("button", { name: "Start New Mission" }));
  } else {
    fireEvent.click(await screen.findByRole("button", { name: /Resume Mission/ }));
  }

  expect(chooseMission).toHaveBeenCalledWith(
    expect.objectContaining({ choice, mission_id: expectedMissionId }),
  );
  await waitFor(() => expect(loadSnapshot).toHaveBeenCalledOnce());
  expect(screen.queryByText("Mission selection required")).not.toBeInTheDocument();
});

test("keeps selection required after a structured failure and permits an acknowledged retry", async () => {
  const codingWorkspace = "/home/mission-commander/projects/acknowledged";
  const loadSnapshot = snapshotMustRemainBlocked();
  const selectCodingWorkspace = vi
    .fn()
    .mockResolvedValueOnce({
      kind: "selection-failure" as const,
      code: "workspace-unsafe",
      message: "The selected repository overlaps an Alfredo backend root.",
      recoverable: true,
    })
    .mockImplementationOnce(async (request: SelectionRequest) => ({
      kind: "acknowledged" as const,
      acknowledgement: {
        schema_version: 1 as const,
        correlation_id: request.correlation_id,
        outcome: "acknowledged" as const,
        starting_location: selectionRequiredContext.starting_location,
        coding_workspace: codingWorkspace,
        selection_mode: "existing" as const,
        active_mission: null,
        replayed: false,
        message: "Coding Workspace acknowledged by the Orchestrator; no Mission has been selected.",
      },
    }));

  render(
    <App
      client={{
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: selectionRequiredContext,
        }),
        loadSnapshot,
        selectCodingWorkspace,
      }}
    />,
  );

  fireEvent.change(await screen.findByRole("textbox", { name: "Coding Workspace path" }), {
    target: { value: codingWorkspace },
  });
  fireEvent.click(screen.getByRole("button", { name: "Choose existing repository" }));

  const failure = await screen.findByRole("alert");
  expect(failure).toHaveTextContent("workspace-unsafe");
  expect(failure).toHaveTextContent("Choose a repository below Starting Location");
  expect(screen.getByText("Choose or create a repository")).toBeVisible();
  expect(screen.queryByText("Mission selection required")).not.toBeInTheDocument();
  expect(loadSnapshot).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "Choose existing repository" }));

  expect(await screen.findByText("Mission selection required")).toBeVisible();
  expect(screen.getByText(codingWorkspace)).toBeVisible();
  expect(loadSnapshot).not.toHaveBeenCalled();
});

test("rejects an acknowledgement for a different correlation without leaving the gate", async () => {
  const codingWorkspace = "/home/mission-commander/projects/acknowledged";
  const loadSnapshot = snapshotMustRemainBlocked();
  const selectCodingWorkspace = vi.fn(async (_request: SelectionRequest) => ({
    kind: "acknowledged" as const,
    acknowledgement: {
      schema_version: 1 as const,
      correlation_id: "unrelated-correlation",
      outcome: "acknowledged" as const,
      starting_location: selectionRequiredContext.starting_location,
      coding_workspace: codingWorkspace,
      selection_mode: "existing" as const,
      active_mission: null,
      replayed: false,
      message: "Unrelated acknowledgement.",
    },
  }));

  render(
    <App
      client={{
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: selectionRequiredContext,
        }),
        loadSnapshot,
        selectCodingWorkspace,
      }}
    />,
  );

  fireEvent.change(await screen.findByRole("textbox", { name: "Coding Workspace path" }), {
    target: { value: codingWorkspace },
  });
  fireEvent.click(screen.getByRole("button", { name: "Choose existing repository" }));

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "invalid-workspace-acknowledgement",
  );
  expect(screen.getByText("Choose or create a repository")).toBeVisible();
  expect(screen.queryByText("Mission selection required")).not.toBeInTheDocument();
  expect(loadSnapshot).not.toHaveBeenCalled();
});

test("accepts a canonical workspace path after the native boundary validates it", async () => {
  const enteredWorkspace = "/home/mission-commander/projects/acknowledged/.";
  const canonicalWorkspace = "/home/mission-commander/projects/acknowledged";
  const selectCodingWorkspace = vi.fn(async (request: SelectionRequest) => ({
    kind: "acknowledged" as const,
    acknowledgement: {
      schema_version: 1 as const,
      correlation_id: request.correlation_id,
      outcome: "acknowledged" as const,
      starting_location: selectionRequiredContext.starting_location,
      coding_workspace: canonicalWorkspace,
      selection_mode: request.selection_mode,
      active_mission: null,
      replayed: false,
      message: "Canonical Coding Workspace acknowledged.",
    },
  }));

  render(
    <App
      client={{
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: selectionRequiredContext,
        }),
        loadSnapshot: snapshotMustRemainBlocked(),
        selectCodingWorkspace,
      }}
    />,
  );

  fireEvent.change(await screen.findByRole("textbox", { name: "Coding Workspace path" }), {
    target: { value: enteredWorkspace },
  });
  fireEvent.click(screen.getByRole("button", { name: "Choose existing repository" }));

  expect(await screen.findByText("Mission selection required")).toBeVisible();
  expect(screen.getByText(canonicalWorkspace)).toBeVisible();
});

test.each([
  "backend-startup-failure",
  "workspace-create-cleanup-failed",
  "runtime-state-write-failed",
])("reuses the correlation when retrying recoverable failure %s", async (failureCode) => {
  const codingWorkspace = "/home/mission-commander/projects/retry";
  const requests: SelectionRequest[] = [];
  const selectCodingWorkspace = vi.fn(async (request: SelectionRequest) => {
    requests.push(request);
    if (requests.length === 1) {
      return {
        kind: "selection-failure" as const,
        code: failureCode,
        message: "The acknowledgement response was unavailable.",
        recoverable: true,
      };
    }
    return {
      kind: "acknowledged" as const,
      acknowledgement: {
        schema_version: 1 as const,
        correlation_id: request.correlation_id,
        outcome: "acknowledged" as const,
        starting_location: selectionRequiredContext.starting_location,
        coding_workspace: codingWorkspace,
        selection_mode: "existing" as const,
        active_mission: null,
        replayed: true,
        message: "Coding Workspace acknowledgement replayed.",
      },
    };
  });

  render(
    <App
      client={{
        loadLaunchContext: async () => ({
          kind: "launch-context",
          context: selectionRequiredContext,
        }),
        loadSnapshot: snapshotMustRemainBlocked(),
        selectCodingWorkspace,
      }}
    />,
  );

  fireEvent.change(await screen.findByRole("textbox", { name: "Coding Workspace path" }), {
    target: { value: codingWorkspace },
  });
  fireEvent.click(screen.getByRole("button", { name: "Choose existing repository" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(failureCode);

  fireEvent.click(screen.getByRole("button", { name: "Choose existing repository" }));

  expect(await screen.findByText("Mission selection required")).toBeVisible();
  expect(requests).toHaveLength(2);
  expect(requests[1]).toEqual(requests[0]);
});
