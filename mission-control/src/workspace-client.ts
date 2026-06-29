import { invoke } from "@tauri-apps/api/core";
import type {
  AgentConsoleHistory,
  AgentConsoleHistoryResult,
  AgentConsoleMessage,
  AgentConsoleMessageRequest,
  AgentConsoleMessageResult,
  AdHocDelegationProposalRequest,
  AdHocDelegationProposalResult,
  ActivityJournalFilters,
  ActivityJournalLoadResult,
  ActivityJournalProjection,
  AdditionalPathGrant,
  AdditionalPathGrantCreateResult,
  AdditionalPathGrantRequest,
  ShellTerminalLoadResult,
  ShellTerminalProjection,
  ShellTerminalCommandRequest,
  ShellTerminalCommandResult,
  ShellTerminalSubmitResult,
  ShellTerminalDecisionRequest,
  ShellTerminalDecisionResult,
  MissionDraftAcknowledgement,
  MissionDraftCreateRequest,
  MissionDraftCreateResult,
  MissionDraftDecisionRequest,
  MissionDraftDecisionResult,
  MissionDraftLoadResult,
  MissionDraftProjection,
  WorkspaceLoadResult,
  WorkspaceActionRequest,
  WorkspaceActionAcknowledgement,
  WorkspaceActionResult,
  WorkspaceMissionSwitchRequest,
  WorkspaceSnapshot,
  WorkspaceScopeRequest,
  WorkspaceUpdateBatch,
  WorkspaceUpdatesResult,
  WorkingContextAcknowledgement,
  WorkingContextCurationRequest,
  WorkingContextCurationResult,
  WorkingContextLoadResult,
  WorkingContextProjection,
  ReviewDecisionAcknowledgement,
  ReviewDecisionRequest,
  ReviewDecisionResult,
  ReviewWorkspaceLoadResult,
  ReviewWorkspaceProjection,
  WorkspaceQueueAcknowledgement,
  WorkspaceQueueDecisionRequest,
  WorkspaceQueueDecisionResult,
  WorkspaceQueueLoadResult,
  WorkspaceQueueProjection,
} from "./contracts";

export interface WorkspaceClient {
  loadSnapshot(): Promise<WorkspaceLoadResult>;
  loadConsoleHistory?(): Promise<AgentConsoleHistoryResult>;
  loadUpdates?(afterRevision: number): Promise<WorkspaceUpdatesResult>;
  submitAction?(action: WorkspaceActionRequest): Promise<WorkspaceActionResult>;
  changeScope?(scope: WorkspaceScopeRequest): Promise<WorkspaceActionResult>;
  switchMission?(request: WorkspaceMissionSwitchRequest): Promise<WorkspaceActionResult>;
  appendConsoleMessage?(message: AgentConsoleMessageRequest): Promise<AgentConsoleMessageResult>;
  loadWorkingContext?(): Promise<WorkingContextLoadResult>;
  curateWorkingContext?(
    request: WorkingContextCurationRequest,
  ): Promise<WorkingContextCurationResult>;
  loadReviewWorkspace?(): Promise<ReviewWorkspaceLoadResult>;
  loadActivityJournal?(filters?: ActivityJournalFilters): Promise<ActivityJournalLoadResult>;
  loadShellTerminal?(): Promise<ShellTerminalLoadResult>;
  submitShellTerminalCommand?(
    request: ShellTerminalCommandRequest,
  ): Promise<ShellTerminalSubmitResult>;
  decideShellTerminalCommand?(
    request: ShellTerminalDecisionRequest,
  ): Promise<ShellTerminalDecisionResult>;
  createAdditionalPathGrant?(
    request: AdditionalPathGrantRequest,
  ): Promise<AdditionalPathGrantCreateResult>;
  submitReviewDecision?(request: ReviewDecisionRequest): Promise<ReviewDecisionResult>;
  loadWorkspaceQueue?(): Promise<WorkspaceQueueLoadResult>;
  submitAdHocDelegationProposal?(
    request: AdHocDelegationProposalRequest,
  ): Promise<AdHocDelegationProposalResult>;
  submitWorkspaceQueueDecision?(
    request: WorkspaceQueueDecisionRequest,
  ): Promise<WorkspaceQueueDecisionResult>;
  loadMissionDrafts?(): Promise<MissionDraftLoadResult>;
  submitMissionDraftCreate?(request: MissionDraftCreateRequest): Promise<MissionDraftCreateResult>;
  submitMissionDraftDecision?(
    request: MissionDraftDecisionRequest,
  ): Promise<MissionDraftDecisionResult>;
}

export class TauriWorkspaceClient implements WorkspaceClient {
  async loadSnapshot(): Promise<WorkspaceLoadResult> {
    try {
      const snapshot = await invoke<WorkspaceSnapshot>("workspace_snapshot");
      return {
        kind: snapshot.workspace_session.status === "empty" ? "empty" : "ready",
        snapshot,
      };
    } catch (error) {
      if (isBridgeFailure(error)) {
        const kind = error.code === "backend-startup-failure" ? "startup-failure" : error.code;
        if (
          kind === "startup-failure" ||
          kind === "persistence-read-failure" ||
          kind === "contract-failure"
        ) {
          return { kind, message: error.message, recoverable: error.recoverable };
        }
      }
      return {
        kind: "startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async loadUpdates(afterRevision: number): Promise<WorkspaceUpdatesResult> {
    try {
      const batch = await invoke<WorkspaceUpdateBatch>("workspace_updates", { afterRevision });
      return { kind: "updates", batch };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "sync-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "sync-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async loadConsoleHistory(): Promise<AgentConsoleHistoryResult> {
    try {
      const history = await invoke<AgentConsoleHistory>("agent_console_history");
      return { kind: "history", history };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "history-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "history-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async submitAction(action: WorkspaceActionRequest): Promise<WorkspaceActionResult> {
    try {
      const acknowledgement = await invoke<WorkspaceActionAcknowledgement>("workspace_action", {
        action,
      });
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: error.code === "stale-action" ? "stale" : "rejected",
          code: error.code,
          message: error.message,
          current_revision:
            typeof (error as Record<string, unknown>).current_revision === "number"
              ? ((error as Record<string, unknown>).current_revision as number)
              : undefined,
        };
      }
      return {
        kind: "rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async changeScope(scope: WorkspaceScopeRequest): Promise<WorkspaceActionResult> {
    try {
      const acknowledgement = await invoke<WorkspaceActionAcknowledgement>("workspace_scope", {
        scope,
      });
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: error.code === "stale-action" ? "stale" : "rejected",
          code: error.code,
          message: error.message,
        };
      }
      return {
        kind: "rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async switchMission(request: WorkspaceMissionSwitchRequest): Promise<WorkspaceActionResult> {
    try {
      const acknowledgement = await invoke<WorkspaceActionAcknowledgement>(
        "workspace_mission_switch",
        { request },
      );
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: error.code === "stale-action" ? "stale" : "rejected",
          code: error.code,
          message: error.message,
          current_revision:
            typeof (error as Record<string, unknown>).current_revision === "number"
              ? ((error as Record<string, unknown>).current_revision as number)
              : undefined,
        };
      }
      return {
        kind: "rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async appendConsoleMessage(
    message: AgentConsoleMessageRequest,
  ): Promise<AgentConsoleMessageResult> {
    try {
      const appended = await invoke<AgentConsoleMessage>("agent_console_message", { message });
      return { kind: "message", message: appended };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return { kind: "message-rejected", code: error.code, message: error.message };
      }
      return {
        kind: "message-rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async loadWorkingContext(): Promise<WorkingContextLoadResult> {
    try {
      const projection = await invoke<WorkingContextProjection>("working_context");
      return { kind: "working-context", projection };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "working-context-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "working-context-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async curateWorkingContext(
    request: WorkingContextCurationRequest,
  ): Promise<WorkingContextCurationResult> {
    try {
      const acknowledgement = await invoke<WorkingContextAcknowledgement>(
        "working_context_curate",
        { request },
      );
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: error.code === "stale-action" ? "stale" : "rejected",
          code: error.code,
          message: error.message,
        };
      }
      return {
        kind: "rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async loadReviewWorkspace(): Promise<ReviewWorkspaceLoadResult> {
    try {
      const projection = await invoke<ReviewWorkspaceProjection>("review_workspace");
      return { kind: "review-workspace", projection };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "review-workspace-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "review-workspace-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async loadActivityJournal(
    filters: ActivityJournalFilters = {},
  ): Promise<ActivityJournalLoadResult> {
    try {
      const projection = await invoke<ActivityJournalProjection>("activity_journal", { filters });
      return { kind: "activity-journal", projection };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "activity-journal-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "activity-journal-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async loadShellTerminal(): Promise<ShellTerminalLoadResult> {
    try {
      const projection = await invoke<ShellTerminalProjection>("shell_terminal");
      return { kind: "shell-terminal", projection };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "shell-terminal-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "shell-terminal-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async submitShellTerminalCommand(
    request: ShellTerminalCommandRequest,
  ): Promise<ShellTerminalSubmitResult> {
    try {
      const result = await invoke<ShellTerminalCommandResult>("shell_terminal_submit", {
        request,
      });
      return { kind: "command-result", result };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return { kind: "command-rejected", code: error.code, message: error.message };
      }
      return {
        kind: "command-rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async decideShellTerminalCommand(
    request: ShellTerminalDecisionRequest,
  ): Promise<ShellTerminalDecisionResult> {
    try {
      const result = await invoke<ShellTerminalCommandResult>("shell_terminal_decision", {
        request,
      });
      return { kind: "command-result", result };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return { kind: "command-rejected", code: error.code, message: error.message };
      }
      return {
        kind: "command-rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async createAdditionalPathGrant(
    request: AdditionalPathGrantRequest,
  ): Promise<AdditionalPathGrantCreateResult> {
    try {
      const grant = await invoke<AdditionalPathGrant>("additional_path_grant_create", {
        request,
      });
      return { kind: "path-grant", grant };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return { kind: "path-grant-rejected", code: error.code, message: error.message };
      }
      return {
        kind: "path-grant-rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async submitReviewDecision(request: ReviewDecisionRequest): Promise<ReviewDecisionResult> {
    try {
      const acknowledgement = await invoke<ReviewDecisionAcknowledgement>(
        "review_decision",
        { request },
      );
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: error.code === "stale-action" ? "stale" : "rejected",
          code: error.code,
          message: error.message,
          current_revision:
            typeof (error as Record<string, unknown>).current_revision === "number"
              ? ((error as Record<string, unknown>).current_revision as number)
              : undefined,
        };
      }
      return {
        kind: "rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async loadWorkspaceQueue(): Promise<WorkspaceQueueLoadResult> {
    try {
      const projection = await invoke<WorkspaceQueueProjection>("workspace_queue");
      return { kind: "workspace-queue", projection };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "workspace-queue-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "workspace-queue-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async submitWorkspaceQueueDecision(
    request: WorkspaceQueueDecisionRequest,
  ): Promise<WorkspaceQueueDecisionResult> {
    try {
      const acknowledgement = await invoke<WorkspaceQueueAcknowledgement>(
        "workspace_queue_decision",
        { request },
      );
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: error.code === "stale-action" ? "stale" : "rejected",
          code: error.code,
          message: error.message,
          current_revision:
            typeof (error as Record<string, unknown>).current_revision === "number"
              ? ((error as Record<string, unknown>).current_revision as number)
              : undefined,
        };
      }
      return {
        kind: "rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async submitAdHocDelegationProposal(
    request: AdHocDelegationProposalRequest,
  ): Promise<AdHocDelegationProposalResult> {
    try {
      const acknowledgement = await invoke<WorkspaceQueueAcknowledgement>(
        "ad_hoc_delegation_proposal",
        { request },
      );
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: error.code === "stale-action" ? "stale" : "rejected",
          code: error.code,
          message: error.message,
          current_revision:
            typeof (error as Record<string, unknown>).current_revision === "number"
              ? ((error as Record<string, unknown>).current_revision as number)
              : undefined,
        };
      }
      return {
        kind: "rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async loadMissionDrafts(): Promise<MissionDraftLoadResult> {
    try {
      const projection = await invoke<MissionDraftProjection>("mission_drafts");
      return { kind: "mission-drafts", projection };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "mission-drafts-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "mission-drafts-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async submitMissionDraftCreate(
    request: MissionDraftCreateRequest,
  ): Promise<MissionDraftCreateResult> {
    try {
      const acknowledgement = await invoke<MissionDraftAcknowledgement>(
        "mission_draft_create",
        { request },
      );
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: error.code === "stale-action" ? "stale" : "rejected",
          code: error.code,
          message: error.message,
          current_revision:
            typeof (error as Record<string, unknown>).current_revision === "number"
              ? ((error as Record<string, unknown>).current_revision as number)
              : undefined,
        };
      }
      return {
        kind: "rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async submitMissionDraftDecision(
    request: MissionDraftDecisionRequest,
  ): Promise<MissionDraftDecisionResult> {
    try {
      const acknowledgement = await invoke<MissionDraftAcknowledgement>(
        "mission_draft_decision",
        { request },
      );
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: error.code === "stale-action" ? "stale" : "rejected",
          code: error.code,
          message: error.message,
          current_revision:
            typeof (error as Record<string, unknown>).current_revision === "number"
              ? ((error as Record<string, unknown>).current_revision as number)
              : undefined,
        };
      }
      return {
        kind: "rejected",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }
}

function isBridgeFailure(
  error: unknown,
): error is { code: string; message: string; recoverable: boolean } {
  return (
    typeof error === "object" &&
    error !== null &&
    typeof (error as Record<string, unknown>).code === "string" &&
    typeof (error as Record<string, unknown>).message === "string" &&
    typeof (error as Record<string, unknown>).recoverable === "boolean"
  );
}
