import { useCallback, useEffect, useRef, useState } from "react";
import type {
  AdditionalPathGrantRequestRecord,
  PathAccessLevel,
  ShellTerminalCommandRequest,
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
  readonly submitCommand: (command: string) => Promise<void>;
  readonly setDenialReason: (commandId: string, reason: string) => void;
  readonly decide: (
    command: ShellTerminalCommandRecord,
    decision: "approve" | "deny",
  ) => Promise<void>;
  readonly createGrantForRequest: (requestId: string) => Promise<void>;
  readonly denyGrantRequest: (requestId: string) => Promise<void>;
}

export interface ShellTerminalWorkstationActionTurn {
  readonly id: string;
  readonly content: string;
  readonly source: string;
  readonly outcome: string;
}

export interface ShellTerminalOptions {
  readonly onWorkstationActionTurn?: (turn: ShellTerminalWorkstationActionTurn) => void;
  readonly onCommandTurnAvailable?: (commandId: string) => void;
}

interface RetryableShellSubmission {
  readonly correlationId: string;
  readonly requestKey: string;
}

let shellSubmissionSequence = 0;

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
  const pathGrantDenialAttemptRef = useRef(0);
  const retryableSubmissionRef = useRef<RetryableShellSubmission | null>(null);
  const onWorkstationActionTurn = options.onWorkstationActionTurn;
  const onCommandTurnAvailable = options.onCommandTurnAvailable;

  useEffect(() => {
    if (workspacePath) setWorkingDirectory((current) => current || workspacePath);
  }, [workspacePath]);

  const emitActionStart = useCallback(
    (correlationId: string, label: string): void => {
      onWorkstationActionTurn?.({
        id: `${correlationId}:intent`,
        content: `Workstation action: Mission Commander requested ${label}.`,
        source: "mission-commander",
        outcome: "pending",
      });
      onWorkstationActionTurn?.({
        id: `${correlationId}:reaction:pending`,
        content: "Orchestrator validating workstation action.",
        source: "orchestrator",
        outcome: "pending",
      });
    },
    [onWorkstationActionTurn],
  );

  const emitActionFinish = useCallback(
    (correlationId: string, outcome: "acknowledged" | "rejected", message: string): void => {
      onWorkstationActionTurn?.({
        id: `${correlationId}:reaction:${outcome}`,
        content:
          outcome === "acknowledged"
            ? `Orchestrator accepted workstation action: ${message}`
            : `Orchestrator rejected workstation action: ${message}`,
        source: "orchestrator",
        outcome,
      });
    },
    [onWorkstationActionTurn],
  );

  const load = useCallback(async (): Promise<boolean> => {
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
    for (const command of result.projection.commands) {
      onCommandTurnAvailable?.(command.command_id);
    }
    setContextualGrantRequest((current) => {
      const requests = result.projection.path_grant_requests ?? [];
      const newestPending = [...requests]
        .reverse()
        .find((request) => request.status === "pending");
      if (newestPending) return contextualGrantRequestFromRecord(newestPending);
      const currentRecord = current
        ? requests.find((request) => request.request_id === current.requestId)
        : undefined;
      return currentRecord ? contextualGrantRequestFromRecord(currentRecord) : null;
    });
    setProjection(result.projection);
    setLoadStatus("idle");
    setErrorMessage("");
    return true;
  }, [client, onCommandTurnAvailable]);

  const submitCommand = useCallback(async (requestedCommand: string) => {
    if (!requestedCommand.trim() || !workingDirectory.trim()) return;
    const command = requestedCommand.trim();
    const requestedPaths = requestedPathsDraft
      .split(/\r?\n/)
      .map((path) => path.trim())
      .filter(Boolean);
    const requestKey = JSON.stringify({
      command,
      working_directory: workingDirectory.trim(),
      requested_paths: requestedPaths,
      requester: "mission-commander",
      access_level: accessLevel,
    });
    const retryable = retryableSubmissionRef.current;
    const correlationId =
      retryable?.requestKey === requestKey
        ? retryable.correlationId
        : `terminal-command-${Date.now()}-${++shellSubmissionSequence}`;
    if (!client.submitShellTerminalCommand) {
      const message = "Shell Terminal command transport is unavailable.";
      setActionStatus({ state: "rejected", message });
      onWorkstationActionTurn?.({
        id: `${correlationId}:reaction:rejected`,
        content: `Shell Terminal command rejected: ${message}`,
        source: "orchestrator",
        outcome: "rejected",
      });
      return;
    }
    const request: ShellTerminalCommandRequest = {
      correlation_id: correlationId,
      command,
      working_directory: workingDirectory.trim(),
      requested_paths: requestedPaths,
      requester: "mission-commander",
      access_level: accessLevel,
    };
    retryableSubmissionRef.current = { correlationId, requestKey };
    setActionStatus({ state: "pending", message: "Command submission pending." });
    const result = await awaitWithTerminalPolling(
      client.submitShellTerminalCommand(request),
      load,
    );
    if (result.kind !== "command-result") {
      setActionStatus({ state: "rejected", message: result.message });
      onWorkstationActionTurn?.({
        id: `${correlationId}:reaction:rejected`,
        content: `Shell Terminal command rejected: ${result.message}`,
        source: "orchestrator",
        outcome: "rejected",
      });
      await load();
      return;
    }
    retryableSubmissionRef.current = null;
    onCommandTurnAvailable?.(result.result.command_id);
    setTranscript((entries) => [...entries, { ...result.result, command }]);
    setCommandDraft((current) => (current.trim() === command ? "" : current));
    setActionStatus({ state: "acknowledged", message: `Command ${result.result.status}.` });
    await load();
  }, [
    accessLevel,
    client,
    load,
    onCommandTurnAvailable,
    onWorkstationActionTurn,
    requestedPathsDraft,
    workingDirectory,
  ]);

  const submit = useCallback(async () => {
    await submitCommand(commandDraft);
  }, [commandDraft, submitCommand]);

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
    const result = await awaitWithTerminalPolling(
      client.decideShellTerminalCommand({
        command_id: command.command_id,
        decision,
        actor: "mission-commander",
        reason,
      }),
      load,
    );
    if (result.kind !== "command-result") {
      setActionStatus({ state: "rejected", message: result.message });
      emitActionFinish(correlationId, "rejected", result.message);
      await load();
      return;
    }
    onCommandTurnAvailable?.(result.result.command_id);
    if (result.result.stdout || result.result.stderr) {
      setTranscript((entries) => [...entries, { ...result.result, command: command.command }]);
    }
    setActionStatus({ state: "acknowledged", message: `Command ${result.result.status}.` });
    emitActionFinish(correlationId, "acknowledged", `Command ${result.result.status}.`);
    await load();
  }, [client, denialReasons, emitActionFinish, emitActionStart, load, onCommandTurnAvailable]);

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
      request_id: request.requestId,
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

  const denyGrantRequest = useCallback(async (requestId: string) => {
    const request = contextualGrantRequest;
    if (!request || request.requestId !== requestId || request.status !== "pending") return;
    pathGrantDenialAttemptRef.current += 1;
    const correlationId =
      `path-grant-denied-${requestId}-${pathGrantDenialAttemptRef.current}`;
    if (!client.denyAdditionalPathGrant) {
      const message = "Additional Path Grant denial transport is unavailable.";
      setActionStatus({ state: "rejected", message });
      emitActionFinish(correlationId, "rejected", message);
      return;
    }
    emitActionStart(correlationId, `Deny Additional Path Grant for ${request.path}`);
    setActionStatus({ state: "pending", message: "Additional Path Grant denial pending." });
    const result = await client.denyAdditionalPathGrant({
      correlation_id: correlationId,
      request_id: request.requestId,
      expected_revision: projection?.revision ?? 0,
      path: request.path,
      access_level: request.accessLevel,
      duration_seconds: request.durationSeconds,
      requester: "mission-commander",
      reason: request.reason,
      affected_action: request.affectedAction,
    });
    if (result.kind !== "path-grant-denied") {
      setActionStatus({ state: "rejected", message: result.message });
      emitActionFinish(correlationId, "rejected", result.message);
      return;
    }
    setContextualGrantRequest({ ...request, status: "denied" });
    const message =
      `Recorded ${result.denial.denial_id}; ${request.affectedAction} remains blocked.`;
    setActionStatus({ state: "acknowledged", message });
    emitActionFinish(correlationId, "acknowledged", message);
    await load();
  }, [client, contextualGrantRequest, emitActionFinish, emitActionStart, load, projection?.revision]);

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
    submitCommand,
    setDenialReason,
    decide,
    createGrantForRequest,
    denyGrantRequest,
  };
}

async function awaitWithTerminalPolling<T>(
  operation: Promise<T>,
  reload: () => Promise<boolean>,
): Promise<T> {
  let settled = false;
  let pollTimer: number | undefined;
  const poll = (): void => {
    pollTimer = window.setTimeout(() => {
      if (settled) return;
      void reload()
        .catch(() => false)
        .finally(() => {
          if (!settled) poll();
        });
    }, 150);
  };
  poll();
  try {
    return await operation;
  } finally {
    settled = true;
    if (pollTimer !== undefined) window.clearTimeout(pollTimer);
  }
}

function contextualGrantRequestFromRecord(
  request: AdditionalPathGrantRequestRecord,
): ContextualPathGrantRequest {
  return {
    requestId: request.request_id,
    path: request.path,
    accessLevel: request.access_level,
    durationSeconds: request.duration_seconds,
    reason: request.reason,
    affectedAction: request.affected_action,
    status: request.status,
  };
}
