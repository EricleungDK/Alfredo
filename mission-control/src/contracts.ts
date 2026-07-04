export type ConversationScopeKind = "working-directory" | "mission" | "issue-slice";

export interface ConversationScope {
  readonly kind: ConversationScopeKind;
  readonly target_id: string;
  readonly label: string;
  readonly mission_id?: string | null;
}

export interface AlfredoLaunchContext {
  readonly selected_agent: string;
  readonly selected_model: string;
  readonly selected_workspace: string;
  readonly runtime_root: string;
  readonly recent_workspaces: readonly string[];
}

export type AlfredoLaunchContextResult =
  | { readonly kind: "launch-context"; readonly context: AlfredoLaunchContext }
  | {
      readonly kind: "launch-context-failure";
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
}

export interface ShellTerminalProjection {
  readonly schema_version: 1;
  readonly revision: number;
  readonly commands: readonly ShellTerminalCommandRecord[];
  readonly grants: readonly AdditionalPathGrant[];
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
  readonly path: string;
  readonly access_level: PathAccessLevel;
  readonly duration_seconds: number;
  readonly requester: "mission-commander";
}

export type AdditionalPathGrantCreateResult =
  | { readonly kind: "path-grant"; readonly grant: AdditionalPathGrant }
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
}

export type AgentConsoleMessageResult =
  | { readonly kind: "message"; readonly message: AgentConsoleMessage }
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

export interface ReviewDecisionRequest {
  readonly correlation_id: string;
  readonly expected_revision: number;
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
  readonly expected_revision: number;
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
  readonly role?: string;
  readonly provider?: string;
  readonly model?: string;
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
  readonly expected_revision: number;
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
