import { useCallback, useEffect, useState } from "react";
import type {
  PathAccessLevel,
  ShellTerminalCommandRecord,
  ShellTerminalCommandResult,
  ShellTerminalProjection,
} from "./contracts";
import type { WorkspaceClient } from "./workspace-client";

export interface ShellTerminalTranscriptEntry extends ShellTerminalCommandResult {
  readonly command: string;
}

export interface ContextualPathGrantRequest {
  readonly requestId: string;
  readonly path: string;
  readonly accessLevel: PathAccessLevel;
  readonly durationSeconds: number;
  readonly reason: string;
  readonly affectedAction: string;
  readonly status: "pending" | "granted" | "denied";
}

export interface ShellTerminalController {
  readonly commandDraft: string;
  readonly workingDirectory: string;
  readonly requestedPathsDraft: string;
  readonly accessLevel: PathAccessLevel;
  readonly projection: ShellTerminalProjection | null;
  readonly loadStatus: "idle" | "pending" | "rejected";
  readonly errorMessage: string;
  readonly transcript: readonly ShellTerminalTranscriptEntry[];
  readonly actionStatus: {
    readonly state: "pending" | "acknowledged" | "rejected";
    readonly message: string;
  } | null;
  readonly denialReasons: Readonly<Record<string, string>>;
  readonly contextualGrantRequest: ContextualPathGrantRequest | null;
  readonly setCommandDraft: (value: string) => void;
  readonly setWorkingDirectory: (value: string) => void;
  readonly setRequestedPathsDraft: (value: string) => void;
  readonly setAccessLevel: (value: PathAccessLevel) => void;
  readonly load: () => Promise<boolean>;
  readonly submit: () => Promise<void>;
  readonly setDenialReason: (commandId: string, reason: string) => void;
  readonly decide: (
    command: ShellTerminalCommandRecord,
    decision: "approve" | "deny",
  ) => Promise<void>;
  readonly createGrantForRequest: (requestId: string) => Promise<void>;
  readonly denyGrantRequest: (requestId: string) => void;
}

export interface ShellTerminalWorkstationActionTurn {
  readonly id: string;
  readonly content: string;
  readonly source: string;
  readonly outcome: string;
}

export interface ShellTerminalOptions {
  readonly onWorkstationActionTurn?: (turn: ShellTerminalWorkstationActionTurn) => void;
}

export function useShellTerminal(
  client: WorkspaceClient,
  workspacePath: string,
  options: ShellTerminalOptions = {},
): ShellTerminalController {
  const [commandDraft, setCommandDraft] = useState("");
  const [workingDirectory, setWorkingDirectory] = useState("");
  const [requestedPathsDraft, setRequestedPathsDraft] = useState("");
  const [accessLevel, setAccessLevel] = useState<PathAccessLevel>("read");
  const [projection, setProjection] = useState<ShellTerminalProjection | null>(null);
  const [loadStatus, setLoadStatus] = useState<"idle" | "pending" | "rejected">("idle");
  const [errorMessage, setErrorMessage] = useState("");
  const [transcript, setTranscript] = useState<readonly ShellTerminalTranscriptEntry[]>([]);
  const [actionStatus, setActionStatus] = useState<{
    state: "pending" | "acknowledged" | "rejected";
    message: string;
  } | null>(null);
  const [denialReasons, setDenialReasons] = useState<Record<string, string>>({});
  const [contextualGrantRequest, setContextualGrantRequest] =
    useState<ContextualPathGrantRequest | null>(null);

  useEffect(() => {
    if (workspacePath) setWorkingDirectory((current) => current || workspacePath);
  }, [workspacePath]);

  const emitActionStart = useCallback(
    (correlationId: string, label: string) => {
      options.onWorkstationActionTurn?.({
        id: `${correlationId}:intent`,
        content: `Workstation action: Mission Commander requested ${label}.`,
        source: "mission-commander",
        outcome: "pending",
      });
      options.onWorkstationActionTurn?.({
        id: `${correlationId}:reaction:pending`,
        content: "Orchestrator validating workstation action.",
        source: "orchestrator",
        outcome: "pending",
      });
    },
    [options],
  );

  const emitActionFinish = useCallback(
    (correlationId: string, outcome: "acknowledged" | "rejected", message: string) => {
      options.onWorkstationActionTurn?.({
        id: `${correlationId}:reaction:${outcome}`,
        content:
          outcome === "acknowledged"
            ? `Orchestrator accepted workstation action: ${message}`
            : `Orchestrator rejected workstation action: ${message}`,
        source: "orchestrator",
        outcome,
      });
    },
    [options],
  );

  const load = useCallback(async () => {
    if (!client.loadShellTerminal) {
      setLoadStatus("rejected");
      setErrorMessage("Shell Terminal transport is unavailable.");
      return false;
    }
    setLoadStatus("pending");
    const result = await client.loadShellTerminal();
    if (result.kind !== "shell-terminal") {
      setLoadStatus("rejected");
      setErrorMessage(result.message);
      return false;
    }
    setProjection(result.projection);
    setLoadStatus("idle");
    setErrorMessage("");
    return true;
  }, [client]);

  const submit = useCallback(async () => {
    if (!commandDraft.trim() || !workingDirectory.trim()) return;
    if (!client.submitShellTerminalCommand) {
      setActionStatus({ state: "rejected", message: "Shell Terminal command transport is unavailable." });
      return;
    }
    setActionStatus({ state: "pending", message: "Command submission pending." });
    const command = commandDraft.trim();
    const requestedPaths = requestedPathsDraft
      .split(/\r?\n/)
      .map((path) => path.trim())
      .filter(Boolean);
    const result = await client.submitShellTerminalCommand({
      correlation_id: `terminal-command-${Date.now()}`,
      command,
      working_directory: workingDirectory.trim(),
      requested_paths: requestedPaths,
      requester: "mission-commander",
      access_level: accessLevel,
    });
    if (result.kind !== "command-result") {
      setActionStatus({ state: "rejected", message: result.message });
      const grantRequest = buildContextualGrantRequest({
        command,
        workingDirectory: workingDirectory.trim(),
        requestedPaths,
        accessLevel,
        reason: result.message,
      });
      if (grantRequest) {
        setContextualGrantRequest(grantRequest);
      }
      return;
    }
    setTranscript((entries) => [...entries, { ...result.result, command }]);
    setCommandDraft("");
    setActionStatus({ state: "acknowledged", message: `Command ${result.result.status}.` });
    await load();
  }, [accessLevel, client, commandDraft, load, requestedPathsDraft, workingDirectory]);

  const setDenialReason = useCallback((commandId: string, reason: string) => {
    setDenialReasons((current) => ({ ...current, [commandId]: reason }));
  }, []);

  const decide = useCallback(async (
    command: ShellTerminalCommandRecord,
    decision: "approve" | "deny",
  ) => {
    const reason = denialReasons[command.command_id]?.trim() ?? "";
    if (decision === "deny" && !reason) {
      setActionStatus({ state: "rejected", message: "Denial requires a reason." });
      return;
    }
    if (!client.decideShellTerminalCommand) {
      setActionStatus({ state: "rejected", message: "Shell Terminal decision transport is unavailable." });
      return;
    }
    const correlationId = `terminal-decision-${decision}-${command.command_id}`;
    emitActionStart(correlationId, `${decision === "approve" ? "Approve" : "Deny"} terminal command ${command.command_id}`);
    setActionStatus({ state: "pending", message: `Decision pending for ${command.command_id}.` });
    const result = await client.decideShellTerminalCommand({
      command_id: command.command_id,
      decision,
      actor: "mission-commander",
      reason,
    });
    if (result.kind !== "command-result") {
      setActionStatus({ state: "rejected", message: result.message });
      emitActionFinish(correlationId, "rejected", result.message);
      return;
    }
    if (result.result.stdout || result.result.stderr) {
      setTranscript((entries) => [...entries, { ...result.result, command: command.command }]);
    }
    setActionStatus({ state: "acknowledged", message: `Command ${result.result.status}.` });
    emitActionFinish(correlationId, "acknowledged", `Command ${result.result.status}.`);
    await load();
  }, [client, denialReasons, emitActionFinish, emitActionStart, load]);

  const createGrantForRequest = useCallback(async (requestId: string) => {
    const request = contextualGrantRequest;
    if (!request || request.requestId !== requestId || request.status !== "pending") return;
    if (!client.createAdditionalPathGrant) {
      setActionStatus({ state: "rejected", message: "Additional Path Grant transport is unavailable." });
      return;
    }
    const correlationId = `path-grant-${Date.now()}`;
    emitActionStart(
      correlationId,
      `Create Additional Path Grant for ${request.path}`,
    );
    setActionStatus({ state: "pending", message: "Additional Path Grant creation pending." });
    const result = await client.createAdditionalPathGrant({
      correlation_id: correlationId,
      expected_revision: projection?.revision ?? 0,
      path: request.path,
      access_level: request.accessLevel,
      duration_seconds: request.durationSeconds,
      requester: "mission-commander",
    });
    if (result.kind !== "path-grant") {
      setActionStatus({ state: "rejected", message: result.message });
      emitActionFinish(correlationId, "rejected", result.message);
      return;
    }
    setContextualGrantRequest({ ...request, status: "granted" });
    setActionStatus({ state: "acknowledged", message: `Created ${result.grant.grant_id}.` });
    emitActionFinish(correlationId, "acknowledged", `Created ${result.grant.grant_id}.`);
    await load();
  }, [client, contextualGrantRequest, emitActionFinish, emitActionStart, load, projection?.revision]);

  const denyGrantRequest = useCallback((requestId: string) => {
    setContextualGrantRequest((request) => {
      if (!request || request.requestId !== requestId || request.status !== "pending") return request;
      const correlationId = `path-grant-denied-${requestId}`;
      options.onWorkstationActionTurn?.({
        id: `${correlationId}:intent`,
        content: `Workstation action: Mission Commander denied Additional Path Grant for ${request.path}.`,
        source: "mission-commander",
        outcome: "rejected",
      });
      options.onWorkstationActionTurn?.({
        id: `${correlationId}:reaction:denied`,
        content: `Orchestrator left command blocked: Additional Path Grant denied for ${request.affectedAction}.`,
        source: "orchestrator",
        outcome: "rejected",
      });
      setActionStatus({ state: "acknowledged", message: "Additional Path Grant request denied." });
      return { ...request, status: "denied" };
    });
  }, [options]);

  return {
    commandDraft,
    workingDirectory,
    requestedPathsDraft,
    accessLevel,
    projection,
    loadStatus,
    errorMessage,
    transcript,
    actionStatus,
    denialReasons,
    contextualGrantRequest,
    setCommandDraft,
    setWorkingDirectory,
    setRequestedPathsDraft,
    setAccessLevel,
    load,
    submit,
    setDenialReason,
    decide,
    createGrantForRequest,
    denyGrantRequest,
  };
}

function buildContextualGrantRequest({
  command,
  workingDirectory,
  requestedPaths,
  accessLevel,
  reason,
}: {
  readonly command: string;
  readonly workingDirectory: string;
  readonly requestedPaths: readonly string[];
  readonly accessLevel: PathAccessLevel;
  readonly reason: string;
}): ContextualPathGrantRequest | null {
  if (!reason.toLowerCase().includes("additional path grant")) return null;
  const path =
    reason.toLowerCase().includes("working directory")
      ? workingDirectory
      : requestedPaths[0] ?? workingDirectory;
  if (!path) return null;
  return {
    requestId: `contextual-grant-${Date.now()}`,
    path,
    accessLevel,
    durationSeconds: 900,
    reason,
    affectedAction: command,
    status: "pending",
  };
}
