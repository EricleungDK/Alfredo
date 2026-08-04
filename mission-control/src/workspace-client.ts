import { invoke as tauriInvoke, isTauri } from "@tauri-apps/api/core";
import type {
  AlfredoLaunchContext,
  AlfredoLaunchContextResult,
  CodingWorkspaceAcknowledgement,
  CodingWorkspaceSelectionRequest,
  CodingWorkspaceSelectionResult,
  MissionChoiceAcknowledgement,
  MissionChoiceRequest,
  MissionChoiceResult,
  AgentCapabilityCatalog,
  AgentCapabilityCatalogResult,
  AgentConsoleHistory,
  AgentConsoleHistoryResult,
  AgentConsoleMessage,
  AgentConsoleMessageRequest,
  AgentConsoleMessageResult,
  AgentConsoleResponseProjection,
  AgentConsoleResponseRequest,
  AgentConsoleResponseResult,
  AdHocDelegationProposalRequest,
  AdHocDelegationProposalResult,
  ActivityJournalFilters,
  ActivityJournalLoadResult,
  ActivityJournalProjection,
  AdditionalPathGrant,
  AdditionalPathGrantCreateResult,
  AdditionalPathGrantDenial,
  AdditionalPathGrantDenialRequest,
  AdditionalPathGrantDenialResult,
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
  PerformanceMarkAcknowledgement,
  PerformanceMarkRequest,
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
  WorkstationActionAcknowledgement,
  WorkstationActionRequest,
  WorkstationActionResult,
  WorkstationSessionRunProjection,
  WorkstationSessionRunRequest,
  WorkstationSessionRunResult,
  SessionArtifactProjection,
  SessionArtifactReadRequest,
  SessionArtifactReadResult,
} from "./contracts";

export interface WorkspaceClient {
  recordPerformanceMark?(
    request: PerformanceMarkRequest,
  ): Promise<PerformanceMarkAcknowledgement>;
  loadLaunchContext?(): Promise<AlfredoLaunchContextResult>;
  selectCodingWorkspace?(
    request: CodingWorkspaceSelectionRequest,
  ): Promise<CodingWorkspaceSelectionResult>;
  chooseMission?(request: MissionChoiceRequest): Promise<MissionChoiceResult>;
  loadAgentCapabilities?(): Promise<AgentCapabilityCatalogResult>;
  loadSnapshot(): Promise<WorkspaceLoadResult>;
  loadConsoleHistory?(): Promise<AgentConsoleHistoryResult>;
  loadUpdates?(afterRevision: number): Promise<WorkspaceUpdatesResult>;
  submitAction?(action: WorkspaceActionRequest): Promise<WorkspaceActionResult>;
  changeScope?(scope: WorkspaceScopeRequest): Promise<WorkspaceActionResult>;
  switchMission?(request: WorkspaceMissionSwitchRequest): Promise<WorkspaceActionResult>;
  appendConsoleMessage?(message: AgentConsoleMessageRequest): Promise<AgentConsoleMessageResult>;
  generateConsoleResponse?(
    request: AgentConsoleResponseRequest,
  ): Promise<AgentConsoleResponseResult>;
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
  denyAdditionalPathGrant?(
    request: AdditionalPathGrantDenialRequest,
  ): Promise<AdditionalPathGrantDenialResult>;
  submitReviewDecision?(request: ReviewDecisionRequest): Promise<ReviewDecisionResult>;
  loadWorkspaceQueue?(): Promise<WorkspaceQueueLoadResult>;
  submitAdHocDelegationProposal?(
    request: AdHocDelegationProposalRequest,
  ): Promise<AdHocDelegationProposalResult>;
  submitWorkspaceQueueDecision?(
    request: WorkspaceQueueDecisionRequest,
  ): Promise<WorkspaceQueueDecisionResult>;
  submitWorkstationAction?(request: WorkstationActionRequest): Promise<WorkstationActionResult>;
  runWorkstationSession?(request: WorkstationSessionRunRequest): Promise<WorkstationSessionRunResult>;
  loadSessionArtifact?(request: SessionArtifactReadRequest): Promise<SessionArtifactReadResult>;
  loadMissionDrafts?(): Promise<MissionDraftLoadResult>;
  submitMissionDraftCreate?(request: MissionDraftCreateRequest): Promise<MissionDraftCreateResult>;
  submitMissionDraftDecision?(
    request: MissionDraftDecisionRequest,
  ): Promise<MissionDraftDecisionResult>;
}

const DESKTOP_BRIDGE_UNAVAILABLE =
  "The browser preview has no Alfredo desktop bridge. Start the managed workstation from the repository root.";

type WorkspaceInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type FetchImplementation = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

export interface AlfredoLocalhostBridgeConfiguration {
  readonly endpoint: string;
  readonly token: string;
}

export class TauriWorkspaceClient implements WorkspaceClient {
  constructor(
    private readonly invokeCommand: WorkspaceInvoke = tauriInvoke,
    private readonly bridgeAvailable: () => boolean = isTauri,
    private readonly performanceMarksEnabled = true,
  ) {}

  async recordPerformanceMark(
    request: PerformanceMarkRequest,
  ): Promise<PerformanceMarkAcknowledgement> {
    if (!this.bridgeAvailable() || !this.performanceMarksEnabled) return { recorded: false };
    return this.invokeCommand<PerformanceMarkAcknowledgement>("performance_mark", { request });
  }

  async loadLaunchContext(): Promise<AlfredoLaunchContextResult> {
    if (!this.bridgeAvailable()) {
      return {
        kind: "launch-context-failure",
        code: "backend-startup-failure",
        message: DESKTOP_BRIDGE_UNAVAILABLE,
        recoverable: false,
      };
    }
    try {
      const context = await this.invokeCommand<AlfredoLaunchContext>("alfredo_launch_context");
      return { kind: "launch-context", context };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "launch-context-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "launch-context-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async selectCodingWorkspace(
    request: CodingWorkspaceSelectionRequest,
  ): Promise<CodingWorkspaceSelectionResult> {
    try {
      const acknowledgement = await this.invokeCommand<CodingWorkspaceAcknowledgement>(
        "coding_workspace_select",
        { request },
      );
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "selection-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "selection-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async chooseMission(request: MissionChoiceRequest): Promise<MissionChoiceResult> {
    try {
      const acknowledgement = await this.invokeCommand<MissionChoiceAcknowledgement>(
        "mission_choice",
        { request },
      );
      return { kind: "acknowledged", acknowledgement };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "mission-choice-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "mission-choice-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async loadAgentCapabilities(): Promise<AgentCapabilityCatalogResult> {
    try {
      const catalog = await this.invokeCommand<AgentCapabilityCatalog>("agent_capabilities");
      return { kind: "capabilities", catalog };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "capabilities-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "capabilities-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async loadSnapshot(): Promise<WorkspaceLoadResult> {
    if (!this.bridgeAvailable()) {
      return {
        kind: "startup-failure",
        message: DESKTOP_BRIDGE_UNAVAILABLE,
        recoverable: false,
      };
    }
    try {
      const snapshot = await this.invokeCommand<WorkspaceSnapshot>("workspace_snapshot");
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
      const batch = await this.invokeCommand<WorkspaceUpdateBatch>("workspace_updates", {
        afterRevision,
      });
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
      const history = await this.invokeCommand<AgentConsoleHistory>("agent_console_history");
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
      const acknowledgement = await this.invokeCommand<WorkspaceActionAcknowledgement>(
        "workspace_action",
        { action },
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

  async changeScope(scope: WorkspaceScopeRequest): Promise<WorkspaceActionResult> {
    try {
      const acknowledgement = await this.invokeCommand<WorkspaceActionAcknowledgement>(
        "workspace_scope",
        { scope },
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

  async switchMission(request: WorkspaceMissionSwitchRequest): Promise<WorkspaceActionResult> {
    try {
      const acknowledgement = await this.invokeCommand<WorkspaceActionAcknowledgement>(
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
      const appended = await this.invokeCommand<AgentConsoleMessage>("agent_console_message", {
        message,
      });
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

  async generateConsoleResponse(
    request: AgentConsoleResponseRequest,
  ): Promise<AgentConsoleResponseResult> {
    try {
      const response = await this.invokeCommand<AgentConsoleResponseProjection>(
        "agent_console_response",
        { request },
      );
      return {
        kind: "message",
        message: response.message,
        route: response.route,
        wayfinder: response.wayfinder,
      };
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
      const projection = await this.invokeCommand<WorkingContextProjection>("working_context");
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
      const acknowledgement = await this.invokeCommand<WorkingContextAcknowledgement>(
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
      const projection = await this.invokeCommand<ReviewWorkspaceProjection>("review_workspace");
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
      const projection = await this.invokeCommand<ActivityJournalProjection>("activity_journal", {
        filters,
      });
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
      const projection = await this.invokeCommand<ShellTerminalProjection>("shell_terminal");
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
      const result = await this.invokeCommand<ShellTerminalCommandResult>("shell_terminal_submit", {
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
      const result = await this.invokeCommand<ShellTerminalCommandResult>(
        "shell_terminal_decision",
        { request },
      );
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
      const grant = await this.invokeCommand<AdditionalPathGrant>("additional_path_grant_create", {
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

  async denyAdditionalPathGrant(
    request: AdditionalPathGrantDenialRequest,
  ): Promise<AdditionalPathGrantDenialResult> {
    try {
      const denial = await this.invokeCommand<AdditionalPathGrantDenial>(
        "additional_path_grant_deny",
        { request },
      );
      return { kind: "path-grant-denied", denial };
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
      const acknowledgement = await this.invokeCommand<ReviewDecisionAcknowledgement>(
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
      const projection = await this.invokeCommand<WorkspaceQueueProjection>("workspace_queue");
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
      const acknowledgement = await this.invokeCommand<WorkspaceQueueAcknowledgement>(
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

  async submitWorkstationAction(
    request: WorkstationActionRequest,
  ): Promise<WorkstationActionResult> {
    try {
      const acknowledgement = await this.invokeCommand<WorkstationActionAcknowledgement>(
        "workstation_action",
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

  async runWorkstationSession(
    request: WorkstationSessionRunRequest,
  ): Promise<WorkstationSessionRunResult> {
    try {
      const session = await this.invokeCommand<WorkstationSessionRunProjection>(
        "workstation_session_run",
        { request },
      );
      return { kind: "session-finished", session };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return { kind: "session-failed", code: error.code, message: error.message };
      }
      return {
        kind: "session-failed",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
      };
    }
  }

  async loadSessionArtifact(
    request: SessionArtifactReadRequest,
  ): Promise<SessionArtifactReadResult> {
    try {
      const artifact = await this.invokeCommand<SessionArtifactProjection>("session_artifact", {
        request,
      });
      return { kind: "session-artifact", artifact };
    } catch (error) {
      if (isBridgeFailure(error)) {
        return {
          kind: "session-artifact-failure",
          code: error.code,
          message: error.message,
          recoverable: error.recoverable,
        };
      }
      return {
        kind: "session-artifact-failure",
        code: "backend-startup-failure",
        message: error instanceof Error ? error.message : String(error),
        recoverable: true,
      };
    }
  }

  async submitAdHocDelegationProposal(
    request: AdHocDelegationProposalRequest,
  ): Promise<AdHocDelegationProposalResult> {
    try {
      const acknowledgement = await this.invokeCommand<WorkspaceQueueAcknowledgement>(
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
      const projection = await this.invokeCommand<MissionDraftProjection>("mission_drafts");
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
      const acknowledgement = await this.invokeCommand<MissionDraftAcknowledgement>(
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
      const acknowledgement = await this.invokeCommand<MissionDraftAcknowledgement>(
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

let localhostRequestSequence = 0;

export function createWorkspaceClient(): WorkspaceClient {
  if (isTauri()) return new TauriWorkspaceClient();
  if (globalThis.__ALFREDO_LOCALHOST_BRIDGE__ !== undefined) {
    return createLocalhostWorkspaceClient(globalThis.__ALFREDO_LOCALHOST_BRIDGE__);
  }
  return new TauriWorkspaceClient();
}

export function createLocalhostWorkspaceClient(
  configuration: AlfredoLocalhostBridgeConfiguration | undefined =
    globalThis.__ALFREDO_LOCALHOST_BRIDGE__,
  fetchImplementation: FetchImplementation = globalThis.fetch,
): TauriWorkspaceClient {
  return new TauriWorkspaceClient(
    createLocalhostInvoke(configuration, fetchImplementation),
    () => true,
    false,
  );
}

function createLocalhostInvoke(
  configuration: AlfredoLocalhostBridgeConfiguration | undefined,
  fetchImplementation: FetchImplementation,
): WorkspaceInvoke {
  return async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
    if (!isLocalhostBridgeConfiguration(configuration)) {
      throw bridgeFailure(
        "contract-failure",
        "The injected localhost bridge configuration is malformed.",
        false,
      );
    }
    if (typeof fetchImplementation !== "function") {
      throw bridgeFailure(
        "backend-startup-failure",
        "The localhost bridge fetch transport is unavailable.",
        true,
      );
    }

    const id = createLocalhostRequestId();
    let response: Response;
    try {
      response = await fetchImplementation(configuration.endpoint, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-Alfredo-Bridge-Token": configuration.token,
        },
        body: JSON.stringify({ id, command, args: args ?? {} }),
      });
    } catch (error) {
      throw bridgeFailure(
        "backend-startup-failure",
        `The localhost bridge request failed: ${errorMessage(error)}`,
        true,
      );
    }

    if (!response.ok) {
      throw bridgeFailure(
        "backend-startup-failure",
        `The localhost bridge request failed with HTTP ${response.status}.`,
        true,
      );
    }

    let envelope: unknown;
    try {
      envelope = await response.json();
    } catch {
      throw malformedLocalhostResponse();
    }
    if (
      !isRecord(envelope) ||
      typeof envelope.id !== "string" ||
      typeof envelope.ok !== "boolean"
    ) {
      throw malformedLocalhostResponse();
    }
    const envelopeKeys = Object.keys(envelope).sort();
    const resultKey = envelope.ok ? "value" : "error";
    if (
      envelopeKeys.length !== 3 ||
      !envelopeKeys.includes("id") ||
      !envelopeKeys.includes("ok") ||
      !envelopeKeys.includes(resultKey)
    ) {
      throw malformedLocalhostResponse();
    }
    if (envelope.id !== id) {
      throw bridgeFailure(
        "contract-failure",
        "The localhost bridge response correlation did not match the request.",
        false,
      );
    }
    if (envelope.ok) {
      if (!Object.prototype.hasOwnProperty.call(envelope, "value")) {
        throw malformedLocalhostResponse();
      }
      return envelope.value as T;
    }
    if (
      !Object.prototype.hasOwnProperty.call(envelope, "error") ||
      !isBridgeFailure(envelope.error)
    ) {
      throw malformedLocalhostResponse();
    }
    throw envelope.error;
  };
}

function createLocalhostRequestId(): string {
  if (typeof globalThis.crypto?.randomUUID === "function") {
    return globalThis.crypto.randomUUID();
  }
  localhostRequestSequence += 1;
  return `alfredo-localhost-${Date.now()}-${localhostRequestSequence}`;
}

function isLocalhostBridgeConfiguration(
  value: unknown,
): value is AlfredoLocalhostBridgeConfiguration {
  return (
    isRecord(value) &&
    typeof value.endpoint === "string" &&
    value.endpoint.length > 0 &&
    typeof value.token === "string" &&
    value.token.length > 0
  );
}

function malformedLocalhostResponse(): {
  code: string;
  message: string;
  recoverable: boolean;
} {
  return bridgeFailure(
    "contract-failure",
    "The localhost bridge returned a malformed response envelope.",
    false,
  );
}

function bridgeFailure(
  code: string,
  message: string,
  recoverable: boolean,
): { code: string; message: string; recoverable: boolean } {
  return { code, message, recoverable };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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
