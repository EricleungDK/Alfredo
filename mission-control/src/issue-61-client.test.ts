import { TauriWorkspaceClient } from "./workspace-client";
import type {
  AgentConsoleResponseProjection,
  AgentConsoleResponseRequest,
  ConversationScope,
} from "./contracts";

test("preserves the Wayfinder projection across the Agent Console client boundary", async () => {
  const scope: ConversationScope = {
    kind: "working-directory",
    target_id: "/workspace/albert",
    label: "albert",
  };
  const request: AgentConsoleResponseRequest = {
    expected_revision: 4,
    message_id: "console-000001",
    scope_kind: scope.kind,
    scope_target: scope.target_id,
    scope_label: scope.label,
  };
  const projection: AgentConsoleResponseProjection = {
    message: {
      message_id: "console-000002",
      sequence: 2,
      role: "assistant",
      content: "Wayfinder Chart mode is active.",
      scope,
      outcome: "acknowledged",
      source: "orchestrator",
    },
    route: {
      intent: "discussion",
      task_request: "",
      acceptance_criteria: [],
    },
    wayfinder: {
      mode: "chart",
      gate: { status: "pending", opened_by: "", receipt_id: "" },
      flow: {
        flow_id: "wayfinder-console-000001",
        mode: "chart",
        originating_message_id: "console-000001",
        scope,
        reference: "",
      },
      continuing: false,
      turn_complete: true,
    },
  };
  const invoke = async <T>(
    command: string,
    args?: Record<string, unknown>,
  ): Promise<T> => {
    expect(command).toBe("agent_console_response");
    expect(args).toEqual({ request });
    return projection as T;
  };

  const result = await new TauriWorkspaceClient(invoke, () => true).generateConsoleResponse(request);

  expect(result).toEqual({
    kind: "message",
    message: projection.message,
    route: projection.route,
    wayfinder: projection.wayfinder,
  });
});
