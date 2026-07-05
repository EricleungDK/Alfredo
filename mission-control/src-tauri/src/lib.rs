use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, OnceLock,
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BridgeConfig {
    pub python: String,
    pub backend_root: PathBuf,
    pub target_repo: PathBuf,
    pub tracker_dir: PathBuf,
    pub issues_dir: Option<PathBuf>,
    pub runtime_root: PathBuf,
    pub mission_id: String,
    pub agent_config: Option<PathBuf>,
    pub mission_catalog: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AlfredoLaunchContext {
    pub selected_agent: String,
    pub selected_model: String,
    pub selected_workspace: String,
    pub runtime_root: String,
    pub recent_workspaces: Vec<String>,
}

impl BridgeConfig {
    pub fn for_repository(backend_root: PathBuf) -> Self {
        Self {
            python: std::env::var("ALBERT_PYTHON").unwrap_or_else(|_| {
                if cfg!(windows) {
                    "python".to_owned()
                } else {
                    "python3".to_owned()
                }
            }),
            target_repo: backend_root.clone(),
            tracker_dir: backend_root.join(".scratch/albert-mission-control-app"),
            issues_dir: Some(backend_root.join(".agent/issues")),
            runtime_root: std::env::temp_dir().join("albert-runtime"),
            mission_id: "albert-mission-control-app".to_owned(),
            agent_config: Some(backend_root.join(".albert/agents.json")),
            mission_catalog: None,
            backend_root,
        }
    }

    pub fn from_environment() -> Self {
        let backend_root = std::env::var_os("ALBERT_BACKEND_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../..")
                    .canonicalize()
                    .expect("Albert backend root must be available")
            });
        let mut config = Self::for_repository(backend_root);
        if let Some(selected_workspace) = std::env::var_os("ALFREDO_SELECTED_WORKSPACE") {
            config.target_repo = PathBuf::from(selected_workspace);
        }
        if let Some(runtime_root) = std::env::var_os("ALFREDO_RUNTIME_ROOT") {
            config.runtime_root = PathBuf::from(runtime_root);
        }
        config.mission_catalog = std::env::var_os("ALBERT_MISSION_CATALOG").map(PathBuf::from);
        config
    }
}

fn recent_workspaces(runtime_root: &PathBuf) -> Vec<String> {
    let path = runtime_root.join("recent-workspaces.json");
    let Ok(contents) = fs::read_to_string(path) else {
        return vec![];
    };
    serde_json::from_str::<Vec<String>>(&contents).unwrap_or_default()
}

pub fn build_launch_context(config: &BridgeConfig) -> AlfredoLaunchContext {
    AlfredoLaunchContext {
        selected_agent: std::env::var("ALFREDO_SELECTED_AGENT").unwrap_or_default(),
        selected_model: std::env::var("ALFREDO_SELECTED_MODEL").unwrap_or_default(),
        selected_workspace: config.target_repo.to_string_lossy().into_owned(),
        runtime_root: config.runtime_root.to_string_lossy().into_owned(),
        recent_workspaces: recent_workspaces(&config.runtime_root),
    }
}

#[derive(Debug, Deserialize)]
struct PersistentResponse {
    id: String,
    success: bool,
    stdout: String,
    stderr: String,
}

struct BackendProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl BackendProcess {
    fn start(config: &BridgeConfig) -> io::Result<Self> {
        let mut child = Command::new(&config.python)
            .current_dir(&config.backend_root)
            .arg("-m")
            .arg("albert_mvp.server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("Albert backend stdin was unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Albert backend stdout was unavailable"))?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    fn request(&mut self, id: &str, argv: &[String]) -> io::Result<ProcessOutput> {
        serde_json::to_writer(
            &mut self.stdin,
            &serde_json::json!({"id": id, "argv": argv}),
        )
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        let mut line = String::new();
        if self.stdout.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Albert backend closed its response stream",
            ));
        }
        let response: PersistentResponse = serde_json::from_str(&line)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if response.id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Albert backend returned a mismatched correlation identifier",
            ));
        }
        Ok(ProcessOutput {
            success: response.success,
            stdout: response.stdout,
            stderr: response.stderr,
        })
    }
}

struct BackendCommand<'a> {
    config: &'a BridgeConfig,
    argv: Vec<String>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static BACKENDS: OnceLock<Mutex<HashMap<BridgeConfig, BackendProcess>>> = OnceLock::new();

impl BackendCommand<'_> {
    fn arg(&mut self, value: impl AsRef<OsStr>) -> &mut Self {
        self.argv
            .push(value.as_ref().to_string_lossy().into_owned());
        self
    }

    fn output(&mut self) -> io::Result<ProcessOutput> {
        let request_id = format!("desktop-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let key = self.config.clone();
        let mut backends = BACKENDS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .map_err(|_| io::Error::other("Albert backend supervisor lock was poisoned"))?;

        for attempt in 0..2 {
            if !backends.contains_key(&key) {
                backends.insert(key.clone(), BackendProcess::start(self.config)?);
            }
            let result = backends
                .get_mut(&key)
                .expect("backend inserted")
                .request(&request_id, &self.argv);
            match result {
                Ok(output) => return Ok(output),
                Err(_) if attempt == 0 => {
                    if let Some(mut backend) = backends.remove(&key) {
                        let _ = backend.child.kill();
                    }
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }
}

#[cfg(feature = "desktop")]
fn shutdown_backends() {
    let Some(backends) = BACKENDS.get() else {
        return;
    };
    let Ok(mut backends) = backends.lock() else {
        return;
    };
    for (_, mut backend) in backends.drain() {
        let _ = backend.child.kill();
        let _ = backend.child.wait();
    }
}

#[derive(Debug)]
pub struct ProcessOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceSnapshot {
    pub schema_version: u32,
    pub revision: u64,
    pub workspace_session: WorkspaceSession,
    pub active_mission: Option<MissionSummary>,
    pub conversation_scope: ConversationScope,
    pub operations_view: String,
    pub mission_board: MissionBoard,
    #[serde(default)]
    pub missions: Vec<WorkspaceMissionSummary>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceSession {
    pub id: String,
    pub workspace_path: String,
    pub status: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MissionSummary {
    pub id: String,
    pub title: String,
    pub issue_count: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ConversationScope {
    pub kind: String,
    pub target_id: String,
    pub label: String,
    #[serde(default)]
    pub mission_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MissionSessionSummary {
    pub session_id: String,
    pub issue_id: String,
    pub assigned_agent: String,
    pub status: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceQueueAttention {
    pub attention_id: String,
    pub mission_id: String,
    pub kind: String,
    pub label: String,
    pub queue_link: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceMissionSummary {
    pub id: String,
    pub title: String,
    pub issue_count: usize,
    pub is_active: bool,
    pub sessions: Vec<MissionSessionSummary>,
    pub attention: Vec<WorkspaceQueueAttention>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MissionBoard {
    pub prd_title: String,
    pub issue_count: usize,
    pub ordered_issue_ids: Vec<String>,
    pub ready_issue_ids: Vec<String>,
    pub approved_issue_ids: Vec<String>,
    #[serde(default)]
    pub issue_slices: Vec<WorkspaceIssueSliceSummary>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceIssueBlockerSummary {
    pub issue_id: String,
    pub title: String,
    pub lifecycle: String,
    pub satisfied: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceIssueBoundary {
    pub what_to_build: String,
    pub acceptance_criteria: Vec<String>,
    pub evidence_requirements: Vec<String>,
    pub source_path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceIssueSessionDetail {
    pub session_id: String,
    pub assigned_agent: String,
    pub role: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub stale: bool,
    pub disconnected: bool,
    pub operation_status: String,
    pub failure: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceIssueEvidenceSummary {
    pub state: String,
    pub changed_files: Vec<String>,
    pub commands_run: Vec<String>,
    pub test_results: String,
    pub risks: String,
    pub artifact_links: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceIssueContextSourceSummary {
    pub source_id: String,
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceIssueProvenance {
    pub role: String,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceModelAssignment {
    pub agent_id: String,
    pub role: String,
    pub provider: String,
    pub model: String,
    pub availability: String,
    pub availability_reason: String,
    pub operation_status: String,
    pub failure: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceIssueSliceSummary {
    pub issue_id: String,
    pub title: String,
    pub lifecycle: String,
    pub progress: String,
    pub launch_eligible: bool,
    pub blockers: Vec<WorkspaceIssueBlockerSummary>,
    pub accepted_boundary: WorkspaceIssueBoundary,
    pub sessions: Vec<WorkspaceIssueSessionDetail>,
    pub provenance: WorkspaceIssueProvenance,
    pub model_assignment: WorkspaceModelAssignment,
    pub evidence: WorkspaceIssueEvidenceSummary,
    pub working_context_sources: Vec<WorkspaceIssueContextSourceSummary>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceEvent {
    pub event_id: String,
    pub correlation_id: String,
    pub revision: u64,
    pub kind: String,
    pub active_mission_id: String,
    pub conversation_scope: ConversationScope,
    pub operations_view: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceUpdateBatch {
    pub after_revision: u64,
    pub current_revision: u64,
    pub events: Vec<WorkspaceEvent>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceActionRequest {
    pub correlation_id: String,
    pub expected_revision: u64,
    pub operations_view: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceActionAcknowledgement {
    pub correlation_id: String,
    pub outcome: String,
    pub revision: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceScopeRequest {
    pub correlation_id: String,
    pub expected_revision: u64,
    pub scope_kind: String,
    pub scope_target: String,
    pub scope_label: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceMissionSwitchRequest {
    pub correlation_id: String,
    pub expected_revision: u64,
    pub active_mission_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AgentConsoleMessageRequest {
    pub role: String,
    pub content: String,
    pub outcome: String,
    pub source: String,
    pub expected_revision: u64,
    pub scope_kind: String,
    pub scope_target: String,
    pub scope_label: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AgentConsoleMessage {
    pub message_id: String,
    pub sequence: u64,
    pub role: String,
    pub content: String,
    pub scope: ConversationScope,
    pub outcome: String,
    pub source: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AgentConsoleHistory {
    pub schema_version: u32,
    pub messages: Vec<AgentConsoleMessage>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkingContextSource {
    pub source_id: String,
    pub kind: String,
    pub label: String,
    pub content: String,
    pub governed: bool,
    pub eligible: bool,
    pub disposition: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkingContextProjection {
    pub schema_version: u32,
    pub revision: u64,
    pub scope: ConversationScope,
    pub sources: Vec<WorkingContextSource>,
    pub content_character_count: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkingContextCurationRequest {
    pub source_id: String,
    pub disposition: String,
    pub expected_context_revision: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkingContextAcknowledgement {
    pub outcome: String,
    pub revision: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewWorkspaceEvidence {
    pub changed_files: Vec<String>,
    pub diff_summary: String,
    pub commands_run: Vec<String>,
    pub test_results: String,
    pub risks: String,
    pub proposed_context_updates: String,
    pub artifact_links: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewWorkspaceVisibilityLimitation {
    pub path: String,
    pub classification: String,
    pub consequence: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewWorkspaceItem {
    pub mission_id: String,
    pub issue_id: String,
    pub issue_title: String,
    pub session_id: String,
    pub assigned_agent: String,
    pub status: String,
    pub lifecycle: String,
    pub evidence_complete: bool,
    pub missing_evidence: Vec<String>,
    pub can_accept: bool,
    pub evidence: ReviewWorkspaceEvidence,
    pub visibility_limitations: Vec<ReviewWorkspaceVisibilityLimitation>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewWorkspaceProjection {
    pub schema_version: u32,
    pub revision: u64,
    pub mission_id: String,
    pub items: Vec<ReviewWorkspaceItem>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewDecisionRequest {
    pub correlation_id: String,
    pub expected_revision: u64,
    pub session_id: String,
    pub decision: String,
    pub reason: String,
    #[serde(default)]
    pub failure_type: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewDecisionAcknowledgement {
    pub correlation_id: String,
    pub outcome: String,
    pub revision: u64,
    pub issue_id: String,
    pub session_id: String,
    pub review_outcome: String,
    pub next_action: String,
    pub issue_lifecycle: String,
    pub effect_summary: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ActivityAffectedEntity {
    pub entity_type: String,
    pub entity_id: String,
    pub label: String,
    pub href: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ActivityJournalEntry {
    pub entry_id: String,
    pub sequence: u64,
    pub recorded_at: String,
    pub actor: String,
    pub action_type: String,
    pub summary: String,
    pub affected_entities: Vec<ActivityAffectedEntity>,
    pub evidence_links: Vec<String>,
    pub correlation_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ActivityJournalProjection {
    pub schema_version: u32,
    pub revision: u64,
    pub entries: Vec<ActivityJournalEntry>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ActivityJournalFilters {
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub mission_id: String,
    #[serde(default)]
    pub actor: String,
    #[serde(default)]
    pub action_type: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub ended_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ShellTerminalCommandRecord {
    pub command_id: String,
    pub correlation_id: String,
    pub command: String,
    pub classification: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub working_directory: String,
    pub requested_paths: Vec<String>,
    pub access_level: String,
    pub requester: String,
    pub approver: String,
    pub decider: String,
    pub reason: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AdditionalPathGrant {
    pub grant_id: String,
    pub correlation_id: String,
    pub path: String,
    pub access_level: String,
    pub duration_seconds: u64,
    pub granted_by: String,
    pub granted_at: String,
    pub expires_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ShellTerminalProjection {
    pub schema_version: u32,
    pub revision: u64,
    pub commands: Vec<ShellTerminalCommandRecord>,
    pub grants: Vec<AdditionalPathGrant>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ShellTerminalCommandRequest {
    pub correlation_id: String,
    pub command: String,
    pub working_directory: String,
    pub requested_paths: Vec<String>,
    pub requester: String,
    pub access_level: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ShellTerminalCommandResult {
    pub command_id: String,
    pub correlation_id: String,
    pub classification: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ShellTerminalDecisionRequest {
    pub command_id: String,
    pub decision: String,
    pub actor: String,
    pub reason: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AdditionalPathGrantRequest {
    pub correlation_id: String,
    pub expected_revision: u64,
    pub path: String,
    pub access_level: String,
    pub duration_seconds: u64,
    pub requester: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceQueueItem {
    pub item_id: String,
    pub mission_id: String,
    pub item_type: String,
    pub status: String,
    pub source: String,
    pub requested_action: String,
    pub affected_boundary: String,
    pub consequence: String,
    pub issue_id: String,
    pub proposed_changes: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceQueueGroup {
    pub group_id: String,
    pub item_type: String,
    pub mission_id: String,
    pub item_count: usize,
    pub items: Vec<WorkspaceQueueItem>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceQueueProjection {
    pub schema_version: u32,
    pub revision: u64,
    pub items: Vec<WorkspaceQueueItem>,
    pub groups: Vec<WorkspaceQueueGroup>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceQueueDecisionRequest {
    pub correlation_id: String,
    #[serde(default)]
    pub action_type: String,
    #[serde(default)]
    pub actor: String,
    pub expected_revision: u64,
    #[serde(default)]
    pub target: Option<WorkspaceQueueDecisionTarget>,
    pub item_id: String,
    pub decision: String,
    pub reason: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceQueueDecisionTarget {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AdHocDelegationProposalRequest {
    pub correlation_id: String,
    pub expected_revision: u64,
    pub source: String,
    pub scope_kind: String,
    pub scope_target: String,
    pub scope_label: String,
    pub acceptance_criteria: Vec<String>,
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub command_policy: std::collections::BTreeMap<String, String>,
    pub proposed_agent: String,
    pub originating_message_id: String,
    #[serde(default)]
    pub mission_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceQueueAcknowledgement {
    pub correlation_id: String,
    pub outcome: String,
    pub revision: u64,
    pub item_id: String,
    pub item_status: String,
    pub effect_summary: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MissionDraftIncludedWork {
    pub work_id: String,
    pub source: String,
    pub status: String,
    pub acceptance_criteria: Vec<String>,
    pub allowed_paths: Vec<String>,
    pub originating_message_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MissionDraft {
    pub draft_id: String,
    pub mission_id: String,
    pub status: String,
    pub proposed_goal: String,
    pub included_ad_hoc_work: Vec<MissionDraftIncludedWork>,
    pub excluded_ad_hoc_work_ids: Vec<String>,
    pub new_work_items: Vec<String>,
    pub dependencies: Vec<String>,
    pub unresolved_decisions: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MissionDraftProjection {
    pub schema_version: u32,
    pub revision: u64,
    pub drafts: Vec<MissionDraft>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MissionDraftCreateRequest {
    pub correlation_id: String,
    pub expected_revision: u64,
    pub proposed_goal: String,
    pub selected_ad_hoc_ids: Vec<String>,
    pub excluded_ad_hoc_ids: Vec<String>,
    pub new_work_items: Vec<String>,
    pub dependencies: Vec<String>,
    pub unresolved_decisions: Vec<String>,
    #[serde(default)]
    pub mission_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MissionDraftDecisionRequest {
    pub correlation_id: String,
    pub expected_revision: u64,
    pub draft_id: String,
    pub decision: String,
    pub reason: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MissionDraftAcknowledgement {
    pub correlation_id: String,
    pub outcome: String,
    pub revision: u64,
    pub draft_id: String,
    pub draft_status: String,
    pub effect_summary: String,
    pub accepted_issue_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BridgeFailure {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}

#[derive(Debug, Deserialize)]
struct PythonFailureEnvelope {
    error: BridgeFailure,
}

pub fn decode_snapshot_output(output: ProcessOutput) -> Result<WorkspaceSnapshot, BridgeFailure> {
    if !output.success {
        if let Ok(envelope) = serde_json::from_str::<PythonFailureEnvelope>(&output.stderr) {
            return Err(envelope.error);
        }
        return Err(BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: if output.stderr.trim().is_empty() {
                "Albert backend exited without a diagnostic message.".to_owned()
            } else {
                output.stderr.trim().to_owned()
            },
            recoverable: true,
        });
    }

    let snapshot = serde_json::from_str::<WorkspaceSnapshot>(&output.stdout).map_err(|error| {
        BridgeFailure {
            code: "contract-failure".to_owned(),
            message: format!("Albert returned an invalid canonical snapshot: {error}"),
            recoverable: true,
        }
    })?;
    if snapshot.schema_version != 1 {
        return Err(BridgeFailure {
            code: "contract-failure".to_owned(),
            message: format!(
                "Albert returned unsupported canonical snapshot schema version {}.",
                snapshot.schema_version
            ),
            recoverable: false,
        });
    }
    Ok(snapshot)
}

pub fn decode_updates_output(output: ProcessOutput) -> Result<WorkspaceUpdateBatch, BridgeFailure> {
    if !output.success {
        if let Ok(envelope) = serde_json::from_str::<PythonFailureEnvelope>(&output.stderr) {
            return Err(envelope.error);
        }
        return Err(BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: output.stderr.trim().to_owned(),
            recoverable: true,
        });
    }
    let batch = serde_json::from_str::<WorkspaceUpdateBatch>(&output.stdout).map_err(|error| {
        BridgeFailure {
            code: "contract-failure".to_owned(),
            message: format!("Albert returned an invalid workspace update batch: {error}"),
            recoverable: true,
        }
    })?;
    let revisions: Vec<u64> = batch.events.iter().map(|event| event.revision).collect();
    let expected: Vec<u64> = ((batch.after_revision + 1)..=batch.current_revision).collect();
    if revisions != expected {
        return Err(BridgeFailure {
            code: "revision-gap".to_owned(),
            message: "Albert returned a non-contiguous workspace update batch.".to_owned(),
            recoverable: true,
        });
    }
    Ok(batch)
}

pub fn execute_snapshot(config: &BridgeConfig) -> Result<WorkspaceSnapshot, BridgeFailure> {
    let output = configured_python_command(config, "workspace-snapshot")
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_snapshot_output(output)
}

fn configured_python_command<'a>(config: &'a BridgeConfig, subcommand: &str) -> BackendCommand<'a> {
    let mut command = BackendCommand {
        config,
        argv: vec![subcommand.to_owned()],
    };
    command
        .arg("--target-repo")
        .arg(&config.target_repo)
        .arg("--tracker-dir")
        .arg(&config.tracker_dir)
        .arg("--runtime-root")
        .arg(&config.runtime_root)
        .arg("--mission-id")
        .arg(&config.mission_id);
    if let Some(issues_dir) = &config.issues_dir {
        command.arg("--issues-dir").arg(issues_dir);
    }
    if let Some(agent_config) = &config.agent_config {
        command.arg("--agent-config").arg(agent_config);
    }
    if let Some(mission_catalog) = &config.mission_catalog {
        command.arg("--mission-catalog").arg(mission_catalog);
    }
    command
}

fn process_output(output: ProcessOutput) -> ProcessOutput {
    output
}

fn decode_backend_json<T: DeserializeOwned>(
    output: ProcessOutput,
    contract_name: &str,
) -> Result<T, BridgeFailure> {
    if !output.success {
        if let Ok(envelope) = serde_json::from_str::<PythonFailureEnvelope>(&output.stderr) {
            return Err(envelope.error);
        }
        return Err(BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: output.stderr.trim().to_owned(),
            recoverable: true,
        });
    }
    serde_json::from_str(&output.stdout).map_err(|error| BridgeFailure {
        code: "contract-failure".to_owned(),
        message: format!("Albert returned an invalid {contract_name}: {error}"),
        recoverable: true,
    })
}

pub fn execute_action(
    config: &BridgeConfig,
    action: &WorkspaceActionRequest,
) -> Result<WorkspaceActionAcknowledgement, BridgeFailure> {
    let output = configured_python_command(config, "workspace-action")
        .arg("--correlation-id")
        .arg(&action.correlation_id)
        .arg("--expected-revision")
        .arg(action.expected_revision.to_string())
        .arg("--operations-view")
        .arg(&action.operations_view)
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    let output = process_output(output);
    if !output.success {
        if let Ok(envelope) = serde_json::from_str::<PythonFailureEnvelope>(&output.stderr) {
            return Err(envelope.error);
        }
        return Err(BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: output.stderr.trim().to_owned(),
            recoverable: true,
        });
    }
    serde_json::from_str(&output.stdout).map_err(|error| BridgeFailure {
        code: "contract-failure".to_owned(),
        message: format!("Albert returned an invalid action acknowledgement: {error}"),
        recoverable: true,
    })
}

pub fn execute_updates(
    config: &BridgeConfig,
    after_revision: u64,
) -> Result<WorkspaceUpdateBatch, BridgeFailure> {
    let output = configured_python_command(config, "workspace-updates")
        .arg("--after-revision")
        .arg(after_revision.to_string())
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_updates_output(process_output(output))
}

pub fn execute_scope(
    config: &BridgeConfig,
    scope: &WorkspaceScopeRequest,
) -> Result<WorkspaceActionAcknowledgement, BridgeFailure> {
    let output = configured_python_command(config, "workspace-scope")
        .arg("--correlation-id")
        .arg(&scope.correlation_id)
        .arg("--expected-revision")
        .arg(scope.expected_revision.to_string())
        .arg("--scope-kind")
        .arg(&scope.scope_kind)
        .arg("--scope-target")
        .arg(&scope.scope_target)
        .arg("--scope-label")
        .arg(&scope.scope_label)
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "scope acknowledgement")
}

pub fn execute_mission_switch(
    config: &BridgeConfig,
    request: &WorkspaceMissionSwitchRequest,
) -> Result<WorkspaceActionAcknowledgement, BridgeFailure> {
    let output = configured_python_command(config, "workspace-mission-switch")
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--expected-revision")
        .arg(request.expected_revision.to_string())
        .arg("--active-mission-id")
        .arg(&request.active_mission_id)
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "mission switch acknowledgement")
}

pub fn execute_console_message(
    config: &BridgeConfig,
    message: &AgentConsoleMessageRequest,
) -> Result<AgentConsoleMessage, BridgeFailure> {
    let output = configured_python_command(config, "agent-console-message")
        .arg("--role")
        .arg(&message.role)
        .arg("--content")
        .arg(&message.content)
        .arg("--outcome")
        .arg(&message.outcome)
        .arg("--source")
        .arg(&message.source)
        .arg("--expected-revision")
        .arg(message.expected_revision.to_string())
        .arg("--scope-kind")
        .arg(&message.scope_kind)
        .arg("--scope-target")
        .arg(&message.scope_target)
        .arg("--scope-label")
        .arg(&message.scope_label)
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Agent Console message")
}

pub fn execute_console_history(
    config: &BridgeConfig,
) -> Result<AgentConsoleHistory, BridgeFailure> {
    let output = configured_python_command(config, "agent-console-history")
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Agent Console history")
}

pub fn execute_working_context(
    config: &BridgeConfig,
) -> Result<WorkingContextProjection, BridgeFailure> {
    let output = configured_python_command(config, "working-context")
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Working Context projection")
}

pub fn execute_working_context_curate(
    config: &BridgeConfig,
    request: &WorkingContextCurationRequest,
) -> Result<WorkingContextAcknowledgement, BridgeFailure> {
    let output = configured_python_command(config, "working-context-curate")
        .arg("--source-id")
        .arg(&request.source_id)
        .arg("--disposition")
        .arg(&request.disposition)
        .arg("--expected-context-revision")
        .arg(request.expected_context_revision.to_string())
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Working Context acknowledgement")
}

pub fn execute_review_workspace(
    config: &BridgeConfig,
) -> Result<ReviewWorkspaceProjection, BridgeFailure> {
    let output = configured_python_command(config, "review-workspace")
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Review Workspace projection")
}

pub fn execute_review_decision(
    config: &BridgeConfig,
    request: &ReviewDecisionRequest,
) -> Result<ReviewDecisionAcknowledgement, BridgeFailure> {
    let mut command = configured_python_command(config, "review-decision");
    command
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--expected-revision")
        .arg(request.expected_revision.to_string())
        .arg("--session-id")
        .arg(&request.session_id)
        .arg("--decision")
        .arg(&request.decision)
        .arg("--reason")
        .arg(&request.reason);
    if let Some(failure_type) = &request.failure_type {
        if !failure_type.is_empty() {
            command.arg("--failure-type").arg(failure_type);
        }
    }
    let output = command.output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Albert backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Review decision acknowledgement")
}

pub fn execute_activity_journal(
    config: &BridgeConfig,
    filters: &ActivityJournalFilters,
) -> Result<ActivityJournalProjection, BridgeFailure> {
    let mut command = configured_python_command(config, "activity-journal");
    if !filters.search.is_empty() {
        command.arg("--search").arg(&filters.search);
    }
    if !filters.mission_id.is_empty() {
        command
            .arg("--activity-mission-id")
            .arg(&filters.mission_id);
    }
    if !filters.actor.is_empty() {
        command.arg("--actor").arg(&filters.actor);
    }
    if !filters.action_type.is_empty() {
        command.arg("--action-type").arg(&filters.action_type);
    }
    if !filters.started_at.is_empty() {
        command.arg("--started-at").arg(&filters.started_at);
    }
    if !filters.ended_at.is_empty() {
        command.arg("--ended-at").arg(&filters.ended_at);
    }
    let output = command.output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Albert backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Activity Journal projection")
}

pub fn execute_shell_terminal(
    config: &BridgeConfig,
) -> Result<ShellTerminalProjection, BridgeFailure> {
    let output = configured_python_command(config, "shell-terminal")
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Shell Terminal projection")
}

pub fn execute_shell_terminal_submit(
    config: &BridgeConfig,
    request: &ShellTerminalCommandRequest,
) -> Result<ShellTerminalCommandResult, BridgeFailure> {
    let mut command = configured_python_command(config, "shell-terminal-submit");
    command
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--command-text")
        .arg(&request.command)
        .arg("--working-directory")
        .arg(&request.working_directory)
        .arg("--requester")
        .arg(&request.requester)
        .arg("--access-level")
        .arg(&request.access_level);
    for path in &request.requested_paths {
        command.arg("--requested-path").arg(path);
    }
    let output = command.output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Albert backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Shell Terminal command result")
}

pub fn execute_shell_terminal_decision(
    config: &BridgeConfig,
    request: &ShellTerminalDecisionRequest,
) -> Result<ShellTerminalCommandResult, BridgeFailure> {
    let output = configured_python_command(config, "shell-terminal-decision")
        .arg("--command-id")
        .arg(&request.command_id)
        .arg("--decision")
        .arg(&request.decision)
        .arg("--actor")
        .arg(&request.actor)
        .arg("--reason")
        .arg(&request.reason)
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Shell Terminal decision result")
}

pub fn execute_additional_path_grant_create(
    config: &BridgeConfig,
    request: &AdditionalPathGrantRequest,
) -> Result<AdditionalPathGrant, BridgeFailure> {
    let output = configured_python_command(config, "additional-path-grant-create")
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--expected-terminal-revision")
        .arg(request.expected_revision.to_string())
        .arg("--path")
        .arg(&request.path)
        .arg("--access-level")
        .arg(&request.access_level)
        .arg("--duration-seconds")
        .arg(request.duration_seconds.to_string())
        .arg("--requester")
        .arg(&request.requester)
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Additional Path Grant result")
}

pub fn execute_workspace_queue(
    config: &BridgeConfig,
) -> Result<WorkspaceQueueProjection, BridgeFailure> {
    let output = configured_python_command(config, "workspace-queue")
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Workspace Queue projection")
}

pub fn execute_ad_hoc_delegation_proposal(
    config: &BridgeConfig,
    request: &AdHocDelegationProposalRequest,
) -> Result<WorkspaceQueueAcknowledgement, BridgeFailure> {
    let mut command = configured_python_command(config, "ad-hoc-delegation-proposal");
    command
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--expected-revision")
        .arg(request.expected_revision.to_string())
        .arg("--source")
        .arg(&request.source)
        .arg("--scope-kind")
        .arg(&request.scope_kind)
        .arg("--scope-target")
        .arg(&request.scope_target)
        .arg("--scope-label")
        .arg(&request.scope_label)
        .arg("--proposed-agent")
        .arg(&request.proposed_agent)
        .arg("--originating-message-id")
        .arg(&request.originating_message_id);
    for criterion in &request.acceptance_criteria {
        command.arg("--acceptance-criterion").arg(criterion);
    }
    for path in &request.allowed_paths {
        command.arg("--allowed-path").arg(path);
    }
    for (backend_command, policy) in &request.command_policy {
        command
            .arg("--command-policy")
            .arg(format!("{backend_command}={policy}"));
    }
    if let Some(mission_id) = &request.mission_id {
        if !mission_id.is_empty() {
            command.arg("--queue-mission-id").arg(mission_id);
        }
    }
    let output = command.output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Albert backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Ad Hoc Delegation acknowledgement")
}

pub fn execute_workspace_queue_decision(
    config: &BridgeConfig,
    request: &WorkspaceQueueDecisionRequest,
) -> Result<WorkspaceQueueAcknowledgement, BridgeFailure> {
    let mut command = configured_python_command(config, "workspace-queue-decision");
    command
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--expected-queue-revision")
        .arg(request.expected_revision.to_string())
        .arg("--item-id")
        .arg(&request.item_id)
        .arg("--decision")
        .arg(&request.decision)
        .arg("--reason")
        .arg(&request.reason);
    if !request.action_type.is_empty() {
        command.arg("--action-type").arg(&request.action_type);
    }
    if !request.actor.is_empty() {
        command.arg("--actor").arg(&request.actor);
    }
    if let Some(target) = &request.target {
        command.arg("--target-kind").arg(&target.kind);
        command.arg("--target-id").arg(&target.id);
    }
    let output = command.output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Albert backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Workspace Queue acknowledgement")
}

pub fn execute_mission_drafts(
    config: &BridgeConfig,
) -> Result<MissionDraftProjection, BridgeFailure> {
    let output = configured_python_command(config, "mission-drafts")
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Mission Draft projection")
}

pub fn execute_mission_draft_create(
    config: &BridgeConfig,
    request: &MissionDraftCreateRequest,
) -> Result<MissionDraftAcknowledgement, BridgeFailure> {
    let mut command = configured_python_command(config, "mission-draft-create");
    command
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--expected-revision")
        .arg(request.expected_revision.to_string())
        .arg("--proposed-goal")
        .arg(&request.proposed_goal);
    for work_id in &request.selected_ad_hoc_ids {
        command.arg("--selected-ad-hoc-id").arg(work_id);
    }
    for work_id in &request.excluded_ad_hoc_ids {
        command.arg("--excluded-ad-hoc-id").arg(work_id);
    }
    for item in &request.new_work_items {
        command.arg("--new-work-item").arg(item);
    }
    for dependency in &request.dependencies {
        command.arg("--dependency").arg(dependency);
    }
    for decision in &request.unresolved_decisions {
        command.arg("--unresolved-decision").arg(decision);
    }
    if let Some(mission_id) = &request.mission_id {
        if !mission_id.is_empty() {
            command.arg("--draft-mission-id").arg(mission_id);
        }
    }
    let output = command.output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Albert backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Mission Draft acknowledgement")
}

pub fn execute_mission_draft_decision(
    config: &BridgeConfig,
    request: &MissionDraftDecisionRequest,
) -> Result<MissionDraftAcknowledgement, BridgeFailure> {
    let command_name = match request.decision.as_str() {
        "confirm" => "mission-draft-confirm",
        "abandon" => "mission-draft-abandon",
        other => {
            return Err(BridgeFailure {
                code: "contract-failure".to_owned(),
                message: format!("Unknown Mission Draft decision: {other}"),
                recoverable: false,
            })
        }
    };
    let output = configured_python_command(config, command_name)
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--expected-draft-revision")
        .arg(request.expected_revision.to_string())
        .arg("--draft-id")
        .arg(&request.draft_id)
        .arg("--reason")
        .arg(&request.reason)
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Mission Draft acknowledgement")
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_snapshot(
    config: tauri::State<'_, BridgeConfig>,
) -> Result<WorkspaceSnapshot, BridgeFailure> {
    execute_snapshot(config.inner())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn alfredo_launch_context(config: tauri::State<'_, BridgeConfig>) -> AlfredoLaunchContext {
    build_launch_context(config.inner())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_updates(
    config: tauri::State<'_, BridgeConfig>,
    after_revision: u64,
) -> Result<WorkspaceUpdateBatch, BridgeFailure> {
    execute_updates(config.inner(), after_revision)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_action(
    config: tauri::State<'_, BridgeConfig>,
    action: WorkspaceActionRequest,
) -> Result<WorkspaceActionAcknowledgement, BridgeFailure> {
    execute_action(config.inner(), &action)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_scope(
    config: tauri::State<'_, BridgeConfig>,
    scope: WorkspaceScopeRequest,
) -> Result<WorkspaceActionAcknowledgement, BridgeFailure> {
    execute_scope(config.inner(), &scope)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_mission_switch(
    config: tauri::State<'_, BridgeConfig>,
    request: WorkspaceMissionSwitchRequest,
) -> Result<WorkspaceActionAcknowledgement, BridgeFailure> {
    execute_mission_switch(config.inner(), &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn agent_console_message(
    config: tauri::State<'_, BridgeConfig>,
    message: AgentConsoleMessageRequest,
) -> Result<AgentConsoleMessage, BridgeFailure> {
    execute_console_message(config.inner(), &message)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn agent_console_history(
    config: tauri::State<'_, BridgeConfig>,
) -> Result<AgentConsoleHistory, BridgeFailure> {
    execute_console_history(config.inner())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn working_context(
    config: tauri::State<'_, BridgeConfig>,
) -> Result<WorkingContextProjection, BridgeFailure> {
    execute_working_context(config.inner())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn working_context_curate(
    config: tauri::State<'_, BridgeConfig>,
    request: WorkingContextCurationRequest,
) -> Result<WorkingContextAcknowledgement, BridgeFailure> {
    execute_working_context_curate(config.inner(), &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn review_workspace(
    config: tauri::State<'_, BridgeConfig>,
) -> Result<ReviewWorkspaceProjection, BridgeFailure> {
    execute_review_workspace(config.inner())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn review_decision(
    config: tauri::State<'_, BridgeConfig>,
    request: ReviewDecisionRequest,
) -> Result<ReviewDecisionAcknowledgement, BridgeFailure> {
    execute_review_decision(config.inner(), &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn activity_journal(
    config: tauri::State<'_, BridgeConfig>,
    filters: ActivityJournalFilters,
) -> Result<ActivityJournalProjection, BridgeFailure> {
    execute_activity_journal(config.inner(), &filters)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn shell_terminal(
    config: tauri::State<'_, BridgeConfig>,
) -> Result<ShellTerminalProjection, BridgeFailure> {
    execute_shell_terminal(config.inner())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn shell_terminal_submit(
    config: tauri::State<'_, BridgeConfig>,
    request: ShellTerminalCommandRequest,
) -> Result<ShellTerminalCommandResult, BridgeFailure> {
    execute_shell_terminal_submit(config.inner(), &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn shell_terminal_decision(
    config: tauri::State<'_, BridgeConfig>,
    request: ShellTerminalDecisionRequest,
) -> Result<ShellTerminalCommandResult, BridgeFailure> {
    execute_shell_terminal_decision(config.inner(), &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn additional_path_grant_create(
    config: tauri::State<'_, BridgeConfig>,
    request: AdditionalPathGrantRequest,
) -> Result<AdditionalPathGrant, BridgeFailure> {
    execute_additional_path_grant_create(config.inner(), &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_queue(
    config: tauri::State<'_, BridgeConfig>,
) -> Result<WorkspaceQueueProjection, BridgeFailure> {
    execute_workspace_queue(config.inner())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn ad_hoc_delegation_proposal(
    config: tauri::State<'_, BridgeConfig>,
    request: AdHocDelegationProposalRequest,
) -> Result<WorkspaceQueueAcknowledgement, BridgeFailure> {
    execute_ad_hoc_delegation_proposal(config.inner(), &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_queue_decision(
    config: tauri::State<'_, BridgeConfig>,
    request: WorkspaceQueueDecisionRequest,
) -> Result<WorkspaceQueueAcknowledgement, BridgeFailure> {
    execute_workspace_queue_decision(config.inner(), &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn mission_drafts(
    config: tauri::State<'_, BridgeConfig>,
) -> Result<MissionDraftProjection, BridgeFailure> {
    execute_mission_drafts(config.inner())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn mission_draft_create(
    config: tauri::State<'_, BridgeConfig>,
    request: MissionDraftCreateRequest,
) -> Result<MissionDraftAcknowledgement, BridgeFailure> {
    execute_mission_draft_create(config.inner(), &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn mission_draft_decision(
    config: tauri::State<'_, BridgeConfig>,
    request: MissionDraftDecisionRequest,
) -> Result<MissionDraftAcknowledgement, BridgeFailure> {
    execute_mission_draft_decision(config.inner(), &request)
}

#[cfg(feature = "desktop")]
pub fn run() {
    tauri::Builder::default()
        .manage(BridgeConfig::from_environment())
        .invoke_handler(tauri::generate_handler![
            alfredo_launch_context,
            workspace_snapshot,
            workspace_updates,
            workspace_action,
            workspace_scope,
            workspace_mission_switch,
            agent_console_message,
            agent_console_history,
            working_context,
            working_context_curate,
            review_workspace,
            review_decision,
            activity_journal,
            shell_terminal,
            shell_terminal_submit,
            shell_terminal_decision,
            additional_path_grant_create,
            workspace_queue,
            ad_hoc_delegation_proposal,
            workspace_queue_decision,
            mission_drafts,
            mission_draft_create,
            mission_draft_decision
        ])
        .build(tauri::generate_context!())
        .expect("Albert Mission Control should build")
        .run(|_app_handle, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                shutdown_backends();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_a_successful_versioned_workspace_snapshot() {
        let output = ProcessOutput {
            success: true,
            stdout: r#"{
                "schema_version": 1,
                "revision": 7,
                "workspace_session": {
                    "id": "workspace-command-deck",
                    "workspace_path": "/workspace/albert",
                    "status": "ready"
                },
                "active_mission": {
                    "id": "command-deck",
                    "title": "Command Deck Mission",
                    "issue_count": 2
                },
                "conversation_scope": {
                    "kind": "mission",
                    "target_id": "command-deck",
                    "label": "Command Deck Mission"
                },
                "operations_view": "mission-board",
                "mission_board": {
                    "prd_title": "Command Deck Mission",
                    "issue_count": 2,
                    "ordered_issue_ids": ["ISS-01", "ISS-02"],
                    "ready_issue_ids": [],
                    "approved_issue_ids": ["ISS-01"],
                    "issue_slices": [
                        {
                            "issue_id": "ISS-01",
                            "title": "Restore workspace session",
                            "lifecycle": "Ready",
                            "progress": "Launch eligible",
                            "launch_eligible": true,
                            "blockers": [],
                            "accepted_boundary": {
                                "what_to_build": "Restore the canonical snapshot.",
                                "acceptance_criteria": ["Snapshot is visible."],
                                "evidence_requirements": ["Focused bridge test."],
                                "source_path": ".agent/issues/01.md"
                            },
                            "sessions": [
                                {
                                    "session_id": "session-ISS-01-1",
                                    "assigned_agent": "qwen-coder-local",
                                    "role": "local-agent",
                                    "provider": "ollama",
                                    "model": "qwen2.5-coder:14b",
                                    "status": "launched",
                                    "stale": false,
                                    "disconnected": false,
                                    "operation_status": "idle",
                                    "failure": ""
                                }
                            ],
                            "provenance": {
                                "role": "local-agent",
                                "provider": "ollama",
                                "model": "qwen2.5-coder:14b"
                            },
                            "model_assignment": {
                                "agent_id": "qwen2.5-coder-14b",
                                "role": "local-agent",
                                "provider": "ollama",
                                "model": "qwen2.5-coder:14b",
                                "availability": "available",
                                "availability_reason": "",
                                "operation_status": "idle",
                                "failure": ""
                            },
                            "evidence": {
                                "state": "missing",
                                "changed_files": [],
                                "commands_run": [],
                                "test_results": "No evidence package recorded.",
                                "risks": "None recorded.",
                                "artifact_links": []
                            },
                            "working_context_sources": [
                                {
                                    "source_id": "shared-context:command-deck:issue-slice:ISS-01",
                                    "kind": "shared-context",
                                    "label": "Shared Context - Restore workspace session"
                                }
                            ]
                        }
                    ]
                }
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let snapshot = decode_snapshot_output(output).expect("snapshot should be valid");

        assert_eq!(snapshot.schema_version, 1);
        assert_eq!(snapshot.revision, 7);
        assert_eq!(snapshot.workspace_session.id, "workspace-command-deck");
        assert_eq!(snapshot.active_mission.unwrap().id, "command-deck");
        assert_eq!(snapshot.mission_board.issue_slices[0].issue_id, "ISS-01");
        assert_eq!(
            snapshot.mission_board.issue_slices[0].sessions[0].provider,
            "ollama"
        );
        assert_eq!(
            snapshot.mission_board.issue_slices[0]
                .model_assignment
                .availability,
            "available"
        );
    }

    #[test]
    fn reports_backend_startup_failure_without_synthesizing_state() {
        let output = ProcessOutput {
            success: false,
            stdout: String::new(),
            stderr: "python3: command not found".to_owned(),
        };

        let failure = decode_snapshot_output(output).expect_err("startup must fail");

        assert_eq!(failure.code, "backend-startup-failure");
        assert!(failure.recoverable);
    }

    #[test]
    fn reports_contract_failure_for_malformed_success_output() {
        let output = ProcessOutput {
            success: true,
            stdout: "not-json".to_owned(),
            stderr: String::new(),
        };

        let failure = decode_snapshot_output(output).expect_err("malformed JSON must fail");

        assert_eq!(failure.code, "contract-failure");
        assert!(failure.message.contains("canonical snapshot"));
    }

    #[test]
    fn preserves_structured_python_persistence_failure() {
        let output = ProcessOutput {
            success: false,
            stdout: String::new(),
            stderr: r#"{"error":{"code":"persistence-read-failure","message":"corrupt preferences","recoverable":true}}"#.to_owned(),
        };

        let failure = decode_snapshot_output(output).expect_err("persistence must fail");

        assert_eq!(failure.code, "persistence-read-failure");
        assert_eq!(failure.message, "corrupt preferences");
    }

    #[test]
    fn desktop_bridge_launches_python_and_restores_the_same_workspace_session() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-bridge-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Restored Desktop Mission\n")
            .expect("PRD should be written");
        fs::write(
            issues_dir.join("01-restore.md"),
            r#"Status: ready-for-agent
Type: AFK

## Parent

PRD.md

## What to build

Restore a desktop session.

## Acceptance criteria

- [ ] Desktop state is restored.

## Blocked by

None - can start immediately
"#,
        )
        .expect("issue should be written");
        let backend_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("backend root");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root,
            target_repo,
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "desktop-restore".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };

        let first = execute_snapshot(&config).expect("first desktop snapshot");
        let warm = execute_snapshot(&config).expect("warm desktop snapshot");
        assert_eq!(first.workspace_session.id, warm.workspace_session.id);

        let key = config.clone();
        {
            let mut backends = BACKENDS
                .get()
                .expect("backend supervisor")
                .lock()
                .expect("backend supervisor lock");
            let backend = backends.get_mut(&key).expect("running backend");
            backend.child.kill().expect("backend should stop");
            backend.child.wait().expect("backend should exit");
        }
        let restored = execute_snapshot(&config).expect("restarted desktop snapshot");

        assert_eq!(first.workspace_session.id, restored.workspace_session.id);
        assert_eq!(first.active_mission.unwrap().id, "desktop-restore");
        assert_eq!(restored.operations_view, "mission-board");
        fs::remove_dir_all(root).expect("temporary bridge fixture should be removed");
    }

    #[test]
    fn repository_config_targets_the_current_command_deck_prd_and_issues() {
        let backend_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("backend root");

        let config = BridgeConfig::for_repository(backend_root.clone());

        assert_eq!(config.target_repo, backend_root);
        assert!(config
            .tracker_dir
            .ends_with(".scratch/albert-mission-control-app"));
        assert!(config.issues_dir.unwrap().ends_with(".agent/issues"));
        assert_eq!(config.mission_id, "albert-mission-control-app");
    }

    #[test]
    fn environment_config_keeps_install_root_and_selected_workspace_separate() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("alfredo-bridge-config-{unique}"));
        let install_root = root.join("install");
        let selected_workspace = root.join("workspace");
        fs::create_dir_all(&install_root).expect("install root");
        fs::create_dir_all(&selected_workspace).expect("selected workspace");
        std::env::set_var("ALBERT_BACKEND_ROOT", &install_root);
        std::env::set_var("ALFREDO_SELECTED_WORKSPACE", &selected_workspace);

        let config = BridgeConfig::from_environment();

        std::env::remove_var("ALBERT_BACKEND_ROOT");
        std::env::remove_var("ALFREDO_SELECTED_WORKSPACE");
        assert_eq!(config.backend_root, install_root);
        assert_eq!(config.target_repo, selected_workspace);
        fs::remove_dir_all(root).expect("temporary bridge config fixture should be removed");
    }

    #[test]
    fn launch_context_surfaces_agent_runtime_and_recent_workspaces() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("alfredo-launch-context-{unique}"));
        let install_root = root.join("install");
        let selected_workspace = root.join("workspace");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&install_root).expect("install root");
        fs::create_dir_all(&selected_workspace).expect("selected workspace");
        fs::create_dir_all(&runtime_root).expect("runtime root");
        fs::write(
            runtime_root.join("recent-workspaces.json"),
            format!(
                "[{}]",
                serde_json::to_string(&selected_workspace.to_string_lossy().to_string())
                    .expect("recent workspace json")
            ),
        )
        .expect("recent workspaces should be written");
        std::env::set_var("ALFREDO_SELECTED_AGENT", "qwen3.6-27b");
        std::env::set_var("ALFREDO_SELECTED_MODEL", "qwen3.6:27b");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: install_root,
            target_repo: selected_workspace.clone(),
            tracker_dir: root.join("tracker"),
            issues_dir: None,
            runtime_root: runtime_root.clone(),
            mission_id: "launch-context".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };

        let context = build_launch_context(&config);

        std::env::remove_var("ALFREDO_SELECTED_AGENT");
        std::env::remove_var("ALFREDO_SELECTED_MODEL");
        assert_eq!(context.selected_agent, "qwen3.6-27b");
        assert_eq!(context.selected_model, "qwen3.6:27b");
        assert_eq!(
            context.selected_workspace,
            selected_workspace.to_string_lossy().to_string()
        );
        assert_eq!(
            context.runtime_root,
            runtime_root.to_string_lossy().to_string()
        );
        assert_eq!(
            context.recent_workspaces,
            vec![selected_workspace.to_string_lossy().to_string()]
        );
        fs::remove_dir_all(root).expect("temporary launch context fixture should be removed");
    }

    #[test]
    fn parses_an_ordered_workspace_update_batch() {
        let output = ProcessOutput {
            success: true,
            stdout: r#"{
                "after_revision": 1,
                "current_revision": 2,
                "events": [{
                    "event_id": "workspace-2-view-activity-1",
                    "correlation_id": "view-activity-1",
                    "revision": 2,
                    "kind": "workspace-preferences-updated",
                    "active_mission_id": "command-deck",
                    "conversation_scope": {
                        "kind": "mission",
                        "target_id": "command-deck",
                        "label": "Command Deck"
                    },
                    "operations_view": "activity"
                }]
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let batch = decode_updates_output(output).expect("update batch should be valid");

        assert_eq!(batch.after_revision, 1);
        assert_eq!(batch.current_revision, 2);
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].correlation_id, "view-activity-1");
        assert_eq!(batch.events[0].operations_view, "activity");
    }

    #[test]
    fn parses_review_workspace_projection_and_decision_acknowledgement() {
        let projection_output = ProcessOutput {
            success: true,
            stdout: r#"{
                "schema_version": 1,
                "revision": 4,
                "mission_id": "command-deck",
                "items": [{
                    "mission_id": "command-deck",
                    "issue_id": "ISS-01",
                    "issue_title": "Review evidence",
                    "session_id": "session-ISS-01-1",
                    "assigned_agent": "qwen-coder-local",
                    "status": "evidence-ready",
                    "lifecycle": "Ready",
                    "evidence_complete": true,
                    "missing_evidence": [],
                    "can_accept": true,
                    "evidence": {
                        "changed_files": ["src/App.tsx"],
                        "diff_summary": "Added Review Workspace.",
                        "commands_run": ["npm test"],
                        "test_results": "Tests passed.",
                        "risks": "None.",
                        "proposed_context_updates": "Document review flow.",
                        "artifact_links": ["app-local://evidence/session-ISS-01-1"]
                    },
                    "visibility_limitations": [{
                        "path": ".env",
                        "classification": "Blocked",
                        "consequence": "Frontier Reviewer cannot inspect this path; human review may be required."
                    }]
                }]
            }"#
            .to_owned(),
            stderr: String::new(),
        };
        let acknowledgement_output = ProcessOutput {
            success: true,
            stdout: r#"{
                "correlation_id": "review-accept-4",
                "outcome": "acknowledged",
                "revision": 5,
                "issue_id": "ISS-01",
                "session_id": "session-ISS-01-1",
                "review_outcome": "Approved",
                "next_action": "prepare-pr",
                "issue_lifecycle": "Complete",
                "effect_summary": "Issue Slice becomes Complete and PR-ready; it is not marked merged."
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let projection: ReviewWorkspaceProjection =
            decode_backend_json(projection_output, "Review Workspace projection")
                .expect("projection should decode");
        let acknowledgement: ReviewDecisionAcknowledgement =
            decode_backend_json(acknowledgement_output, "Review decision acknowledgement")
                .expect("acknowledgement should decode");

        assert_eq!(
            projection.items[0].evidence.diff_summary,
            "Added Review Workspace."
        );
        assert_eq!(
            projection.items[0].visibility_limitations[0].classification,
            "Blocked"
        );
        assert_eq!(acknowledgement.next_action, "prepare-pr");
        assert_eq!(acknowledgement.issue_lifecycle, "Complete");
    }

    #[test]
    fn parses_activity_journal_projection() {
        let projection_output = ProcessOutput {
            success: true,
            stdout: r#"{
                "schema_version": 1,
                "revision": 1,
                "entries": [{
                    "entry_id": "activity-000001",
                    "sequence": 1,
                    "recorded_at": "2026-06-26T10:15:00Z",
                    "actor": "mission-commander",
                    "action_type": "review-decision",
                    "summary": "Mission Commander recorded Review Workspace decision Approved.",
                    "affected_entities": [{
                        "entity_type": "issue-slice",
                        "entity_id": "ISS-01",
                        "label": "Restore Workspace Session",
                        "href": "app-local://missions/command-deck/issues/ISS-01"
                    }, {
                        "entity_type": "evidence-package",
                        "entity_id": "session-ISS-01-1",
                        "label": "Evidence Package session-ISS-01-1",
                        "href": "app-local://evidence/session-ISS-01-1"
                    }],
                    "evidence_links": ["app-local://evidence/session-ISS-01-1"],
                    "correlation_id": "review-activity-cli-1"
                }]
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let projection: ActivityJournalProjection =
            decode_backend_json(projection_output, "Activity Journal projection")
                .expect("projection should decode");

        assert_eq!(projection.revision, 1);
        assert_eq!(projection.entries[0].action_type, "review-decision");
        assert_eq!(
            projection.entries[0].affected_entities[1].entity_type,
            "evidence-package"
        );
        assert_eq!(
            projection.entries[0].evidence_links[0],
            "app-local://evidence/session-ISS-01-1"
        );
    }

    #[test]
    fn parses_shell_terminal_projection_without_terminal_bytes() {
        let projection_output = ProcessOutput {
            success: true,
            stdout: r#"{
                "schema_version": 1,
                "revision": 2,
                "commands": [{
                    "command_id": "terminal-command-000001",
                    "correlation_id": "terminal-rust-1",
                    "command": "python3 -m unittest --help",
                    "classification": "auto-allowed",
                    "status": "completed",
                    "exit_code": 0,
                    "working_directory": "/workspace/albert",
                    "requested_paths": [],
                    "access_level": "read",
                    "requester": "mission-commander",
                    "approver": "",
                    "decider": "",
                    "reason": ""
                }],
                "grants": [{
                    "grant_id": "path-grant-000001",
                    "correlation_id": "path-grant-rust-1",
                    "path": "/external/docs",
                    "access_level": "read",
                    "duration_seconds": 900,
                    "granted_by": "mission-commander",
                    "granted_at": "2026-06-27T08:00:00Z",
                    "expires_at": "2026-06-27T08:15:00Z"
                }]
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let projection: ShellTerminalProjection =
            decode_backend_json(projection_output, "Shell Terminal projection")
                .expect("projection should decode");

        assert_eq!(projection.commands[0].status, "completed");
        assert_eq!(projection.commands[0].exit_code, Some(0));
        assert_eq!(projection.grants[0].duration_seconds, 900);
        assert_eq!(projection.grants[0].granted_by, "mission-commander");
    }

    #[test]
    fn desktop_bridge_reads_shell_terminal_metadata_from_python_backend() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-terminal-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Shell Terminal Bridge\n").expect("PRD");
        fs::write(
            issues_dir.join("01-terminal.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nTerminal bridge.\n\n## Acceptance criteria\n\n- [ ] Terminal bridge.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("issue");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo: target_repo.clone(),
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "terminal-bridge".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };
        let output = configured_python_command(&config, "shell-terminal-submit")
            .arg("--correlation-id")
            .arg("terminal-bridge-submit-1")
            .arg("--command-text")
            .arg("python3 -m unittest --help")
            .arg("--working-directory")
            .arg(target_repo)
            .arg("--requester")
            .arg("mission-commander")
            .output()
            .expect("Python command should start");
        let result: serde_json::Value =
            decode_backend_json(process_output(output), "Shell Terminal result")
                .expect("terminal submit should succeed");

        let projection = execute_shell_terminal(&config).expect("terminal should inspect");

        assert_eq!(result["status"], "completed");
        assert_eq!(projection.commands[0].command_id, "terminal-command-000001");
        assert_eq!(projection.commands[0].status, "completed");
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_submits_shell_terminal_command_through_python_backend() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-terminal-submit-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(
            tracker_dir.join("PRD.md"),
            "# Shell Terminal Submit Bridge\n",
        )
        .expect("PRD");
        fs::write(
            issues_dir.join("01-terminal.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nTerminal submit bridge.\n\n## Acceptance criteria\n\n- [ ] Terminal submit bridge.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("issue");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo: target_repo.clone(),
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "terminal-submit-bridge".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };
        let request = ShellTerminalCommandRequest {
            correlation_id: "terminal-bridge-submit-2".to_owned(),
            command: "python3 -m unittest --help".to_owned(),
            working_directory: target_repo.display().to_string(),
            requested_paths: Vec::new(),
            requester: "mission-commander".to_owned(),
            access_level: "read".to_owned(),
        };

        let result = execute_shell_terminal_submit(&config, &request)
            .expect("terminal command should execute");

        assert_eq!(result.correlation_id, "terminal-bridge-submit-2");
        assert_eq!(result.classification, "auto-allowed");
        assert_eq!(result.status, "completed");
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("usage:"));
        assert!(result.stderr.is_empty());
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_approves_pending_shell_terminal_command_through_python_backend() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-terminal-decision-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(
            tracker_dir.join("PRD.md"),
            "# Shell Terminal Decision Bridge\n",
        )
        .expect("PRD");
        fs::write(
            issues_dir.join("01-terminal.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nTerminal decision bridge.\n\n## Acceptance criteria\n\n- [ ] Terminal decision bridge.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("issue");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo: target_repo.clone(),
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "terminal-decision-bridge".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };
        let pending = execute_shell_terminal_submit(
            &config,
            &ShellTerminalCommandRequest {
                correlation_id: "terminal-bridge-human-1".to_owned(),
                command: "python3 -c \"print('human approved')\"".to_owned(),
                working_directory: target_repo.display().to_string(),
                requested_paths: Vec::new(),
                requester: "mission-commander".to_owned(),
                access_level: "read".to_owned(),
            },
        )
        .expect("terminal command should be pending");
        let request = ShellTerminalDecisionRequest {
            command_id: pending.command_id.clone(),
            decision: "approve".to_owned(),
            actor: "mission-commander".to_owned(),
            reason: "Approved for the requested repository task.".to_owned(),
        };

        let result = execute_shell_terminal_decision(&config, &request)
            .expect("terminal command should execute after approval");

        assert_eq!(pending.status, "pending-approval");
        assert_eq!(pending.classification, "human-required");
        assert_eq!(result.command_id, pending.command_id);
        assert_eq!(result.status, "completed");
        assert_eq!(result.stdout.trim(), "human approved");
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_creates_additional_path_grant_through_python_backend() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-path-grant-{unique}"));
        let target_repo = root.join("target");
        let external_path = root.join("external-docs");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&external_path).expect("external path");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(
            tracker_dir.join("PRD.md"),
            "# Additional Path Grant Bridge\n",
        )
        .expect("PRD");
        fs::write(
            issues_dir.join("01-terminal.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nAdditional Path Grant bridge.\n\n## Acceptance criteria\n\n- [ ] Additional Path Grant bridge.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("issue");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo,
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "path-grant-bridge".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };
        let request = AdditionalPathGrantRequest {
            correlation_id: "path-grant-bridge-1".to_owned(),
            expected_revision: 0,
            path: external_path.display().to_string(),
            access_level: "write".to_owned(),
            duration_seconds: 900,
            requester: "mission-commander".to_owned(),
        };

        let grant = execute_additional_path_grant_create(&config, &request)
            .expect("path grant should be created");
        let projection = execute_shell_terminal(&config).expect("terminal should inspect");

        assert_eq!(grant.grant_id, "path-grant-000001");
        assert_eq!(grant.correlation_id, "path-grant-bridge-1");
        assert_eq!(grant.path, external_path.display().to_string());
        assert_eq!(grant.access_level, "write");
        assert_eq!(grant.duration_seconds, 900);
        assert_eq!(grant.granted_by, "mission-commander");
        assert_eq!(projection.grants[0].grant_id, grant.grant_id);
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn parses_workspace_queue_projection_and_decision_acknowledgement() {
        let projection_output = ProcessOutput {
            success: true,
            stdout: r#"{
                "schema_version": 1,
                "revision": 2,
                "items": [{
                    "item_id": "issue-change-command-deck-ISS-01-000001",
                    "mission_id": "command-deck",
                    "item_type": "issue-change-proposal",
                    "status": "pending",
                    "source": "issue-slice-inspector",
                    "requested_action": "Change accepted Issue Slice contract",
                    "affected_boundary": "acceptance_criteria",
                    "consequence": "Approval will reopen ISS-01 for re-review.",
                    "issue_id": "ISS-01",
                    "proposed_changes": {
                        "acceptance_criteria": ["Queue proposals preserve state."]
                    }
                }],
                "groups": [{
                    "group_id": "issue-change-proposal:command-deck",
                    "item_type": "issue-change-proposal",
                    "mission_id": "command-deck",
                    "item_count": 1,
                    "items": [{
                        "item_id": "issue-change-command-deck-ISS-01-000001",
                        "mission_id": "command-deck",
                        "item_type": "issue-change-proposal",
                        "status": "pending",
                        "source": "issue-slice-inspector",
                        "requested_action": "Change accepted Issue Slice contract",
                        "affected_boundary": "acceptance_criteria",
                        "consequence": "Approval will reopen ISS-01 for re-review.",
                        "issue_id": "ISS-01",
                        "proposed_changes": {
                            "acceptance_criteria": ["Queue proposals preserve state."]
                        }
                    }]
                }]
            }"#
            .to_owned(),
            stderr: String::new(),
        };
        let acknowledgement_output = ProcessOutput {
            success: true,
            stdout: r#"{
                "correlation_id": "queue-reject-2",
                "outcome": "acknowledged",
                "revision": 3,
                "item_id": "issue-change-command-deck-ISS-01-000001",
                "item_status": "rejected",
                "effect_summary": "Rejected; accepted Mission state is unchanged."
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let projection: WorkspaceQueueProjection =
            decode_backend_json(projection_output, "Workspace Queue projection")
                .expect("projection should decode");
        let acknowledgement: WorkspaceQueueAcknowledgement =
            decode_backend_json(acknowledgement_output, "Workspace Queue acknowledgement")
                .expect("acknowledgement should decode");

        assert_eq!(projection.items[0].item_type, "issue-change-proposal");
        assert_eq!(
            projection.groups[0].group_id,
            "issue-change-proposal:command-deck"
        );
        assert_eq!(
            projection.items[0].proposed_changes["acceptance_criteria"][0],
            "Queue proposals preserve state."
        );
        assert_eq!(acknowledgement.item_status, "rejected");
        assert_eq!(acknowledgement.revision, 3);
    }

    #[test]
    fn parses_mission_draft_projection() {
        let projection_output = ProcessOutput {
            success: true,
            stdout: r#"{
                "schema_version": 1,
                "revision": 2,
                "drafts": [{
                    "draft_id": "mission-draft-command-deck-000001",
                    "mission_id": "command-deck",
                    "status": "draft",
                    "proposed_goal": "Create a focused Command Deck follow-up mission.",
                    "included_ad_hoc_work": [{
                        "work_id": "ADHOC-000001",
                        "source": "agent-console",
                        "status": "pending",
                        "acceptance_criteria": ["Represent selected ad hoc work."],
                        "allowed_paths": ["docs/mission-draft.md"],
                        "originating_message_id": "console-000001"
                    }],
                    "excluded_ad_hoc_work_ids": ["ADHOC-000002"],
                    "new_work_items": ["Add confirmation handling."],
                    "dependencies": ["Issue 10 remains authoritative."],
                    "unresolved_decisions": ["Choose final UI placement."]
                }]
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let projection: MissionDraftProjection =
            decode_backend_json(projection_output, "Mission Draft projection")
                .expect("projection should decode");

        assert_eq!(projection.revision, 2);
        assert_eq!(
            projection.drafts[0].draft_id,
            "mission-draft-command-deck-000001"
        );
        assert_eq!(
            projection.drafts[0].included_ad_hoc_work[0].work_id,
            "ADHOC-000001"
        );
        assert_eq!(
            projection.drafts[0].excluded_ad_hoc_work_ids[0],
            "ADHOC-000002"
        );
    }

    #[test]
    fn desktop_bridge_submits_action_then_reads_the_ordered_update() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-sync-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Live Sync Mission\n").expect("PRD");
        fs::write(
            issues_dir.join("01-sync.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nSync.\n\n## Acceptance criteria\n\n- [ ] Sync.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("issue");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo,
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "live-sync".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };

        let acknowledgement = execute_action(
            &config,
            &WorkspaceActionRequest {
                correlation_id: "view-activity-1".to_owned(),
                expected_revision: 1,
                operations_view: "activity".to_owned(),
            },
        )
        .expect("action should be acknowledged");
        let batch = execute_updates(&config, 1).expect("updates should be returned");

        assert_eq!(acknowledgement.outcome, "acknowledged");
        assert_eq!(acknowledgement.revision, 2);
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].correlation_id, "view-activity-1");
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_switches_active_catalog_mission() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-mission-switch-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let background_tracker = root.join("background-tracker");
        let background_issues = background_tracker.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::create_dir_all(&background_issues).expect("background issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Primary Mission\n").expect("primary PRD");
        fs::write(
            issues_dir.join("01-primary.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nPrimary.\n\n## Acceptance criteria\n\n- [ ] Primary.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("primary issue");
        fs::write(background_tracker.join("PRD.md"), "# Background Mission\n")
            .expect("background PRD");
        fs::write(
            background_issues.join("01-background.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nBackground.\n\n## Acceptance criteria\n\n- [ ] Background.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("background issue");
        let catalog_path = root.join("mission-catalog.json");
        fs::write(
            &catalog_path,
            format!(
                r#"{{"schema_version":1,"missions":[{{"mission_id":"background-mission","tracker_dir":"{}"}}]}}"#,
                background_tracker.display()
            ),
        )
        .expect("catalog");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo,
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "primary-mission".to_owned(),
            agent_config: None,
            mission_catalog: Some(catalog_path),
        };

        let acknowledgement = execute_mission_switch(
            &config,
            &WorkspaceMissionSwitchRequest {
                correlation_id: "active-mission-background-1".to_owned(),
                expected_revision: 1,
                active_mission_id: "background-mission".to_owned(),
            },
        )
        .expect("mission switch should be acknowledged");
        let restored = execute_snapshot(&config).expect("snapshot should restore switched mission");

        assert_eq!(acknowledgement.revision, 2);
        assert_eq!(restored.active_mission.unwrap().id, "background-mission");
        assert_eq!(restored.mission_board.prd_title, "Background Mission");
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_restores_scoped_agent_console_history() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-console-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Agent Console Mission\n").expect("PRD");
        fs::write(
            issues_dir.join("01-console.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nConsole.\n\n## Acceptance criteria\n\n- [ ] Console.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("issue");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo,
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "agent-console".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };

        execute_scope(
            &config,
            &WorkspaceScopeRequest {
                correlation_id: "scope-issue-1".to_owned(),
                expected_revision: 1,
                scope_kind: "issue-slice".to_owned(),
                scope_target: "ISS-01".to_owned(),
                scope_label: "Console".to_owned(),
            },
        )
        .expect("scope should be acknowledged");
        execute_console_message(
            &config,
            &AgentConsoleMessageRequest {
                role: "user".to_owned(),
                content: "Explain this Issue Slice.".to_owned(),
                outcome: "proposed".to_owned(),
                source: "mission-commander".to_owned(),
                expected_revision: 2,
                scope_kind: "issue-slice".to_owned(),
                scope_target: "ISS-01".to_owned(),
                scope_label: "Console".to_owned(),
            },
        )
        .expect("message should append");

        let restored = execute_console_history(&config).expect("history should restore");

        assert_eq!(restored.messages.len(), 1);
        assert_eq!(restored.messages[0].message_id, "console-000001");
        assert_eq!(restored.messages[0].scope.kind, "issue-slice");
        assert_eq!(restored.messages[0].scope.target_id, "ISS-01");
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_proposes_ad_hoc_delegation_into_workspace_queue() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-ad-hoc-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Ad Hoc Mission\n").expect("PRD");
        fs::write(
            issues_dir.join("01-ad-hoc.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nAd hoc.\n\n## Acceptance criteria\n\n- [ ] Ad hoc.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("issue");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo: target_repo.clone(),
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "ad-hoc-mission".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };
        let origin = execute_console_message(
            &config,
            &AgentConsoleMessageRequest {
                role: "user".to_owned(),
                content: "Refresh smoke-test notes.".to_owned(),
                outcome: "proposed".to_owned(),
                source: "mission-commander".to_owned(),
                expected_revision: 1,
                scope_kind: "working-directory".to_owned(),
                scope_target: target_repo.display().to_string(),
                scope_label: "target".to_owned(),
            },
        )
        .expect("origin message should append");

        let acknowledgement = execute_ad_hoc_delegation_proposal(
            &config,
            &AdHocDelegationProposalRequest {
                correlation_id: "tauri-ad-hoc-1".to_owned(),
                expected_revision: 1,
                source: "agent-console".to_owned(),
                scope_kind: "working-directory".to_owned(),
                scope_target: target_repo.display().to_string(),
                scope_label: "target".to_owned(),
                acceptance_criteria: vec![
                    "Smoke-test notes mention the focused unit command.".to_owned()
                ],
                allowed_paths: vec!["docs/smoke-tests.md".to_owned()],
                command_policy: std::collections::BTreeMap::from([(
                    "python3 -m unittest tests.test_workspace_snapshot".to_owned(),
                    "auto-allowed".to_owned(),
                )]),
                proposed_agent: "qwen-coder-local-1".to_owned(),
                originating_message_id: origin.message_id,
                mission_id: None,
            },
        )
        .expect("ad hoc proposal should be acknowledged");
        let queue = execute_workspace_queue(&config).expect("queue should inspect");

        assert_eq!(acknowledgement.item_status, "pending");
        assert_eq!(queue.items[0].item_type, "ad-hoc-delegation");
        assert_eq!(queue.items[0].issue_id, "ADHOC-000001");
        assert_eq!(
            queue.items[0].proposed_changes["allowed_paths"][0],
            "docs/smoke-tests.md"
        );
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_creates_mission_draft_without_accepting_mission_state() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-draft-create-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Mission Draft Creation\n").expect("PRD");
        fs::write(
            issues_dir.join("01-draft-create.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nDraft creation.\n\n## Acceptance criteria\n\n- [ ] Draft creation.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("issue");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo: target_repo.clone(),
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "draft-create".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };
        let origin = execute_console_message(
            &config,
            &AgentConsoleMessageRequest {
                role: "user".to_owned(),
                content: "Promote useful ad hoc work into a mission draft.".to_owned(),
                outcome: "proposed".to_owned(),
                source: "mission-commander".to_owned(),
                expected_revision: 1,
                scope_kind: "working-directory".to_owned(),
                scope_target: target_repo.display().to_string(),
                scope_label: "target".to_owned(),
            },
        )
        .expect("origin message should append");
        for (correlation_id, criterion, path) in [
            (
                "tauri-draft-ad-hoc-1",
                "Selected ad hoc work is represented in the Mission Draft.",
                "docs/selected.md",
            ),
            (
                "tauri-draft-ad-hoc-2",
                "Excluded ad hoc work stays outside the Mission Draft.",
                "docs/excluded.md",
            ),
        ] {
            execute_ad_hoc_delegation_proposal(
                &config,
                &AdHocDelegationProposalRequest {
                    correlation_id: correlation_id.to_owned(),
                    expected_revision: 1,
                    source: "agent-console".to_owned(),
                    scope_kind: "working-directory".to_owned(),
                    scope_target: target_repo.display().to_string(),
                    scope_label: "target".to_owned(),
                    acceptance_criteria: vec![criterion.to_owned()],
                    allowed_paths: vec![path.to_owned()],
                    command_policy: std::collections::BTreeMap::new(),
                    proposed_agent: "qwen-coder-local-1".to_owned(),
                    originating_message_id: origin.message_id.clone(),
                    mission_id: None,
                },
            )
            .expect("ad hoc proposal should be acknowledged");
        }

        let acknowledgement = execute_mission_draft_create(
            &config,
            &MissionDraftCreateRequest {
                correlation_id: "tauri-mission-draft-create-1".to_owned(),
                expected_revision: 1,
                proposed_goal: "Create a focused follow-up mission.".to_owned(),
                selected_ad_hoc_ids: vec!["ADHOC-000001".to_owned()],
                excluded_ad_hoc_ids: vec!["ADHOC-000002".to_owned()],
                new_work_items: vec!["Add explicit confirmation handling.".to_owned()],
                dependencies: vec!["Issue 10 approval remains authoritative.".to_owned()],
                unresolved_decisions: vec!["Choose final queue grouping.".to_owned()],
                mission_id: None,
            },
        )
        .expect("draft creation should be acknowledged");
        let projection = execute_mission_drafts(&config).expect("drafts should inspect");
        let snapshot = execute_snapshot(&config).expect("snapshot should inspect");

        assert_eq!(acknowledgement.draft_status, "draft");
        assert_eq!(acknowledgement.accepted_issue_id, "");
        assert_eq!(
            projection.drafts[0].included_ad_hoc_work[0].work_id,
            "ADHOC-000001"
        );
        assert_eq!(
            projection.drafts[0].excluded_ad_hoc_work_ids[0],
            "ADHOC-000002"
        );
        assert_eq!(snapshot.mission_board.issue_count, 1);
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_reads_mission_drafts_from_python_backend() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-mission-drafts-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Mission Draft Bridge\n").expect("PRD");
        fs::write(
            issues_dir.join("01-draft.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nDraft bridge.\n\n## Acceptance criteria\n\n- [ ] Draft bridge.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("issue");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo,
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "draft-bridge".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };

        let output = configured_python_command(&config, "mission-draft-create")
            .arg("--correlation-id")
            .arg("mission-draft-bridge-create-1")
            .arg("--expected-revision")
            .arg("1")
            .arg("--proposed-goal")
            .arg("Create a bridge-visible Mission Draft.")
            .arg("--new-work-item")
            .arg("Expose Mission Drafts through Tauri.")
            .output()
            .expect("Python command should start");
        let acknowledgement: serde_json::Value =
            decode_backend_json(process_output(output), "Mission Draft acknowledgement")
                .expect("draft creation should be acknowledged");
        let projection = execute_mission_drafts(&config).expect("drafts should inspect");

        assert_eq!(acknowledgement["draft_status"], "draft");
        assert_eq!(projection.revision, 2);
        assert_eq!(
            projection.drafts[0].draft_id,
            "mission-draft-draft-bridge-000001"
        );
        assert_eq!(
            projection.drafts[0].new_work_items[0],
            "Expose Mission Drafts through Tauri."
        );
        let confirmed = execute_mission_draft_decision(
            &config,
            &MissionDraftDecisionRequest {
                correlation_id: "mission-draft-bridge-confirm-1".to_owned(),
                expected_revision: projection.revision,
                draft_id: projection.drafts[0].draft_id.clone(),
                decision: "confirm".to_owned(),
                reason: "Confirmed through Tauri bridge.".to_owned(),
            },
        )
        .expect("draft confirmation should be acknowledged");
        let confirmed_projection =
            execute_mission_drafts(&config).expect("confirmed draft should inspect");

        assert_eq!(confirmed.draft_status, "confirmed");
        assert_eq!(confirmed.accepted_issue_id, "ISS-02");
        assert_eq!(confirmed_projection.drafts[0].status, "confirmed");
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_curates_and_restores_working_context() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-context-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Working Context Mission\n").expect("PRD");
        fs::write(
            issues_dir.join("01-context.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nContext.\n\n## Acceptance criteria\n\n- [ ] Context.\n\n## Blocked by\n\nNone - can start immediately\n",
        )
        .expect("issue");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo,
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "working-context".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };
        execute_console_message(
            &config,
            &AgentConsoleMessageRequest {
                role: "user".to_owned(),
                content: "Keep this source.".to_owned(),
                outcome: "proposed".to_owned(),
                source: "mission-commander".to_owned(),
                expected_revision: 1,
                scope_kind: "working-directory".to_owned(),
                scope_target: config.target_repo.to_string_lossy().into_owned(),
                scope_label: "target".to_owned(),
            },
        )
        .expect("message should append");

        let initial = execute_working_context(&config).expect("context should inspect");
        let acknowledgement = execute_working_context_curate(
            &config,
            &WorkingContextCurationRequest {
                source_id: "message:console-000001".to_owned(),
                disposition: "pinned".to_owned(),
                expected_context_revision: 1,
            },
        )
        .expect("curation should be acknowledged");
        let restored = execute_working_context(&config).expect("context should restore");

        assert_eq!(initial.revision, 1);
        assert_eq!(acknowledgement.revision, 2);
        assert_eq!(restored.revision, 2);
        assert!(restored.sources.iter().any(|source| {
            source.source_id == "message:console-000001" && source.disposition == "pinned"
        }));
        assert!(restored.sources.iter().any(|source| {
            source.kind == "shared-context" && source.governed && !source.eligible
        }));
        let governed_source = restored
            .sources
            .iter()
            .find(|source| source.kind == "shared-context")
            .expect("governed Shared Context source");
        let failure = execute_working_context_curate(
            &config,
            &WorkingContextCurationRequest {
                source_id: governed_source.source_id.clone(),
                disposition: "excluded".to_owned(),
                expected_context_revision: 2,
            },
        )
        .expect_err("governed curation must reject");
        assert_eq!(failure.code, "context-source-ineligible");
        assert_eq!(
            execute_working_context(&config)
                .expect("rejected curation leaves context readable")
                .revision,
            2
        );
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_switches_catalog_mission_and_preserves_conversation() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-missions-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("primary");
        let background_tracker = root.join("background");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(tracker_dir.join("issues")).expect("primary issues");
        fs::create_dir_all(background_tracker.join("issues")).expect("background issues");
        fs::write(tracker_dir.join("PRD.md"), "# Primary Mission\n").expect("primary PRD");
        fs::write(background_tracker.join("PRD.md"), "# Background Mission\n")
            .expect("background PRD");
        let issue = "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nWork.\n\n## Acceptance criteria\n\n- [ ] Work.\n\n## Blocked by\n\nNone - can start immediately\n";
        fs::write(tracker_dir.join("issues/01-primary.md"), issue).expect("primary issue");
        fs::write(background_tracker.join("issues/01-background.md"), issue)
            .expect("background issue");
        let catalog = root.join("mission-catalog.json");
        fs::write(
            &catalog,
            format!(
                r#"{{"schema_version":1,"missions":[{{"mission_id":"background","tracker_dir":{}}}]}}"#,
                serde_json::to_string(&background_tracker).expect("catalog path")
            ),
        )
        .expect("catalog");
        let config = BridgeConfig {
            python: "python3".to_owned(),
            backend_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root"),
            target_repo,
            tracker_dir,
            issues_dir: None,
            runtime_root,
            mission_id: "primary".to_owned(),
            agent_config: None,
            mission_catalog: Some(catalog),
        };
        let initial = execute_snapshot(&config).expect("initial snapshot");
        execute_console_message(
            &config,
            &AgentConsoleMessageRequest {
                role: "user".to_owned(),
                content: "Continuous mission conversation".to_owned(),
                outcome: "proposed".to_owned(),
                source: "mission-commander".to_owned(),
                expected_revision: 1,
                scope_kind: "working-directory".to_owned(),
                scope_target: config.target_repo.to_string_lossy().into_owned(),
                scope_label: "target".to_owned(),
            },
        )
        .expect("message");

        let acknowledgement = execute_mission_switch(
            &config,
            &WorkspaceMissionSwitchRequest {
                correlation_id: "switch-background-1".to_owned(),
                expected_revision: 1,
                active_mission_id: "background".to_owned(),
            },
        )
        .expect("switch acknowledgement");
        let restored = execute_snapshot(&config).expect("restored snapshot");
        let history = execute_console_history(&config).expect("restored history");

        assert_eq!(acknowledgement.revision, 2);
        assert_eq!(
            restored.active_mission.expect("active mission").id,
            "background"
        );
        assert_eq!(restored.workspace_session.id, initial.workspace_session.id);
        assert_eq!(restored.conversation_scope.kind, "working-directory");
        assert_eq!(restored.missions.len(), 2);
        assert_eq!(
            history.messages[0].content,
            "Continuous mission conversation"
        );
        fs::remove_dir_all(root).expect("fixture cleanup");
    }
}
