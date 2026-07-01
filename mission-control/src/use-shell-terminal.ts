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
  readonly grantPath: string;
  readonly grantAccessLevel: PathAccessLevel;
  readonly grantDuration: string;
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
  readonly setGrantPath: (value: string) => void;
  readonly setGrantAccessLevel: (value: PathAccessLevel) => void;
  readonly setGrantDuration: (value: string) => void;
  readonly createGrant: () => Promise<void>;
}

export function useShellTerminal(
  client: WorkspaceClient,
  workspacePath: string,
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
  const [grantPath, setGrantPath] = useState("");
  const [grantAccessLevel, setGrantAccessLevel] = useState<PathAccessLevel>("read");
  const [grantDuration, setGrantDuration] = useState("900");

  useEffect(() => {
    if (workspacePath) setWorkingDirectory((current) => current || workspacePath);
  }, [workspacePath]);

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
    const result = await client.submitShellTerminalCommand({
      correlation_id: `terminal-command-${Date.now()}`,
      command,
      working_directory: workingDirectory.trim(),
      requested_paths: requestedPathsDraft
        .split(/\r?\n/)
        .map((path) => path.trim())
        .filter(Boolean),
      requester: "mission-commander",
      access_level: accessLevel,
    });
    if (result.kind !== "command-result") {
      setActionStatus({ state: "rejected", message: result.message });
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
    setActionStatus({ state: "pending", message: `Decision pending for ${command.command_id}.` });
    const result = await client.decideShellTerminalCommand({
      command_id: command.command_id,
      decision,
      actor: "mission-commander",
      reason,
    });
    if (result.kind !== "command-result") {
      setActionStatus({ state: "rejected", message: result.message });
      return;
    }
    if (result.result.stdout || result.result.stderr) {
      setTranscript((entries) => [...entries, { ...result.result, command: command.command }]);
    }
    setActionStatus({ state: "acknowledged", message: `Command ${result.result.status}.` });
    await load();
  }, [client, denialReasons, load]);

  const createGrant = useCallback(async () => {
    const durationSeconds = Number(grantDuration);
    if (!grantPath.trim() || !Number.isInteger(durationSeconds) || durationSeconds <= 0) {
      setActionStatus({
        state: "rejected",
        message: "Path and a positive whole-second duration are required.",
      });
      return;
    }
    if (!client.createAdditionalPathGrant) {
      setActionStatus({ state: "rejected", message: "Additional Path Grant transport is unavailable." });
      return;
    }
    setActionStatus({ state: "pending", message: "Additional Path Grant creation pending." });
    const result = await client.createAdditionalPathGrant({
      correlation_id: `path-grant-${Date.now()}`,
      path: grantPath.trim(),
      access_level: grantAccessLevel,
      duration_seconds: durationSeconds,
      requester: "mission-commander",
    });
    if (result.kind !== "path-grant") {
      setActionStatus({ state: "rejected", message: result.message });
      return;
    }
    setGrantPath("");
    setActionStatus({ state: "acknowledged", message: `Created ${result.grant.grant_id}.` });
    await load();
  }, [client, grantAccessLevel, grantDuration, grantPath, load]);

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
    grantPath,
    grantAccessLevel,
    grantDuration,
    setCommandDraft,
    setWorkingDirectory,
    setRequestedPathsDraft,
    setAccessLevel,
    load,
    submit,
    setDenialReason,
    decide,
    setGrantPath,
    setGrantAccessLevel,
    setGrantDuration,
    createGrant,
  };
}
