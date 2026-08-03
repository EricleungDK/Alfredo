export type ConversationScopeKind = "working-directory" | "mission" | "issue-slice";

export type PerformanceStage =
  | "S0"
  | "S1"
  | "S2"
  | "S3"
  | "S4"
  | "S5"
  | "S6"
  | "S7"
  | "S8"
  | "S9"
  | "R0"
  | "R1"
  | "R2"
  | "R3"
  | "R4"
  | "R5"
  | "R6";

export interface PerformanceMarkRequest {
  readonly stage: PerformanceStage;
  readonly boundary: "start" | "end";
  readonly clock: "native" | "frontend";
  readonly monotonic_ns: string;
  readonly clock_id: string;
  readonly detail: Readonly<Record<string, unknown>>;
}

export interface PerformanceMarkAcknowledgement {
  readonly recorded: boolean;
}

export interface ConversationScope {
  readonly kind: ConversationScopeKind;
  readonly target_id: string;
  readonly label: string;
  readonly mission_id?: string | null;
}

export interface AlfredoLaunchContext {
  readonly schema_version: 1;
  readonly selected_agent: string;
  readonly selected_model: string;
  readonly starting_location: string;
  /** Backend-computed create target; null means no safe child target exists. */
  readonly suggested_workspace_path?: string | null;
  readonly coding_workspace: string | null;
  readonly active_mission: string | null;
  readonly revision?: number;
  readonly known_missions?: readonly MissionChoiceOption[];
  readonly phase: "selection-required" | "mission-choice-required" | "workspace-ready";
  readonly runtime_root: string;
  readonly recent_workspaces: readonly string[];
}

export interface MissionChoiceOption {
  readonly id: string;
  readonly title: string;
}

export type AlfredoLaunchContextResult =
  | { readonly kind: "launch-context"; readonly context: AlfredoLaunchContext }
  | {
      readonly kind: "launch-context-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export interface CodingWorkspaceSelectionRequest {
  readonly correlation_id: string;
  readonly workspace_path: string;
  readonly selection_mode: "existing" | "create";
}

export interface CodingWorkspaceAcknowledgement {
  readonly schema_version: 1;
  readonly correlation_id: string;
  readonly outcome: "acknowledged";
  readonly starting_location: string;
  readonly coding_workspace: string;
  readonly selection_mode: "existing" | "create";
  readonly active_mission: null;
  readonly replayed: boolean;
  readonly message: string;
  readonly known_missions?: readonly MissionChoiceOption[];
}

export type CodingWorkspaceSelectionResult =
  | {
      readonly kind: "acknowledged";
      readonly acknowledgement: CodingWorkspaceAcknowledgement;
    }
  | {
      readonly kind: "selection-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export interface MissionChoiceRequest {
  readonly correlation_id: string;
  readonly expected_revision: number;
  readonly choice: "resume" | "new";
  readonly mission_id: string;
  readonly mission_title?: string;
}

export interface MissionChoiceAcknowledgement {
  readonly schema_version: 1;
  readonly correlation_id: string;
  readonly outcome: "acknowledged";
  readonly coding_workspace: string;
  readonly choice: "resume" | "new";
  readonly active_mission: string;
  readonly revision: number;
  readonly replayed: boolean;
  readonly missions: readonly MissionChoiceOption[];
  readonly message: string;
}

export type MissionChoiceResult =
  | {
      readonly kind: "acknowledged";
      readonly acknowledgement: MissionChoiceAcknowledgement;
    }
  | {
      readonly kind: "mission-choice-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export interface SkillCapability {
  readonly name: string;
  readonly description: string;
  readonly source: string;
  readonly invocation: string;
}

export interface CommandCapability {
  readonly name: string;
  readonly usage: string;
  readonly description: string;
  readonly category: string;
}

export interface AgentCapability {
  readonly id: string;
  readonly role: string;
  readonly provider: string;
  readonly runner: string;
  readonly model: string;
  readonly routing: string;
  readonly availability: string;
  readonly availability_reason: string;
  readonly assignable?: boolean;
  readonly delegate_only?: boolean;
  readonly requires_approval?: boolean;
}

export interface AgentCapabilityCatalog {
  readonly schema_version: 1;
  readonly default_agent_id: string;
  readonly skills: readonly SkillCapability[];
  readonly commands: readonly CommandCapability[];
  readonly agents: readonly AgentCapability[];
}

export type AgentCapabilityCatalogResult =
  | { readonly kind: "capabilities"; readonly catalog: AgentCapabilityCatalog }
  | {
      readonly kind: "capabilities-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export type AgentConsoleRole = "user" | "assistant" | "system";
export type AgentConsoleOutcome =
  | "proposed"
  | "pending"
  | "acknowledged"
  | "rejected"
  | "model-commentary";

export interface AgentConsoleMessage {
  readonly message_id: string;
  readonly sequence: number;
  readonly role: AgentConsoleRole;
  readonly content: string;
  readonly scope: ConversationScope;
  readonly outcome: AgentConsoleOutcome;
  readonly source: string;
  readonly correlation_id?: string;
  readonly action_phase?: string;
  readonly action_outcome?: "no-action" | "awaiting-orchestrator" | "";
  readonly action_message?: string;
}

export interface AgentConsoleHistory {
  readonly schema_version: 1;
  readonly messages: readonly AgentConsoleMessage[];
}

export type AgentConsoleHistoryResult =
  | { readonly kind: "history"; readonly history: AgentConsoleHistory }
  | {
      readonly kind: "history-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export type ActivityActor =
  | "mission-commander"
  | "orchestrator"
  | "frontier-model"
  | "local-agent";

export interface ActivityAffectedEntity {
  readonly entity_type: string;
  readonly entity_id: string;
  readonly label: string;
  readonly href: string;
}

export interface ActivityJournalEntry {
  readonly entry_id: string;
  readonly sequence: number;
  readonly recorded_at: string;
  readonly actor: ActivityActor;
  readonly action_type: string;
  readonly summary: string;
  readonly affected_entities: readonly ActivityAffectedEntity[];
  readonly evidence_links: readonly string[];
  readonly correlation_id: string;
}

export interface ActivityJournalProjection {
  readonly schema_version: 1;
  readonly revision: number;
  readonly entries: readonly ActivityJournalEntry[];
}

export interface ActivityJournalFilters {
  readonly search?: string;
  readonly mission_id?: string;
  readonly actor?: ActivityActor | "";
  readonly action_type?: string;
  readonly started_at?: string;
  readonly ended_at?: string;
}

export type ActivityJournalLoadResult =
  | { readonly kind: "activity-journal"; readonly projection: ActivityJournalProjection }
  | {
      readonly kind: "activity-journal-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export type ShellTerminalClassification =
  | "auto-allowed"
  | "frontier-approvable"
  | "human-required";
export type ShellTerminalCommandStatus =
  | "pending-approval"
  | "executing"
  | "outcome-unknown"
  | "completed"
  | "failed"
  | "denied";
export type PathAccessLevel = "read" | "write";

export interface ShellTerminalCommandRecord {
  readonly command_id: string;
  readonly correlation_id: string;
  readonly command: string;
  readonly classification: ShellTerminalClassification;
  readonly status: ShellTerminalCommandStatus;
  readonly exit_code: number | null;
  readonly working_directory: string;
  readonly requested_paths: readonly string[];
  readonly access_level: PathAccessLevel;
  readonly requester: string;
  readonly approver: string;
  readonly decider: string;
  readonly reason: string;
}

export interface AdditionalPathGrant {
  readonly grant_id: string;
  readonly correlation_id: string;
  readonly path: string;
  readonly access_level: PathAccessLevel;
  readonly duration_seconds: number;
  readonly granted_by: "mission-commander";
  readonly granted_at: string;
  readonly expires_at: string;
  readonly request_id?: string;
}

export interface AdditionalPathGrantRequestRecord {
  readonly request_id: string;
  readonly correlation_id: string;
  readonly mission_id: string;
  readonly path: string;
  readonly access_level: PathAccessLevel;
  readonly duration_seconds: number;
  readonly requester: string;
  readonly requested_at: string;
  readonly reason: string;
  readonly affected_action: string;
  readonly status: "pending" | "granted" | "denied";
}

export interface AdditionalPathGrantDenial {
  readonly denial_id: string;
  readonly correlation_id: string;
  readonly request_id: string;
  readonly path: string;
  readonly access_level: PathAccessLevel;
  readonly duration_seconds: number;
  readonly denied_by: "mission-commander";
  readonly denied_at: string;
  readonly reason: string;
  readonly affected_action: string;
}

export interface ShellTerminalProjection {
  readonly schema_version: 1;
  readonly revision: number;
  readonly commands: readonly ShellTerminalCommandRecord[];
  readonly grants: readonly AdditionalPathGrant[];
  readonly grant_denials?: readonly AdditionalPathGrantDenial[];
  readonly path_grant_requests?: readonly AdditionalPathGrantRequestRecord[];
}

export type ShellTerminalLoadResult =
  | { readonly kind: "shell-terminal"; readonly projection: ShellTerminalProjection }
  | {
      readonly kind: "shell-terminal-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export interface ShellTerminalCommandRequest {
  readonly correlation_id: string;
  readonly command: string;
  readonly working_directory: string;
  readonly requested_paths: readonly string[];
  readonly requester: string;
  readonly access_level: PathAccessLevel;
}

export interface ShellTerminalCommandResult {
  readonly command_id: string;
  readonly correlation_id: string;
  readonly classification: ShellTerminalClassification;
  readonly status: ShellTerminalCommandStatus;
  readonly exit_code: number | null;
  readonly stdout: string;
  readonly stderr: string;
}

export type ShellTerminalSubmitResult =
  | { readonly kind: "command-result"; readonly result: ShellTerminalCommandResult }
  | { readonly kind: "command-rejected"; readonly code: string; readonly message: string };

export interface ShellTerminalDecisionRequest {
  readonly command_id: string;
  readonly decision: "approve" | "deny";
  readonly actor: "mission-commander" | "frontier-model";
  readonly reason: string;
}

export type ShellTerminalDecisionResult =
  | { readonly kind: "command-result"; readonly result: ShellTerminalCommandResult }
  | { readonly kind: "command-rejected"; readonly code: string; readonly message: string };

export interface AdditionalPathGrantRequest {
  readonly correlation_id: string;
  readonly request_id: string;
  readonly expected_revision: number;
  readonly path: string;
  readonly access_level: PathAccessLevel;
  readonly duration_seconds: number;
  readonly requester: "mission-commander";
}

export type AdditionalPathGrantCreateResult =
  | { readonly kind: "path-grant"; readonly grant: AdditionalPathGrant }
  | { readonly kind: "path-grant-rejected"; readonly code: string; readonly message: string };

export interface AdditionalPathGrantDenialRequest {
  readonly correlation_id: string;
  readonly request_id: string;
  readonly expected_revision: number;
  readonly path: string;
  readonly access_level: PathAccessLevel;
  readonly duration_seconds: number;
  readonly requester: "mission-commander";
  readonly reason: string;
  readonly affected_action: string;
}

export type AdditionalPathGrantDenialResult =
  | { readonly kind: "path-grant-denied"; readonly denial: AdditionalPathGrantDenial }
  | { readonly kind: "path-grant-rejected"; readonly code: string; readonly message: string };

export interface AgentConsoleMessageRequest {
  readonly role: AgentConsoleRole;
  readonly content: string;
  readonly outcome: AgentConsoleOutcome;
  readonly source: string;
  readonly expected_revision: number;
  readonly scope_kind: ConversationScopeKind;
  readonly scope_target: string;
  readonly scope_label: string;
  readonly scope_mission_id?: string;
}

export interface AgentConsoleResponseRequest {
  readonly expected_revision: number;
  readonly message_id: string;
  readonly scope_kind: ConversationScopeKind;
  readonly scope_target: string;
  readonly scope_label: string;
  readonly scope_mission_id?: string;
  readonly agent_id?: string;
}

export type AgentConsoleMessageResult =
  | { readonly kind: "message"; readonly message: AgentConsoleMessage }
  | {
      readonly kind: "message-rejected";
      readonly code: string;
      readonly message: string;
    };

export interface AgentConsoleResponseRoute {
  readonly intent: "discussion" | "coding-task";
  readonly task_request: string;
  readonly acceptance_criteria: readonly string[];
}

export type WayfinderMode = "outside" | "chart" | "work-through";
export type WayfinderGateStatus = "not-applicable" | "pending" | "open";

export interface WayfinderGate {
  readonly status: WayfinderGateStatus;
  readonly opened_by: string;
  readonly receipt_id: string;
}

export interface WayfinderFlow {
  readonly flow_id: string;
  readonly mode: "chart" | "work-through";
  readonly originating_message_id: string;
  readonly scope: ConversationScope;
  readonly reference: string;
}

export interface WayfinderProjection {
  readonly mode: WayfinderMode;
  readonly gate: WayfinderGate;
  readonly flow: WayfinderFlow | null;
  readonly continuing: boolean;
  readonly turn_complete: boolean;
}

export interface AgentConsoleResponseProjection {
  readonly message: AgentConsoleMessage;
  readonly route: AgentConsoleResponseRoute;
  /** Omitted only by a compatible pre-Wayfinder backend. */
  readonly wayfinder?: WayfinderProjection;
}

export type AgentConsoleResponseResult =
  | {
      readonly kind: "message";
      readonly message: AgentConsoleMessage;
      readonly route: AgentConsoleResponseRoute;
      readonly wayfinder?: WayfinderProjection;
    }
  | {
      readonly kind: "message-rejected";
      readonly code: string;
      readonly message: string;
    };

export type WorkingContextSourceKind =
  | "workspace-session"
  | "shared-context"
  | "unresolved-item"
  | "recent-conversation"
  | "deliberate-reference";

export type WorkingContextDisposition = "required" | "included" | "pinned" | "excluded";

export interface WorkingContextSource {
  readonly source_id: string;
  readonly kind: WorkingContextSourceKind;
  readonly label: string;
  readonly content: string;
  readonly governed: boolean;
  readonly eligible: boolean;
  readonly disposition: WorkingContextDisposition;
}

export interface WorkingContextProjection {
  readonly schema_version: 1;
  readonly revision: number;
  readonly scope: ConversationScope;
  readonly sources: readonly WorkingContextSource[];
  readonly content_character_count: number;
}

export type WorkingContextLoadResult =
  | { readonly kind: "working-context"; readonly projection: WorkingContextProjection }
  | {
      readonly kind: "working-context-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export interface WorkingContextCurationRequest {
  readonly source_id: string;
  readonly disposition: "included" | "pinned" | "excluded";
  readonly expected_context_revision: number;
}

export interface WorkingContextAcknowledgement {
  readonly outcome: "acknowledged";
  readonly revision: number;
}

export type WorkingContextCurationResult =
  | {
      readonly kind: "acknowledged";
      readonly acknowledgement: WorkingContextAcknowledgement;
    }
  | {
      readonly kind: "stale" | "rejected";
      readonly code: string;
      readonly message: string;
    };

export interface ReviewWorkspaceEvidence {
  readonly changed_files: readonly string[];
  readonly diff_summary: string;
  readonly commands_run: readonly string[];
  readonly test_results: string;
  readonly risks: string;
  readonly proposed_context_updates: string;
  readonly artifact_links: readonly string[];
}

export interface ReviewWorkspaceVisibilityLimitation {
  readonly path: string;
  readonly classification: string;
  readonly consequence: string;
}

export interface ReviewWorkspaceItem {
  readonly mission_id: string;
  readonly issue_id: string;
  readonly issue_title: string;
  readonly session_id: string;
  readonly assigned_agent: string;
  readonly status: string;
  readonly lifecycle: string;
  readonly evidence_complete: boolean;
  readonly missing_evidence: readonly string[];
  readonly can_accept: boolean;
  readonly evidence: ReviewWorkspaceEvidence;
  readonly visibility_limitations: readonly ReviewWorkspaceVisibilityLimitation[];
}

export interface ReviewWorkspaceProjection {
  readonly schema_version: 1;
  readonly revision: number;
  readonly mission_id: string;
  readonly items: readonly ReviewWorkspaceItem[];
}

export type ReviewWorkspaceLoadResult =
  | { readonly kind: "review-workspace"; readonly projection: ReviewWorkspaceProjection }
  | {
      readonly kind: "review-workspace-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export type ReviewDecision = "accept" | "repair" | "escalate-human";

export interface ReviewDecisionTarget {
  readonly kind: "agent-session";
  readonly id: string;
}

export interface ReviewDecisionRequest {
  readonly correlation_id: string;
  readonly action_type: "review-decision";
  readonly actor: "mission-commander";
  readonly expected_revision: number;
  readonly target: ReviewDecisionTarget;
  readonly mission_id?: string;
  readonly session_id: string;
  readonly decision: ReviewDecision;
  readonly reason: string;
  readonly failure_type?: string;
}

export interface ReviewDecisionAcknowledgement {
  readonly correlation_id: string;
  readonly outcome: "acknowledged";
  readonly revision: number;
  readonly issue_id: string;
  readonly session_id: string;
  readonly review_outcome: string;
  readonly next_action: string;
  readonly issue_lifecycle: string;
  readonly effect_summary: string;
}

export type ReviewDecisionResult =
  | {
      readonly kind: "acknowledged";
      readonly acknowledgement: ReviewDecisionAcknowledgement;
    }
  | {
      readonly kind: "stale" | "rejected";
      readonly code: string;
      readonly message: string;
      readonly current_revision?: number;
    };

export type WorkspaceQueueItemType =
  | "issue-change-proposal"
  | "frontier-confirmation"
  | "ad-hoc-delegation";
export type WorkspaceQueueItemStatus = "pending" | "approved" | "rejected" | "deferred";

export interface WorkspaceQueueItem {
  readonly item_id: string;
  readonly mission_id: string;
  readonly item_type: WorkspaceQueueItemType;
  readonly status: WorkspaceQueueItemStatus;
  readonly source: string;
  readonly requested_action: string;
  readonly affected_boundary: string;
  readonly consequence: string;
  readonly issue_id: string;
  readonly proposed_changes: Readonly<Record<string, unknown>>;
  readonly proposal_correlation_id?: string;
  readonly decision_correlation_id?: string;
}

export interface WorkspaceQueueGroup {
  readonly group_id: string;
  readonly item_type: WorkspaceQueueItemType;
  readonly mission_id: string;
  readonly item_count: number;
  readonly items: readonly WorkspaceQueueItem[];
}

export interface WorkspaceQueueProjection {
  readonly schema_version: 1;
  readonly revision: number;
  readonly items: readonly WorkspaceQueueItem[];
  readonly groups: readonly WorkspaceQueueGroup[];
}

export type WorkspaceQueueLoadResult =
  | { readonly kind: "workspace-queue"; readonly projection: WorkspaceQueueProjection }
  | {
      readonly kind: "workspace-queue-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export type WorkspaceQueueDecision = "approve" | "reject" | "defer";

export interface WorkspaceQueueDecisionRequest {
  readonly correlation_id: string;
  readonly action_type?: "workspace-queue-decision";
  readonly actor?: "mission-commander";
  readonly expected_revision: number;
  readonly target?: {
    readonly kind: "workspace-queue-item";
    readonly id: string;
  };
  readonly item_id: string;
  readonly decision: WorkspaceQueueDecision;
  readonly reason: string;
}

export interface AdHocDelegationProposalRequest {
  readonly correlation_id: string;
  readonly expected_revision: number;
  readonly source: string;
  readonly scope_kind: ConversationScopeKind;
  readonly scope_target: string;
  readonly scope_label: string;
  readonly acceptance_criteria: readonly string[];
  readonly allowed_paths: readonly string[];
  readonly command_policy: Readonly<Record<string, string>>;
  readonly proposed_agent: string;
  readonly originating_message_id: string;
  readonly mission_id?: string;
}

export interface WorkspaceQueueAcknowledgement {
  readonly correlation_id: string;
  readonly outcome: "acknowledged";
  readonly revision: number;
  readonly item_id: string;
  readonly item_status: WorkspaceQueueItemStatus;
  readonly effect_summary: string;
  readonly session_id?: string | null;
}

export type WorkspaceQueueDecisionResult =
  | {
      readonly kind: "acknowledged";
      readonly acknowledgement: WorkspaceQueueAcknowledgement;
    }
  | {
      readonly kind: "stale" | "rejected";
      readonly code: string;
      readonly message: string;
      readonly current_revision?: number;
    };

export type AdHocDelegationProposalResult = WorkspaceQueueDecisionResult;

export type WorkstationActionType =
  | "issue-approve"
  | "issue-launch"
  | "issue-retry"
  | "session-cancel"
  | "model-assignment-change";

export interface WorkstationActionRequest {
  readonly correlation_id: string;
  readonly action_type: WorkstationActionType;
  readonly actor: "mission-commander";
  readonly expected_revision: number;
  readonly target:
    | { readonly kind: "issue-slice"; readonly id: string }
    | { readonly kind: "agent-session"; readonly id: string };
  readonly mission_id?: string;
  readonly issue_id?: string;
  readonly session_id?: string;
  readonly agent_id?: string;
  readonly reason?: string;
  readonly allowed_paths?: readonly string[];
  readonly command_policy?: Readonly<Record<string, string>>;
}

export interface WorkstationActionAcknowledgement {
  readonly correlation_id: string;
  readonly outcome: "acknowledged";
  readonly revision: number;
  readonly action_type: WorkstationActionType;
  readonly issue_id: string;
  readonly session_id: string;
  readonly effect_summary: string;
}

export interface WorkstationSessionRunRequest {
  readonly session_id: string;
  readonly mission_id?: string;
}

export interface WorkstationSessionRunProjection {
  readonly schema_version: 1;
  readonly mission_id: string;
  readonly session_id: string;
  readonly issue_id: string;
  readonly status: string;
  readonly runner_started_at: string;
  readonly runner_ended_at: string;
  readonly runner_exit_status: number | null;
  readonly evidence_valid: boolean;
}

export type WorkstationSessionRunResult =
  | { readonly kind: "session-finished"; readonly session: WorkstationSessionRunProjection }
  | { readonly kind: "session-failed"; readonly code: string; readonly message: string };

export interface SessionArtifactReadRequest {
  readonly mission_id: string;
  readonly session_id: string;
  readonly artifact_ref: string;
}

export interface SessionArtifactProjection {
  readonly schema_version: 1;
  readonly mission_id: string;
  readonly session_id: string;
  readonly artifact_id: string;
  readonly label: string;
  readonly media_type: string;
  readonly content: string;
  readonly byte_count: number;
  readonly content_limit_bytes: number;
  readonly truncated: boolean;
}

export type SessionArtifactReadResult =
  | { readonly kind: "session-artifact"; readonly artifact: SessionArtifactProjection }
  | {
      readonly kind: "session-artifact-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export type WorkstationActionResult =
  | {
      readonly kind: "acknowledged";
      readonly acknowledgement: WorkstationActionAcknowledgement;
    }
  | {
      readonly kind: "stale" | "rejected";
      readonly code: string;
      readonly message: string;
      readonly current_revision?: number;
    };

export interface MissionDraftIncludedWork {
  readonly work_id: string;
  readonly source: string;
  readonly status: WorkspaceQueueItemStatus;
  readonly acceptance_criteria: readonly string[];
  readonly allowed_paths: readonly string[];
  readonly originating_message_id: string;
}

export type MissionDraftStatus = "draft" | "confirmed" | "abandoned";

export interface MissionDraft {
  readonly draft_id: string;
  readonly mission_id: string;
  readonly status: MissionDraftStatus;
  readonly proposed_goal: string;
  readonly included_ad_hoc_work: readonly MissionDraftIncludedWork[];
  readonly excluded_ad_hoc_work_ids: readonly string[];
  readonly new_work_items: readonly string[];
  readonly dependencies: readonly string[];
  readonly unresolved_decisions: readonly string[];
}

export interface MissionDraftProjection {
  readonly schema_version: 1;
  readonly revision: number;
  readonly drafts: readonly MissionDraft[];
}

export type MissionDraftLoadResult =
  | { readonly kind: "mission-drafts"; readonly projection: MissionDraftProjection }
  | {
      readonly kind: "mission-drafts-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export type MissionDraftDecision = "confirm" | "abandon";

export interface MissionDraftCreateRequest {
  readonly correlation_id: string;
  readonly expected_revision: number;
  readonly proposed_goal: string;
  readonly selected_ad_hoc_ids: readonly string[];
  readonly excluded_ad_hoc_ids: readonly string[];
  readonly new_work_items: readonly string[];
  readonly dependencies: readonly string[];
  readonly unresolved_decisions: readonly string[];
  readonly mission_id?: string;
}

export interface MissionDraftDecisionRequest {
  readonly correlation_id: string;
  readonly expected_revision: number;
  readonly draft_id: string;
  readonly decision: MissionDraftDecision;
  readonly reason: string;
}

export interface MissionDraftAcknowledgement {
  readonly correlation_id: string;
  readonly outcome: "acknowledged";
  readonly revision: number;
  readonly draft_id: string;
  readonly draft_status: MissionDraftStatus;
  readonly effect_summary: string;
  readonly accepted_issue_id: string;
}

export type MissionDraftDecisionResult =
  | {
      readonly kind: "acknowledged";
      readonly acknowledgement: MissionDraftAcknowledgement;
    }
  | {
      readonly kind: "stale" | "rejected";
      readonly code: string;
      readonly message: string;
      readonly current_revision?: number;
    };

export type MissionDraftCreateResult = MissionDraftDecisionResult;

export interface WorkspaceSnapshot {
  readonly schema_version: 1;
  readonly revision: number;
  readonly workspace_session: {
    readonly id: string;
    readonly workspace_path: string;
    readonly status: "ready" | "empty";
  };
  readonly active_mission: {
    readonly id: string;
    readonly title: string;
    readonly issue_count: number;
  } | null;
  readonly conversation_scope: ConversationScope;
  readonly operations_view: string;
  readonly mission_board: {
    readonly prd_title: string;
    readonly issue_count: number;
    readonly ordered_issue_ids: readonly string[];
    readonly ready_issue_ids: readonly string[];
    readonly approved_issue_ids: readonly string[];
    readonly issue_slices?: readonly WorkspaceIssueSliceSummary[];
  };
  readonly missions?: readonly WorkspaceMissionSummary[];
}

export interface WorkspaceIssueBlockerSummary {
  readonly issue_id: string;
  readonly title: string;
  readonly lifecycle: string;
  readonly satisfied: boolean;
}

export interface WorkspaceIssueBoundary {
  readonly what_to_build: string;
  readonly acceptance_criteria: readonly string[];
  readonly evidence_requirements: readonly string[];
  readonly source_path: string;
}

export interface WorkspaceIssueSessionDetail {
  readonly session_id: string;
  readonly assigned_agent: string;
  readonly role: string;
  readonly provider: string;
  readonly model: string;
  readonly status: string;
  readonly stale: boolean;
  readonly disconnected: boolean;
  readonly operation_status: string;
  readonly failure: string;
}

export interface WorkspaceIssueEvidenceSummary {
  readonly state: string;
  readonly changed_files: readonly string[];
  readonly commands_run: readonly string[];
  readonly test_results: string;
  readonly risks: string;
  readonly artifact_links: readonly string[];
}

export interface WorkspaceIssueContextSourceSummary {
  readonly source_id: string;
  readonly kind: WorkingContextSourceKind;
  readonly label: string;
}

export interface WorkspaceIssueProvenance {
  readonly role: string;
  readonly provider: string;
  readonly model: string;
}

export interface WorkspaceModelAssignment {
  readonly agent_id: string;
  readonly role: string;
  readonly provider: string;
  readonly model: string;
  readonly availability: string;
  readonly availability_reason: string;
  readonly operation_status: string;
  readonly failure: string;
}

export interface WorkspaceIssueSliceSummary {
  readonly issue_id: string;
  readonly title: string;
  readonly work_type?: string;
  readonly tracker_status?: string;
  readonly lifecycle: string;
  readonly progress: string;
  readonly launch_eligible: boolean;
  readonly blockers: readonly WorkspaceIssueBlockerSummary[];
  readonly accepted_boundary: WorkspaceIssueBoundary;
  readonly sessions: readonly WorkspaceIssueSessionDetail[];
  readonly provenance: WorkspaceIssueProvenance;
  readonly model_assignment: WorkspaceModelAssignment;
  readonly evidence: WorkspaceIssueEvidenceSummary;
  readonly working_context_sources: readonly WorkspaceIssueContextSourceSummary[];
}

export interface MissionSessionSummary {
  readonly session_id: string;
  readonly issue_id: string;
  readonly assigned_agent: string;
  readonly status: string;
  readonly last_activity_at?: string;
  readonly runner_started_at?: string;
  readonly role?: string;
  readonly provider?: string;
  readonly model?: string;
  readonly task_title?: string;
  readonly operation_status?: string;
  readonly failure?: string;
  readonly changed_files?: readonly string[];
  readonly commands_run?: readonly string[];
  readonly test_results?: string;
  readonly risks?: string;
  readonly artifact_links?: readonly string[];
  readonly launch_correlation_id?: string;
  readonly evidence_correlation_id?: string;
  readonly review_correlation_id?: string;
  readonly review_outcome?: string;
  readonly review_next_action?: string;
  readonly repair_action_available?: boolean;
}

export interface WorkspaceQueueAttention {
  readonly attention_id: string;
  readonly mission_id: string;
  readonly kind:
    | "delegation-approval"
    | "clarification"
    | "issue-change-proposal"
    | "frontier-confirmation"
    | "ad-hoc-delegation";
  readonly label: string;
  readonly queue_link: string;
}

export interface WorkspaceMissionSummary {
  readonly id: string;
  readonly title: string;
  readonly issue_count: number;
  readonly is_active: boolean;
  readonly sessions: readonly MissionSessionSummary[];
  readonly attention: readonly WorkspaceQueueAttention[];
}

export interface WorkspaceEvent {
  readonly event_id: string;
  readonly correlation_id: string;
  readonly revision: number;
  readonly kind: "workspace-preferences-updated";
  readonly active_mission_id: string;
  readonly conversation_scope: ConversationScope;
  readonly operations_view: string;
}

export interface WorkspaceUpdateBatch {
  readonly after_revision: number;
  readonly current_revision: number;
  readonly events: readonly WorkspaceEvent[];
}

export interface WorkspaceActionRequest {
  readonly correlation_id: string;
  readonly expected_revision: number;
  readonly operations_view: string;
}

export interface WorkspaceScopeRequest {
  readonly correlation_id: string;
  readonly action_type: "conversation-scope-change";
  readonly actor: "mission-commander";
  readonly expected_revision: number;
  readonly target: {
    readonly kind: "conversation-scope";
    readonly id: string;
  };
  readonly scope_kind: ConversationScopeKind;
  readonly scope_target: string;
  readonly scope_label: string;
}

export interface WorkspaceMissionSwitchRequest {
  readonly correlation_id: string;
  readonly expected_revision: number;
  readonly active_mission_id: string;
}

export interface WorkspaceActionAcknowledgement {
  readonly correlation_id: string;
  readonly outcome: "acknowledged";
  readonly revision: number;
}

export type WorkspaceUpdatesResult =
  | { readonly kind: "updates"; readonly batch: WorkspaceUpdateBatch }
  | {
      readonly kind: "sync-failure";
      readonly code: string;
      readonly message: string;
      readonly recoverable: boolean;
    };

export type WorkspaceActionResult =
  | {
      readonly kind: "acknowledged";
      readonly acknowledgement: WorkspaceActionAcknowledgement;
    }
  | {
      readonly kind: "stale" | "rejected";
      readonly code: string;
      readonly message: string;
      readonly current_revision?: number;
    };

export type WorkspaceLoadResult =
  | { readonly kind: "ready"; readonly snapshot: WorkspaceSnapshot }
  | { readonly kind: "empty"; readonly snapshot: WorkspaceSnapshot }
  | {
      readonly kind: "startup-failure" | "persistence-read-failure" | "contract-failure";
      readonly message: string;
      readonly recoverable: boolean;
    };
