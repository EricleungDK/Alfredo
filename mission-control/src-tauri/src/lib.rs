use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, OnceLock,
};
use std::time::Instant;

#[cfg(feature = "rust-orchestrator-prototype")]
#[path = "../prototypes/rust-orchestrator-slice/src/model.rs"]
mod rust_orchestrator_prototype_model;

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

const MEASUREMENT_ENVIRONMENT: [(&str, &str); 12] = [
    ("jsonl_path", "ALFREDO_MEASUREMENT_JSONL"),
    ("run_id", "ALFREDO_MEASUREMENT_RUN_ID"),
    ("sample_id", "ALFREDO_MEASUREMENT_SAMPLE_ID"),
    ("cohort_id", "ALFREDO_MEASUREMENT_COHORT_ID"),
    ("correlation_id", "ALFREDO_MEASUREMENT_CORRELATION_ID"),
    ("fixture_id", "ALFREDO_MEASUREMENT_FIXTURE_ID"),
    ("fixture_sha256", "ALFREDO_MEASUREMENT_FIXTURE_SHA256"),
    ("source_sha256", "ALFREDO_MEASUREMENT_SOURCE_SHA256"),
    ("artifact_sha256", "ALFREDO_MEASUREMENT_ARTIFACT_SHA256"),
    ("variant", "ALFREDO_MEASUREMENT_VARIANT"),
    ("workflow", "ALFREDO_MEASUREMENT_WORKFLOW"),
    ("mode", "ALFREDO_MEASUREMENT_MODE"),
];
static PROCESS_MONOTONIC_ORIGIN: OnceLock<Instant> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PerformanceIdentity {
    run_id: String,
    sample_id: String,
    cohort_id: String,
    correlation_id: String,
    fixture_id: String,
    fixture_sha256: String,
    source_sha256: String,
    artifact_sha256: String,
    variant: String,
    workflow: String,
    mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desktop_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desktop_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PerformanceControl {
    jsonl_path: PathBuf,
    #[serde(flatten)]
    identity: PerformanceIdentity,
}

#[derive(Debug, Deserialize)]
pub struct PerformanceMarkRequest {
    pub stage: String,
    pub boundary: String,
    pub clock: String,
    #[serde(default)]
    pub monotonic_ns: String,
    #[serde(default)]
    pub clock_id: String,
    #[serde(default)]
    pub detail: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct PerformanceMarkAcknowledgement {
    pub recorded: bool,
}

fn measurement_failure(message: impl Into<String>) -> BridgeFailure {
    BridgeFailure {
        code: "measurement-failure".to_owned(),
        message: message.into(),
        recoverable: false,
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_performance_identity(
    path: PathBuf,
    identity: PerformanceIdentity,
) -> Result<(PathBuf, PerformanceIdentity), BridgeFailure> {
    if !path.is_absolute() {
        return Err(measurement_failure(
            "ALFREDO_MEASUREMENT_JSONL must be absolute",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| measurement_failure("measurement output has no parent directory"))?;
    parent.canonicalize().map_err(|error| {
        measurement_failure(format!("measurement output parent is unavailable: {error}"))
    })?;
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(measurement_failure(
                "measurement output must be a regular non-symlink file",
            ));
        }
    }
    for (field, value) in [
        ("fixture_sha256", &identity.fixture_sha256),
        ("source_sha256", &identity.source_sha256),
        ("artifact_sha256", &identity.artifact_sha256),
    ] {
        if !valid_sha256(value) {
            return Err(measurement_failure(format!(
                "{field} must be a lowercase SHA-256"
            )));
        }
    }
    if identity.mode != "process-cold" && identity.mode != "process-warm" {
        return Err(measurement_failure(
            "measurement mode must be process-cold or process-warm",
        ));
    }
    match (identity.desktop_pid, identity.desktop_session_id.as_deref()) {
        (None, None) => {}
        (Some(pid), Some(session)) if pid > 0 && !session.trim().is_empty() => {}
        _ => {
            return Err(measurement_failure(
                "desktop_pid and desktop_session_id must be provided together",
            ))
        }
    }
    Ok((path, identity))
}

fn performance_identity_from_control(
    control_path: &Path,
) -> Result<Option<(PathBuf, PerformanceIdentity)>, BridgeFailure> {
    if !control_path.is_absolute() {
        return Err(measurement_failure(
            "ALFREDO_MEASUREMENT_CONTROL_PATH must be absolute",
        ));
    }
    let parent = control_path
        .parent()
        .ok_or_else(|| measurement_failure("measurement control path has no parent"))?;
    parent.canonicalize().map_err(|error| {
        measurement_failure(format!(
            "measurement control parent is unavailable: {error}"
        ))
    })?;
    let metadata = match fs::symlink_metadata(control_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(measurement_failure(format!(
                "measurement control metadata is unavailable: {error}"
            )))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(measurement_failure(
            "ALFREDO_MEASUREMENT_CONTROL_PATH must be a regular non-symlink file",
        ));
    }
    if metadata.len() > 16_384 {
        return Err(measurement_failure(
            "measurement control file exceeds 16 KiB",
        ));
    }
    let bytes = fs::read(control_path).map_err(|error| {
        measurement_failure(format!(
            "measurement control file could not be read: {error}"
        ))
    })?;
    let control: PerformanceControl = serde_json::from_slice(&bytes).map_err(|error| {
        measurement_failure(format!("measurement control file is invalid: {error}"))
    })?;
    validate_performance_identity(control.jsonl_path, control.identity).map(Some)
}

fn performance_identity() -> Result<Option<(PathBuf, PerformanceIdentity)>, BridgeFailure> {
    let control_path = std::env::var("ALFREDO_MEASUREMENT_CONTROL_PATH")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let legacy_present = MEASUREMENT_ENVIRONMENT.iter().any(|(_, variable)| {
        std::env::var(variable)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    });
    if let Some(control_path) = control_path {
        if legacy_present {
            return Err(measurement_failure(
                "ALFREDO_MEASUREMENT_CONTROL_PATH must not be combined with legacy measurement identity",
            ));
        }
        return performance_identity_from_control(Path::new(&control_path));
    }
    let mut values = HashMap::new();
    let mut missing = Vec::new();
    for (field, variable) in MEASUREMENT_ENVIRONMENT {
        match std::env::var(variable) {
            Ok(value) if !value.trim().is_empty() => {
                values.insert(field, value);
            }
            _ => missing.push(variable),
        }
    }
    if missing.len() == MEASUREMENT_ENVIRONMENT.len() {
        return Ok(None);
    }
    if !missing.is_empty() {
        return Err(measurement_failure(format!(
            "measurement environment is incomplete: missing {}",
            missing.join(", ")
        )));
    }
    let path = PathBuf::from(
        values
            .remove("jsonl_path")
            .expect("measurement path was inserted"),
    );
    let take = |field: &str, values: &mut HashMap<&str, String>| {
        values
            .remove(field)
            .expect("measurement identity field was inserted")
    };
    let identity = PerformanceIdentity {
        run_id: take("run_id", &mut values),
        sample_id: take("sample_id", &mut values),
        cohort_id: take("cohort_id", &mut values),
        correlation_id: take("correlation_id", &mut values),
        fixture_id: take("fixture_id", &mut values),
        fixture_sha256: take("fixture_sha256", &mut values),
        source_sha256: take("source_sha256", &mut values),
        artifact_sha256: take("artifact_sha256", &mut values),
        variant: take("variant", &mut values),
        workflow: take("workflow", &mut values),
        mode: take("mode", &mut values),
        desktop_pid: None,
        desktop_session_id: None,
    };
    validate_performance_identity(path, identity).map(Some)
}

fn measurement_stage_matches_workflow(stage: &str, workflow: &str) -> bool {
    match workflow {
        "startup" => matches!(
            stage,
            "S0" | "S1" | "S2" | "S3" | "S4" | "S5" | "S6" | "S7" | "S8" | "S9"
        ),
        "queue-defer" => matches!(stage, "R0" | "R1" | "R2" | "R3" | "R4" | "R5"),
        "queue-approve" | "session-claim" => {
            matches!(stage, "R0" | "R1" | "R2" | "R3" | "R4" | "R5" | "R6")
        }
        _ => false,
    }
}

fn write_performance_mark(
    path: &PathBuf,
    identity: &PerformanceIdentity,
    source: &str,
    clock_id: &str,
    stage: &str,
    boundary: &str,
    monotonic_ns: &str,
    detail: serde_json::Value,
) -> Result<(), BridgeFailure> {
    if !measurement_stage_matches_workflow(stage, &identity.workflow) {
        return Err(measurement_failure(format!(
            "{stage} is not valid for workflow {}",
            identity.workflow
        )));
    }
    if boundary != "start" && boundary != "end" {
        return Err(measurement_failure(format!(
            "unknown measurement boundary: {boundary}"
        )));
    }
    if source.trim().is_empty() || clock_id.trim().is_empty() {
        return Err(measurement_failure(
            "measurement source and clock id must not be empty",
        ));
    }
    if monotonic_ns.is_empty() || !monotonic_ns.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(measurement_failure(
            "measurement monotonic_ns must be an unsigned integer",
        ));
    }
    if !detail.is_object() {
        return Err(measurement_failure("measurement detail must be an object"));
    }
    let mut payload = serde_json::json!({
        "schema_version": 1,
        "record_type": "stage-mark",
        "run_id": identity.run_id,
        "sample_id": identity.sample_id,
        "cohort_id": identity.cohort_id,
        "correlation_id": identity.correlation_id,
        "fixture_id": identity.fixture_id,
        "fixture_sha256": identity.fixture_sha256,
        "source_sha256": identity.source_sha256,
        "artifact_sha256": identity.artifact_sha256,
        "variant": identity.variant,
        "workflow": identity.workflow,
        "mode": identity.mode,
        "source": source,
        "clock_id": clock_id,
        "stage": stage,
        "boundary": boundary,
        "monotonic_ns": monotonic_ns,
        "detail": detail,
    });
    if let (Some(desktop_pid), Some(desktop_session_id)) =
        (identity.desktop_pid, identity.desktop_session_id.as_deref())
    {
        let object = payload
            .as_object_mut()
            .expect("measurement payload is always an object");
        object.insert("desktop_pid".to_owned(), serde_json::json!(desktop_pid));
        object.insert(
            "desktop_session_id".to_owned(),
            serde_json::json!(desktop_session_id),
        );
    }
    let mut encoded = serde_json::to_vec(&payload).map_err(|error| {
        measurement_failure(format!("measurement mark did not serialize: {error}"))
    })?;
    encoded.push(b'\n');
    if encoded.len() > 16_384 {
        return Err(measurement_failure("measurement stage mark exceeds 16 KiB"));
    }
    let mut output = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            measurement_failure(format!("measurement output could not be opened: {error}"))
        })?;
    output.write_all(&encoded).map_err(|error| {
        measurement_failure(format!("measurement stage mark write failed: {error}"))
    })
}

fn native_monotonic_ns() -> String {
    PROCESS_MONOTONIC_ORIGIN
        .get_or_init(Instant::now)
        .elapsed()
        .as_nanos()
        .to_string()
}

fn record_native_performance_mark(
    stage: &str,
    boundary: &str,
    detail: serde_json::Value,
) -> Result<bool, BridgeFailure> {
    let Some((path, identity)) = performance_identity()? else {
        return Ok(false);
    };
    if !measurement_stage_matches_workflow(stage, &identity.workflow) {
        return Ok(false);
    }
    write_performance_mark(
        &path,
        &identity,
        "native-shell",
        &format!("native-shell:{}", std::process::id()),
        stage,
        boundary,
        &native_monotonic_ns(),
        detail,
    )?;
    Ok(true)
}

pub fn record_native_main_start() -> Result<(), BridgeFailure> {
    record_native_performance_mark("S2", "start", serde_json::json!({"outcome": "pass"}))?;
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AlfredoLaunchContext {
    pub schema_version: u32,
    pub selected_agent: String,
    pub selected_model: String,
    pub starting_location: String,
    pub coding_workspace: Option<String>,
    pub active_mission: Option<String>,
    pub revision: u64,
    pub known_missions: Vec<MissionChoiceOption>,
    pub phase: String,
    pub runtime_root: String,
    pub recent_workspaces: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MissionChoiceOption {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CodingWorkspaceSelectionRequest {
    pub correlation_id: String,
    pub workspace_path: String,
    pub selection_mode: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CodingWorkspaceAcknowledgement {
    pub schema_version: u32,
    pub correlation_id: String,
    pub outcome: String,
    pub starting_location: String,
    pub coding_workspace: String,
    pub selection_mode: String,
    pub active_mission: Option<String>,
    pub replayed: bool,
    pub message: String,
    #[serde(default)]
    pub known_missions: Vec<MissionChoiceOption>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MissionChoiceRequest {
    pub correlation_id: String,
    pub expected_revision: u64,
    pub choice: String,
    pub mission_id: String,
    #[serde(default)]
    pub mission_title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MissionChoiceAcknowledgement {
    pub schema_version: u32,
    pub correlation_id: String,
    pub outcome: String,
    pub coding_workspace: String,
    pub choice: String,
    pub active_mission: String,
    pub revision: u64,
    pub replayed: bool,
    pub missions: Vec<MissionChoiceOption>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize)]
struct PersistedMissionOption {
    id: String,
    title: String,
    tracker_dir: String,
    #[serde(default)]
    issues_dir: String,
}

#[derive(Debug, Deserialize)]
struct PersistedWorkspaceSession {
    starting_location: String,
    coding_workspace: String,
    revision: u64,
    active_mission: Option<String>,
    missions: Vec<PersistedMissionOption>,
    mission_catalog: String,
}

#[derive(Debug, Deserialize)]
struct PersistedWorkspaceSessions {
    schema_version: u32,
    sessions: Vec<PersistedWorkspaceSession>,
}

#[derive(Clone, Debug, Default)]
struct WorkspaceBindingState {
    coding_workspace: Option<PathBuf>,
    active_mission: Option<String>,
    revision: u64,
    known_missions: Vec<MissionChoiceOption>,
    missions: Vec<PersistedMissionOption>,
    mission_catalog: Option<PathBuf>,
    persistence_failure: Option<BridgeFailure>,
    pending_selection: Option<PendingWorkspaceSelection>,
    accepted_selection: Option<AcceptedWorkspaceSelection>,
}

#[derive(Clone, Debug)]
struct PendingWorkspaceSelection {
    correlation_id: String,
    workspace_path: String,
    selection_mode: String,
}

#[derive(Clone, Debug)]
struct AcceptedWorkspaceSelection {
    correlation_id: String,
    workspace_path: String,
    selection_mode: String,
    acknowledgement: CodingWorkspaceAcknowledgement,
}

#[derive(Debug, Default)]
pub struct WorkspaceBinding {
    inner: Mutex<WorkspaceBindingState>,
}

impl WorkspaceBinding {
    fn from_environment() -> Self {
        Self::default()
    }

    fn from_config(config: &BridgeConfig) -> Self {
        let binding = Self::default();
        if let Err(error) = binding.reload_from_persistence(config) {
            let mut state = binding
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.persistence_failure = Some(error);
            drop(state);
            return binding;
        }
        binding
    }

    fn state(&self) -> WorkspaceBindingState {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn ensure_persistence_healthy(&self) -> Result<(), BridgeFailure> {
        self.state().persistence_failure.map_or(Ok(()), Err)
    }

    fn reload_from_persistence(&self, config: &BridgeConfig) -> Result<(), BridgeFailure> {
        let path = config.runtime_root.join("workspace-sessions.json");
        if !path.exists() {
            return Ok(());
        }
        let payload: PersistedWorkspaceSessions =
            serde_json::from_slice(&fs::read(&path).map_err(|error| BridgeFailure {
                code: "persistence-read-failure".to_owned(),
                message: format!("The canonical Workspace journey could not be read: {error}"),
                recoverable: true,
            })?)
            .map_err(|error| BridgeFailure {
                code: "persistence-read-failure".to_owned(),
                message: format!("The canonical Workspace journey is invalid: {error}"),
                recoverable: false,
            })?;
        if payload.schema_version != 1 {
            return Err(BridgeFailure {
                code: "persistence-read-failure".to_owned(),
                message: format!(
                    "Unsupported canonical Workspace journey schema version {}.",
                    payload.schema_version
                ),
                recoverable: false,
            });
        }
        let starting_location = canonical_or_original(&bridge_starting_location(config));
        let matching_sessions = payload
            .sessions
            .iter()
            .filter(|session| {
                canonical_or_original(Path::new(&session.starting_location)) == starting_location
            })
            .collect::<Vec<_>>();
        if matching_sessions.len() > 1 {
            // A Starting Location can own several acknowledged workspaces. Do
            // not infer one from recency; leave the gate active so the user
            // can acknowledge the exact repository explicitly.
            return Ok(());
        }
        let Some(session) = matching_sessions.first() else {
            return Ok(());
        };
        let coding_workspace = PathBuf::from(&session.coding_workspace)
            .canonicalize()
            .map_err(|error| BridgeFailure {
                code: "workspace-restore-failure".to_owned(),
                message: format!("The acknowledged Coding Workspace is unavailable: {error}"),
                recoverable: true,
            })?;
        if session.revision == 0 || session.mission_catalog.trim().is_empty() {
            return Err(BridgeFailure {
                code: "persistence-read-failure".to_owned(),
                message:
                    "The canonical Workspace journey has an invalid revision or Mission catalog."
                        .to_owned(),
                recoverable: false,
            });
        }
        let known_missions = session
            .missions
            .iter()
            .map(|mission| MissionChoiceOption {
                id: mission.id.clone(),
                title: mission.title.clone(),
            })
            .collect::<Vec<_>>();
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.coding_workspace = Some(coding_workspace);
        state.active_mission = session.active_mission.clone();
        state.revision = session.revision;
        state.known_missions = known_missions;
        state.missions = session.missions.clone();
        state.mission_catalog = Some(PathBuf::from(&session.mission_catalog));
        state.persistence_failure = None;
        Ok(())
    }

    fn require_workspace(&self) -> Result<(), BridgeFailure> {
        self.ensure_persistence_healthy()?;
        if self.state().coding_workspace.is_none() {
            return Err(BridgeFailure {
                code: "coding-workspace-selection-required".to_owned(),
                message: "Choose or create a Coding Workspace before choosing a Mission."
                    .to_owned(),
                recoverable: true,
            });
        }
        Ok(())
    }

    fn bound_config(&self, config: &BridgeConfig) -> Result<BridgeConfig, BridgeFailure> {
        self.require_active_mission()?;
        let state = self.state();
        let active_mission = state.active_mission.clone().ok_or_else(|| BridgeFailure {
            code: "mission-selection-required".to_owned(),
            message: "Choose Resume Mission or Start New Mission before using Mission commands."
                .to_owned(),
            recoverable: true,
        })?;
        let mission = state
            .missions
            .iter()
            .find(|mission| mission.id == active_mission)
            .ok_or_else(|| BridgeFailure {
                code: "persistence-read-failure".to_owned(),
                message: "The active Mission is missing from the canonical Mission catalog."
                    .to_owned(),
                recoverable: false,
            })?;
        let coding_workspace = state.coding_workspace.ok_or_else(|| BridgeFailure {
            code: "coding-workspace-selection-required".to_owned(),
            message: "Choose or create a Coding Workspace before using Mission commands."
                .to_owned(),
            recoverable: true,
        })?;
        let mut bound = config.clone();
        bound.target_repo = coding_workspace;
        bound.tracker_dir = PathBuf::from(&mission.tracker_dir);
        bound.issues_dir = if mission.issues_dir.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(&mission.issues_dir))
        };
        bound.mission_id = active_mission;
        bound.mission_catalog = state.mission_catalog;
        Ok(bound)
    }

    pub fn acknowledge(
        &self,
        request: &CodingWorkspaceSelectionRequest,
        acknowledgement: &CodingWorkspaceAcknowledgement,
    ) -> Result<(), BridgeFailure> {
        if acknowledgement.schema_version != 1
            || acknowledgement.outcome != "acknowledged"
            || acknowledgement.active_mission.is_some()
            || acknowledgement.correlation_id != request.correlation_id
            || acknowledgement.selection_mode != request.selection_mode
        {
            return Err(BridgeFailure {
                code: "invalid-workspace-acknowledgement".to_owned(),
                message: "The Orchestrator returned an invalid Coding Workspace acknowledgement."
                    .to_owned(),
                recoverable: true,
            });
        }
        let coding_workspace = PathBuf::from(&acknowledgement.coding_workspace)
            .canonicalize()
            .map_err(|error| BridgeFailure {
                code: "invalid-workspace-path".to_owned(),
                message: format!(
                    "The acknowledged Coding Workspace is no longer available: {error}"
                ),
                recoverable: true,
            })?;
        let requested_workspace = PathBuf::from(&request.workspace_path)
            .canonicalize()
            .map_err(|error| BridgeFailure {
                code: "invalid-workspace-path".to_owned(),
                message: format!("The requested Coding Workspace is unavailable: {error}"),
                recoverable: true,
            })?;
        if requested_workspace != coding_workspace {
            return Err(BridgeFailure {
                code: "invalid-workspace-acknowledgement".to_owned(),
                message: "The Orchestrator acknowledged a different Coding Workspace boundary."
                    .to_owned(),
                recoverable: true,
            });
        }
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pending_matches = state.pending_selection.as_ref().is_some_and(|pending| {
            pending.correlation_id == request.correlation_id
                && pending.workspace_path == request.workspace_path
                && pending.selection_mode == request.selection_mode
        });
        if !pending_matches {
            return Err(BridgeFailure {
                code: "workspace-selection-not-pending".to_owned(),
                message:
                    "The Coding Workspace acknowledgement does not match the reserved request."
                        .to_owned(),
                recoverable: true,
            });
        }
        state.coding_workspace = Some(coding_workspace);
        state.active_mission = None;
        state.revision = 1;
        state.known_missions = acknowledgement.known_missions.clone();
        state.missions.clear();
        state.mission_catalog = None;
        state.pending_selection = None;
        state.accepted_selection = Some(AcceptedWorkspaceSelection {
            correlation_id: request.correlation_id.clone(),
            workspace_path: request.workspace_path.clone(),
            selection_mode: request.selection_mode.clone(),
            acknowledgement: acknowledgement.clone(),
        });
        Ok(())
    }

    pub fn reserve_selection(
        &self,
        request: &CodingWorkspaceSelectionRequest,
    ) -> Result<Option<CodingWorkspaceAcknowledgement>, BridgeFailure> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(accepted) = state.accepted_selection.as_ref() {
            if accepted.correlation_id != request.correlation_id {
                return Err(BridgeFailure {
                    code: "workspace-already-selected".to_owned(),
                    message: "This Alfredo process already has an acknowledged Coding Workspace."
                        .to_owned(),
                    recoverable: false,
                });
            }
            if accepted.workspace_path != request.workspace_path
                || accepted.selection_mode != request.selection_mode
            {
                return Err(BridgeFailure {
                    code: "correlation-conflict".to_owned(),
                    message:
                        "The Coding Workspace correlation id was already used for a different boundary."
                            .to_owned(),
                    recoverable: false,
                });
            }
            let mut acknowledgement = accepted.acknowledgement.clone();
            acknowledgement.replayed = true;
            return Ok(Some(acknowledgement));
        }
        if state.coding_workspace.is_some() {
            return Err(BridgeFailure {
                code: "workspace-already-selected".to_owned(),
                message: "This Alfredo process already has an acknowledged Coding Workspace."
                    .to_owned(),
                recoverable: false,
            });
        }
        if let Some(pending) = state.pending_selection.as_ref() {
            if pending.correlation_id == request.correlation_id
                && (pending.workspace_path != request.workspace_path
                    || pending.selection_mode != request.selection_mode)
            {
                return Err(BridgeFailure {
                    code: "correlation-conflict".to_owned(),
                    message:
                        "The Coding Workspace correlation id is pending for a different boundary."
                            .to_owned(),
                    recoverable: false,
                });
            }
            return Err(BridgeFailure {
                code: "workspace-selection-pending".to_owned(),
                message: "A Coding Workspace selection is already waiting for acknowledgement."
                    .to_owned(),
                recoverable: true,
            });
        }
        state.pending_selection = Some(PendingWorkspaceSelection {
            correlation_id: request.correlation_id.clone(),
            workspace_path: request.workspace_path.clone(),
            selection_mode: request.selection_mode.clone(),
        });
        Ok(None)
    }

    pub fn release_selection(&self, request: &CodingWorkspaceSelectionRequest) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.pending_selection.as_ref().is_some_and(|pending| {
            pending.correlation_id == request.correlation_id
                && pending.workspace_path == request.workspace_path
                && pending.selection_mode == request.selection_mode
        }) {
            state.pending_selection = None;
        }
    }

    pub fn require_active_mission(&self) -> Result<(), BridgeFailure> {
        self.ensure_persistence_healthy()?;
        let state = self.state();
        if state.coding_workspace.is_none() {
            return Err(BridgeFailure {
                code: "coding-workspace-selection-required".to_owned(),
                message: "Choose or create a Coding Workspace before using Mission commands."
                    .to_owned(),
                recoverable: true,
            });
        }
        if state.active_mission.is_none() {
            return Err(BridgeFailure {
                code: "mission-selection-required".to_owned(),
                message:
                    "Choose Resume Mission or Start New Mission before using Mission commands."
                        .to_owned(),
                recoverable: true,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SkillCapability {
    pub name: String,
    pub description: String,
    pub source: String,
    pub invocation: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CommandCapability {
    pub name: String,
    pub usage: String,
    pub description: String,
    pub category: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AgentCapability {
    pub id: String,
    pub role: String,
    pub provider: String,
    pub runner: String,
    pub model: String,
    pub routing: String,
    pub availability: String,
    pub availability_reason: String,
    #[serde(default)]
    pub assignable: bool,
    #[serde(default)]
    pub delegate_only: bool,
    #[serde(default)]
    pub requires_approval: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AgentCapabilityCatalog {
    pub schema_version: u32,
    pub default_agent_id: String,
    pub skills: Vec<SkillCapability>,
    pub commands: Vec<CommandCapability>,
    pub agents: Vec<AgentCapability>,
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
            tracker_dir: backend_root.join(".scratch/alfredo-console-first-workstation-redesign"),
            issues_dir: None,
            runtime_root: std::env::temp_dir().join("albert-runtime"),
            mission_id: "alfredo-console-first-workstation-redesign".to_owned(),
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
        if let Some(tracker_dir) = std::env::var_os("ALBERT_TRACKER_DIR") {
            config.tracker_dir = PathBuf::from(tracker_dir);
        }
        if let Some(issues_dir) = std::env::var_os("ALBERT_ISSUES_DIR") {
            config.issues_dir = Some(PathBuf::from(issues_dir));
        }
        if let Ok(mission_id) = std::env::var("ALBERT_MISSION_ID") {
            if !mission_id.trim().is_empty() {
                config.mission_id = mission_id;
            }
        }
        if let Some(runtime_root) = std::env::var_os("ALFREDO_RUNTIME_ROOT") {
            config.runtime_root = PathBuf::from(runtime_root);
        }
        if let Some(agent_config) = std::env::var_os("ALFREDO_AGENT_CONFIG") {
            if !agent_config.is_empty() {
                config.agent_config = Some(PathBuf::from(agent_config));
            }
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

pub fn build_launch_context_with_binding(
    config: &BridgeConfig,
    binding: &WorkspaceBinding,
) -> AlfredoLaunchContext {
    let binding = binding.state();
    let coding_workspace = binding
        .coding_workspace
        .map(|workspace| workspace.to_string_lossy().into_owned());
    let active_mission = binding.active_mission;
    let phase = if coding_workspace.is_none() {
        "selection-required"
    } else if active_mission.is_none() {
        "mission-choice-required"
    } else {
        "workspace-ready"
    };
    AlfredoLaunchContext {
        schema_version: 1,
        selected_agent: std::env::var("ALFREDO_SELECTED_AGENT").unwrap_or_default(),
        selected_model: std::env::var("ALFREDO_SELECTED_MODEL").unwrap_or_default(),
        starting_location: bridge_starting_location(config)
            .to_string_lossy()
            .into_owned(),
        coding_workspace,
        active_mission,
        revision: binding.revision,
        known_missions: binding.known_missions,
        phase: phase.to_owned(),
        runtime_root: config.runtime_root.to_string_lossy().into_owned(),
        recent_workspaces: recent_workspaces(&config.runtime_root),
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn build_launch_context(config: &BridgeConfig) -> AlfredoLaunchContext {
    build_launch_context_with_binding(config, &WorkspaceBinding::from_environment())
}

fn bridge_starting_location(config: &BridgeConfig) -> PathBuf {
    std::env::var_os("ALFREDO_STARTING_LOCATION")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| {
            config
                .backend_root
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
        })
}

#[derive(Debug, Deserialize)]
struct PersistentResponse {
    id: String,
    success: bool,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Deserialize)]
struct PersistentAcceptance {
    id: String,
    accepted: bool,
}

struct BackendProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

fn python_backend_process(config: &BridgeConfig) -> Command {
    let mut command = Command::new(&config.python);
    command
        .current_dir(&config.backend_root)
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH");
    command
}

impl BackendProcess {
    fn start(config: &BridgeConfig) -> io::Result<Self> {
        let mut child = python_backend_process(config)
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
        if argv
            .first()
            .is_some_and(|value| value == "workspace-snapshot")
        {
            let mut acceptance_line = String::new();
            if self.stdout.read_line(&mut acceptance_line)? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Albert backend closed before accepting the request",
                ));
            }
            let acceptance: PersistentAcceptance = serde_json::from_str(&acceptance_line)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if acceptance.id != id || !acceptance.accepted {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Albert backend returned an invalid request acceptance",
                ));
            }
            record_native_performance_mark(
                "S5",
                "end",
                serde_json::json!({"outcome": "pass", "boundary": "python-request-accepted"}),
            )
            .map_err(|error| io::Error::other(error.message))?;
        }
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

fn decode_isolated_process_stream(bytes: Vec<u8>, stream_name: &str) -> io::Result<String> {
    String::from_utf8(bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Albert backend {stream_name} was not valid UTF-8: {error}"),
        )
    })
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

    /// Run a potentially long command outside the shared persistent backend.
    ///
    /// Controller inference, governed shell commands, and agent execution must
    /// not hold the supervisor mutex: snapshot polling and acknowledgements
    /// need to remain responsive while longer work is running.
    fn isolated_output(&mut self) -> io::Result<ProcessOutput> {
        let output = python_backend_process(self.config)
            .arg("-m")
            .arg("albert_mvp")
            .args(&self.argv)
            .stdin(Stdio::null())
            .output()?;
        Ok(ProcessOutput {
            success: output.status.success(),
            stdout: decode_isolated_process_stream(output.stdout, "stdout")?,
            stderr: decode_isolated_process_stream(output.stderr, "stderr")?,
        })
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
    pub last_activity_at: String,
    #[serde(default)]
    pub runner_started_at: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub task_title: String,
    #[serde(default)]
    pub operation_status: String,
    #[serde(default)]
    pub failure: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub commands_run: Vec<String>,
    #[serde(default)]
    pub test_results: String,
    #[serde(default)]
    pub risks: String,
    #[serde(default)]
    pub artifact_links: Vec<String>,
    #[serde(default)]
    pub launch_correlation_id: String,
    #[serde(default)]
    pub evidence_correlation_id: String,
    #[serde(default)]
    pub review_correlation_id: String,
    #[serde(default)]
    pub review_outcome: String,
    #[serde(default)]
    pub review_next_action: String,
    #[serde(default)]
    pub repair_action_available: bool,
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
    #[serde(default)]
    pub work_type: String,
    #[serde(default)]
    pub tracker_status: String,
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
pub struct WorkspaceScopeTarget {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkspaceScopeRequest {
    pub correlation_id: String,
    pub action_type: String,
    pub actor: String,
    pub expected_revision: u64,
    pub target: WorkspaceScopeTarget,
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
    #[serde(default)]
    pub scope_mission_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AgentConsoleResponseRequest {
    pub expected_revision: u64,
    pub message_id: String,
    pub scope_kind: String,
    pub scope_target: String,
    pub scope_label: String,
    #[serde(default)]
    pub scope_mission_id: String,
    pub agent_id: Option<String>,
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
    #[serde(default)]
    pub correlation_id: String,
    #[serde(default)]
    pub action_phase: String,
    #[serde(default)]
    pub action_outcome: String,
    #[serde(default)]
    pub action_message: String,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub enum AgentConsoleResponseIntent {
    #[serde(rename = "discussion")]
    Discussion,
    #[serde(rename = "coding-task")]
    CodingTask,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AgentConsoleResponseRoute {
    pub intent: AgentConsoleResponseIntent,
    pub task_request: String,
    pub acceptance_criteria: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AgentConsoleResponseProjection {
    pub message: AgentConsoleMessage,
    pub route: AgentConsoleResponseRoute,
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
pub struct ReviewDecisionTarget {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewDecisionRequest {
    pub correlation_id: String,
    pub action_type: String,
    pub actor: String,
    pub expected_revision: u64,
    pub target: ReviewDecisionTarget,
    #[serde(default)]
    pub mission_id: String,
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
    #[serde(default)]
    pub request_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AdditionalPathGrantRequestRecord {
    pub request_id: String,
    pub correlation_id: String,
    pub mission_id: String,
    pub path: String,
    pub access_level: String,
    pub duration_seconds: u64,
    pub requester: String,
    pub requested_at: String,
    pub reason: String,
    pub affected_action: String,
    pub status: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AdditionalPathGrantDenial {
    pub denial_id: String,
    pub correlation_id: String,
    pub request_id: String,
    pub path: String,
    pub access_level: String,
    pub duration_seconds: u64,
    pub denied_by: String,
    pub denied_at: String,
    pub reason: String,
    pub affected_action: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ShellTerminalProjection {
    pub schema_version: u32,
    pub revision: u64,
    pub commands: Vec<ShellTerminalCommandRecord>,
    pub grants: Vec<AdditionalPathGrant>,
    #[serde(default)]
    pub grant_denials: Vec<AdditionalPathGrantDenial>,
    #[serde(default)]
    pub path_grant_requests: Vec<AdditionalPathGrantRequestRecord>,
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
    #[serde(default)]
    pub request_id: String,
    pub expected_revision: u64,
    pub path: String,
    pub access_level: String,
    pub duration_seconds: u64,
    pub requester: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AdditionalPathGrantDenialRequest {
    pub correlation_id: String,
    pub request_id: String,
    pub expected_revision: u64,
    pub path: String,
    pub access_level: String,
    pub duration_seconds: u64,
    pub requester: String,
    pub reason: String,
    pub affected_action: String,
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
    #[serde(default)]
    pub proposal_correlation_id: String,
    #[serde(default)]
    pub decision_correlation_id: String,
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
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkstationActionTarget {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkstationActionRequest {
    pub correlation_id: String,
    pub action_type: String,
    pub actor: String,
    pub expected_revision: u64,
    pub target: WorkstationActionTarget,
    #[serde(default)]
    pub mission_id: String,
    #[serde(default)]
    pub issue_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub command_policy: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkstationActionAcknowledgement {
    pub correlation_id: String,
    pub outcome: String,
    pub revision: u64,
    pub action_type: String,
    pub issue_id: String,
    pub session_id: String,
    pub effect_summary: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkstationSessionRunRequest {
    pub session_id: String,
    #[serde(default)]
    pub mission_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkstationSessionRunProjection {
    pub schema_version: u32,
    pub mission_id: String,
    pub session_id: String,
    pub issue_id: String,
    pub status: String,
    pub runner_started_at: String,
    pub runner_ended_at: String,
    pub runner_exit_status: Option<i64>,
    pub evidence_valid: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SessionArtifactRequest {
    pub mission_id: String,
    pub session_id: String,
    pub artifact_ref: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SessionArtifactProjection {
    pub schema_version: u32,
    pub mission_id: String,
    pub session_id: String,
    pub artifact_id: String,
    pub label: String,
    pub media_type: String,
    pub content: String,
    pub byte_count: usize,
    pub content_limit_bytes: usize,
    pub truncated: bool,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BridgeFailure {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}

#[cfg(feature = "rust-orchestrator-prototype")]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RustPrototypePythonAuthority {
    pub revision: u64,
    pub workspace_path: String,
    pub workspace_status: String,
    pub active_mission_id: Option<String>,
    pub active_mission_title: Option<String>,
}

#[cfg(feature = "rust-orchestrator-prototype")]
#[derive(Debug, Deserialize, Serialize)]
pub struct RustOrchestratorPrototypeRequest {
    pub operation: String,
    #[serde(default)]
    pub state: Option<rust_orchestrator_prototype_model::PrototypeState>,
    #[serde(default)]
    pub action: Option<rust_orchestrator_prototype_model::Action>,
}

#[cfg(feature = "rust-orchestrator-prototype")]
#[derive(Debug, Serialize)]
pub struct RustOrchestratorPrototypeResponse {
    pub schema_version: u32,
    pub mode: String,
    pub state: rust_orchestrator_prototype_model::PrototypeState,
    pub receipt: Option<rust_orchestrator_prototype_model::EffectReceipt>,
    pub python_authority: RustPrototypePythonAuthority,
    pub python_unchanged_during_request: bool,
    pub canonical_writes_performed: bool,
    pub elapsed_micros: u128,
    pub message: String,
}

#[cfg(feature = "rust-orchestrator-prototype")]
fn rust_prototype_failure(
    failure: rust_orchestrator_prototype_model::DecisionFailure,
) -> BridgeFailure {
    BridgeFailure {
        code: failure.code,
        message: failure.message,
        recoverable: failure.recoverable,
    }
}

#[cfg(feature = "rust-orchestrator-prototype")]
fn rust_prototype_python_authority(snapshot: &WorkspaceSnapshot) -> RustPrototypePythonAuthority {
    RustPrototypePythonAuthority {
        revision: snapshot.revision,
        workspace_path: snapshot.workspace_session.workspace_path.clone(),
        workspace_status: snapshot.workspace_session.status.clone(),
        active_mission_id: snapshot
            .active_mission
            .as_ref()
            .map(|mission| mission.id.clone()),
        active_mission_title: snapshot
            .active_mission
            .as_ref()
            .map(|mission| mission.title.clone()),
    }
}

#[cfg(feature = "rust-orchestrator-prototype")]
fn rust_prototype_import_snapshot(
    config: &BridgeConfig,
    snapshot: &WorkspaceSnapshot,
) -> Result<rust_orchestrator_prototype_model::PrototypeState, BridgeFailure> {
    let encoded = serde_json::to_string(snapshot).map_err(|error| BridgeFailure {
        code: "contract-failure".to_owned(),
        message: format!(
            "Unable to encode the live Python snapshot for Rust shadow import: {error}"
        ),
        recoverable: true,
    })?;
    let starting_location = config
        .target_repo
        .parent()
        .unwrap_or(config.target_repo.as_path())
        .to_string_lossy()
        .into_owned();
    rust_orchestrator_prototype_model::import_legacy_v1(&encoded, starting_location)
        .map_err(rust_prototype_failure)
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
    record_native_performance_mark("S5", "start", serde_json::json!({"outcome": "pass"}))?;
    let output_result = configured_python_command(config, "workspace-snapshot").output();
    let output = output_result.map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Albert backend: {error}"),
        recoverable: true,
    })?;
    record_native_performance_mark("S7", "start", serde_json::json!({"outcome": "pass"}))?;
    let decoded = decode_snapshot_output(output);
    record_native_performance_mark(
        "S7",
        "end",
        serde_json::json!({
            "outcome": if decoded.is_ok() { "pass" } else { "fail" },
        }),
    )?;
    decoded
}

pub fn execute_agent_capabilities(
    config: &BridgeConfig,
) -> Result<AgentCapabilityCatalog, BridgeFailure> {
    let output = configured_python_command(config, "agent-capabilities")
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "agent capability catalog")
}

pub fn execute_coding_workspace_select(
    config: &BridgeConfig,
    starting_location: &std::path::Path,
    request: &CodingWorkspaceSelectionRequest,
) -> Result<CodingWorkspaceAcknowledgement, BridgeFailure> {
    let mut command = BackendCommand {
        config,
        argv: vec!["coding-workspace-select".to_owned()],
    };
    command
        .arg("--starting-location")
        .arg(starting_location)
        .arg("--workspace-path")
        .arg(&request.workspace_path)
        .arg("--selection-mode")
        .arg(&request.selection_mode)
        .arg("--runtime-root")
        .arg(&config.runtime_root)
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--forbidden-root")
        .arg(&config.backend_root)
        .arg("--forbidden-root")
        .arg(&config.runtime_root);
    if let Some(install_root) = std::env::var_os("ALFREDO_INSTALL_ROOT") {
        if !install_root.is_empty() {
            command
                .arg("--forbidden-root")
                .arg(PathBuf::from(install_root));
        }
    }
    let output = command.output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Alfredo backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(
        process_output(output),
        "Coding Workspace selection acknowledgement",
    )
}

pub fn execute_mission_choice(
    config: &BridgeConfig,
    starting_location: &Path,
    coding_workspace: &Path,
    request: &MissionChoiceRequest,
) -> Result<MissionChoiceAcknowledgement, BridgeFailure> {
    let mut command = BackendCommand {
        config,
        argv: vec!["mission-choice".to_owned()],
    };
    command
        .arg("--starting-location")
        .arg(starting_location)
        .arg("--coding-workspace")
        .arg(coding_workspace)
        .arg("--runtime-root")
        .arg(&config.runtime_root)
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--expected-revision")
        .arg(request.expected_revision.to_string())
        .arg("--choice")
        .arg(&request.choice)
        .arg("--mission-id")
        .arg(&request.mission_id)
        .arg("--mission-title")
        .arg(&request.mission_title);
    let output = command.output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Alfredo backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Mission choice acknowledgement")
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
        .arg("--action-type")
        .arg(&scope.action_type)
        .arg("--actor")
        .arg(&scope.actor)
        .arg("--expected-revision")
        .arg(scope.expected_revision.to_string())
        .arg("--target-kind")
        .arg(&scope.target.kind)
        .arg("--target-id")
        .arg(&scope.target.id)
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
    let mut command = configured_python_command(config, "agent-console-message");
    command
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
        .arg(&message.scope_label);
    if !message.scope_mission_id.is_empty() {
        command
            .arg("--scope-mission-id")
            .arg(&message.scope_mission_id);
    }
    let output = command.output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Albert backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Agent Console message")
}

pub fn execute_console_response(
    config: &BridgeConfig,
    request: &AgentConsoleResponseRequest,
) -> Result<AgentConsoleResponseProjection, BridgeFailure> {
    let mut command = configured_python_command(config, "agent-console-response");
    command
        .arg("--expected-revision")
        .arg(request.expected_revision.to_string())
        .arg("--message-id")
        .arg(&request.message_id)
        .arg("--scope-kind")
        .arg(&request.scope_kind)
        .arg("--scope-target")
        .arg(&request.scope_target)
        .arg("--scope-label")
        .arg(&request.scope_label);
    if !request.scope_mission_id.is_empty() {
        command
            .arg("--scope-mission-id")
            .arg(&request.scope_mission_id);
    }
    if let Some(agent_id) = &request.agent_id {
        if !agent_id.is_empty() {
            command.arg("--agent-id").arg(agent_id);
        }
    }
    let output = command.isolated_output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Albert backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Agent Console response")
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
        .arg("--action-type")
        .arg(&request.action_type)
        .arg("--actor")
        .arg(&request.actor)
        .arg("--target-kind")
        .arg(&request.target.kind)
        .arg("--target-id")
        .arg(&request.target.id)
        .arg("--session-id")
        .arg(&request.session_id)
        .arg("--decision")
        .arg(&request.decision)
        .arg("--reason")
        .arg(&request.reason);
    if !request.mission_id.is_empty() {
        command.arg("--review-mission-id").arg(&request.mission_id);
    }
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
    let output = command.isolated_output().map_err(|error| BridgeFailure {
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
        .isolated_output()
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
        .arg("--request-id")
        .arg(&request.request_id)
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

pub fn execute_additional_path_grant_deny(
    config: &BridgeConfig,
    request: &AdditionalPathGrantDenialRequest,
) -> Result<AdditionalPathGrantDenial, BridgeFailure> {
    let output = configured_python_command(config, "additional-path-grant-deny")
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--request-id")
        .arg(&request.request_id)
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
        .arg("--reason")
        .arg(&request.reason)
        .arg("--affected-action")
        .arg(&request.affected_action)
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the Albert backend: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "Additional Path Grant denial")
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
    record_native_performance_mark(
        "R3",
        "start",
        serde_json::json!({"outcome": if output.success { "pass" } else { "fail" }}),
    )?;
    decode_backend_json(process_output(output), "Workspace Queue acknowledgement")
}

pub fn execute_workstation_action(
    config: &BridgeConfig,
    request: &WorkstationActionRequest,
) -> Result<WorkstationActionAcknowledgement, BridgeFailure> {
    let mut command = configured_python_command(config, "workstation-action");
    command
        .arg("--correlation-id")
        .arg(&request.correlation_id)
        .arg("--expected-revision")
        .arg(request.expected_revision.to_string())
        .arg("--action-type")
        .arg(&request.action_type)
        .arg("--actor")
        .arg(&request.actor)
        .arg("--target-kind")
        .arg(&request.target.kind)
        .arg("--target-id")
        .arg(&request.target.id);
    if !request.mission_id.is_empty() {
        command.arg("--action-mission-id").arg(&request.mission_id);
    }
    if !request.issue_id.is_empty() {
        command.arg("--issue-id").arg(&request.issue_id);
    }
    if !request.session_id.is_empty() {
        command.arg("--session-id").arg(&request.session_id);
    }
    if !request.agent_id.is_empty() {
        command.arg("--agent").arg(&request.agent_id);
    }
    if !request.reason.is_empty() {
        command.arg("--reason").arg(&request.reason);
    }
    for path in &request.allowed_paths {
        command.arg("--allowed-path").arg(path);
    }
    for (backend_command, policy) in &request.command_policy {
        command
            .arg("--command-policy")
            .arg(format!("{backend_command}={policy}"));
    }
    let output = command.output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the Albert backend: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Workstation action acknowledgement")
}

pub fn execute_workstation_session_run(
    config: &BridgeConfig,
    request: &WorkstationSessionRunRequest,
) -> Result<WorkstationSessionRunProjection, BridgeFailure> {
    let mut command = configured_python_command(config, "workstation-session-run");
    command.arg("--session-id").arg(&request.session_id);
    if !request.mission_id.is_empty() {
        command.arg("--session-mission-id").arg(&request.mission_id);
    }
    let output = command.isolated_output().map_err(|error| BridgeFailure {
        code: "backend-startup-failure".to_owned(),
        message: format!("Unable to start the deferred Local Agent runner: {error}"),
        recoverable: true,
    })?;
    decode_backend_json(process_output(output), "Workstation session lifecycle")
}

pub fn execute_session_artifact(
    config: &BridgeConfig,
    request: &SessionArtifactRequest,
) -> Result<SessionArtifactProjection, BridgeFailure> {
    let output = configured_python_command(config, "session-artifact")
        .arg("--artifact-mission-id")
        .arg(&request.mission_id)
        .arg("--session-id")
        .arg(&request.session_id)
        .arg("--artifact-ref")
        .arg(&request.artifact_ref)
        .output()
        .map_err(|error| BridgeFailure {
            code: "backend-startup-failure".to_owned(),
            message: format!("Unable to start the bounded evidence reader: {error}"),
            recoverable: true,
        })?;
    decode_backend_json(process_output(output), "session artifact projection")
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
async fn run_blocking_bridge<T, F>(job: F) -> Result<T, BridgeFailure>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, BridgeFailure> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|error| BridgeFailure {
            code: "bridge-worker-failure".to_owned(),
            message: format!("The desktop bridge worker did not complete: {error}"),
            recoverable: true,
        })?
}

#[cfg(feature = "desktop")]
fn write_gui_smoke_ready(
    config: &BridgeConfig,
    context: &AlfredoLaunchContext,
) -> Result<(), BridgeFailure> {
    fs::create_dir_all(&config.runtime_root).map_err(|error| BridgeFailure {
        code: "gui-smoke-marker-failure".to_owned(),
        message: format!("Unable to create the Alfredo GUI smoke runtime directory: {error}"),
        recoverable: false,
    })?;
    let custom_marker = std::env::var("ALFREDO_WARM_READY_MARKER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let marker = custom_marker
        .clone()
        .unwrap_or_else(|| config.runtime_root.join("gui-smoke-ready.json"));
    if custom_marker.is_some() {
        if !marker.is_absolute() {
            return Err(BridgeFailure {
                code: "gui-smoke-marker-failure".to_owned(),
                message: "ALFREDO_WARM_READY_MARKER must be absolute".to_owned(),
                recoverable: false,
            });
        }
        marker
            .parent()
            .ok_or_else(|| BridgeFailure {
                code: "gui-smoke-marker-failure".to_owned(),
                message: "The warm readiness marker has no parent directory".to_owned(),
                recoverable: false,
            })?
            .canonicalize()
            .map_err(|error| BridgeFailure {
                code: "gui-smoke-marker-failure".to_owned(),
                message: format!("The warm readiness marker parent is unavailable: {error}"),
                recoverable: false,
            })?;
        if fs::symlink_metadata(&marker).is_ok() {
            return Err(BridgeFailure {
                code: "gui-smoke-marker-failure".to_owned(),
                message: "The warm readiness marker must be create-only".to_owned(),
                recoverable: false,
            });
        }
    }
    let desktop_session_id = std::env::var("ALFREDO_MEASUREMENT_DESKTOP_SESSION_ID")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if custom_marker.is_some() && desktop_session_id.is_none() {
        return Err(BridgeFailure {
            code: "gui-smoke-marker-failure".to_owned(),
            message: "ALFREDO_MEASUREMENT_DESKTOP_SESSION_ID is required for warm readiness"
                .to_owned(),
            recoverable: false,
        });
    }
    let payload = serde_json::json!({
        "schema_version": 1,
        "status": "ready",
        "process_id": std::process::id(),
        "phase": context.phase,
        "starting_location": context.starting_location,
        "coding_workspace": context.coding_workspace,
        "active_mission": context.active_mission,
        "backend_root": config.backend_root.to_string_lossy(),
        "desktop_session_id": desktop_session_id,
    });
    let encoded = serde_json::to_vec_pretty(&payload).expect("GUI smoke marker should serialize");
    let write_result = if custom_marker.is_some() {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
            .and_then(|mut output| output.write_all(&encoded))
    } else {
        fs::write(&marker, encoded)
    };
    write_result.map_err(|error| BridgeFailure {
        code: "gui-smoke-marker-failure".to_owned(),
        message: format!("Unable to write the Alfredo GUI smoke marker: {error}"),
        recoverable: false,
    })
}

#[cfg(feature = "desktop")]
fn record_gui_smoke_ready(
    config: &BridgeConfig,
    context: &AlfredoLaunchContext,
) -> Result<(), BridgeFailure> {
    if std::env::var("ALFREDO_GUI_SMOKE").as_deref() == Ok("1") {
        write_gui_smoke_ready(config, context)?;
    }
    Ok(())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn performance_mark(
    request: PerformanceMarkRequest,
) -> Result<PerformanceMarkAcknowledgement, BridgeFailure> {
    let Some((path, identity)) = performance_identity()? else {
        return Ok(PerformanceMarkAcknowledgement { recorded: false });
    };
    if !measurement_stage_matches_workflow(&request.stage, &identity.workflow) {
        return Ok(PerformanceMarkAcknowledgement { recorded: false });
    }
    if request.clock == "native" {
        let recorded = record_native_performance_mark(
            &request.stage,
            &request.boundary,
            if request.detail.is_null() {
                serde_json::json!({})
            } else {
                request.detail
            },
        )?;
        return Ok(PerformanceMarkAcknowledgement { recorded });
    }
    if request.clock != "frontend" {
        return Err(measurement_failure(
            "measurement clock must be native or frontend",
        ));
    }
    write_performance_mark(
        &path,
        &identity,
        "react",
        &request.clock_id,
        &request.stage,
        &request.boundary,
        &request.monotonic_ns,
        if request.detail.is_null() {
            serde_json::json!({})
        } else {
            request.detail
        },
    )?;
    Ok(PerformanceMarkAcknowledgement { recorded: true })
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_snapshot(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
) -> Result<WorkspaceSnapshot, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    record_native_performance_mark("S4", "end", serde_json::json!({"outcome": "pass"}))?;
    let snapshot = execute_snapshot(&bound_config)?;
    let context = build_launch_context_with_binding(config.inner(), binding.inner());
    record_gui_smoke_ready(config.inner(), &context)?;
    Ok(snapshot)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn agent_capabilities(
    config: tauri::State<'_, BridgeConfig>,
) -> Result<AgentCapabilityCatalog, BridgeFailure> {
    execute_agent_capabilities(config.inner())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn alfredo_launch_context(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
) -> Result<AlfredoLaunchContext, BridgeFailure> {
    binding.reload_from_persistence(config.inner())?;
    let context = build_launch_context_with_binding(config.inner(), binding.inner());
    record_gui_smoke_ready(config.inner(), &context)?;
    Ok(context)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn coding_workspace_select(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: CodingWorkspaceSelectionRequest,
) -> Result<CodingWorkspaceAcknowledgement, BridgeFailure> {
    if let Some(acknowledgement) = binding.reserve_selection(&request)? {
        return Ok(acknowledgement);
    }
    let acknowledgement = match execute_coding_workspace_select(
        config.inner(),
        &bridge_starting_location(config.inner()),
        &request,
    ) {
        Ok(acknowledgement) => acknowledgement,
        Err(error) => {
            binding.release_selection(&request);
            return Err(error);
        }
    };
    match binding.acknowledge(&request, &acknowledgement) {
        Ok(()) => {
            binding.reload_from_persistence(config.inner())?;
            Ok(acknowledgement)
        }
        Err(error) => {
            binding.release_selection(&request);
            Err(error)
        }
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn mission_choice(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: MissionChoiceRequest,
) -> Result<MissionChoiceAcknowledgement, BridgeFailure> {
    binding.require_workspace()?;
    let state = binding.state();
    let coding_workspace = state.coding_workspace.ok_or_else(|| BridgeFailure {
        code: "coding-workspace-selection-required".to_owned(),
        message: "Choose or create a Coding Workspace before choosing a Mission.".to_owned(),
        recoverable: true,
    })?;
    let acknowledgement = execute_mission_choice(
        config.inner(),
        &bridge_starting_location(config.inner()),
        &coding_workspace,
        &request,
    )?;
    binding.reload_from_persistence(config.inner())?;
    Ok(acknowledgement)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_updates(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    after_revision: u64,
) -> Result<WorkspaceUpdateBatch, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_updates(&bound_config, after_revision)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_action(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    action: WorkspaceActionRequest,
) -> Result<WorkspaceActionAcknowledgement, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_action(&bound_config, &action)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_scope(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    scope: WorkspaceScopeRequest,
) -> Result<WorkspaceActionAcknowledgement, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_scope(&bound_config, &scope)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_mission_switch(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: WorkspaceMissionSwitchRequest,
) -> Result<WorkspaceActionAcknowledgement, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_mission_switch(&bound_config, &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn agent_console_message(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    message: AgentConsoleMessageRequest,
) -> Result<AgentConsoleMessage, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_console_message(&bound_config, &message)
}

#[cfg(feature = "desktop")]
#[tauri::command]
async fn agent_console_response(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: AgentConsoleResponseRequest,
) -> Result<AgentConsoleResponseProjection, BridgeFailure> {
    let config = binding.bound_config(config.inner())?;
    run_blocking_bridge(move || execute_console_response(&config, &request)).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn agent_console_history(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
) -> Result<AgentConsoleHistory, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_console_history(&bound_config)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn working_context(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
) -> Result<WorkingContextProjection, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_working_context(&bound_config)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn working_context_curate(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: WorkingContextCurationRequest,
) -> Result<WorkingContextAcknowledgement, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_working_context_curate(&bound_config, &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn review_workspace(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
) -> Result<ReviewWorkspaceProjection, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_review_workspace(&bound_config)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn review_decision(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: ReviewDecisionRequest,
) -> Result<ReviewDecisionAcknowledgement, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_review_decision(&bound_config, &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn activity_journal(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    filters: ActivityJournalFilters,
) -> Result<ActivityJournalProjection, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_activity_journal(&bound_config, &filters)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn shell_terminal(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
) -> Result<ShellTerminalProjection, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_shell_terminal(&bound_config)
}

#[cfg(feature = "desktop")]
#[tauri::command]
async fn shell_terminal_submit(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: ShellTerminalCommandRequest,
) -> Result<ShellTerminalCommandResult, BridgeFailure> {
    let config = binding.bound_config(config.inner())?;
    run_blocking_bridge(move || execute_shell_terminal_submit(&config, &request)).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
async fn shell_terminal_decision(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: ShellTerminalDecisionRequest,
) -> Result<ShellTerminalCommandResult, BridgeFailure> {
    let config = binding.bound_config(config.inner())?;
    run_blocking_bridge(move || execute_shell_terminal_decision(&config, &request)).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn additional_path_grant_create(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: AdditionalPathGrantRequest,
) -> Result<AdditionalPathGrant, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_additional_path_grant_create(&bound_config, &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn additional_path_grant_deny(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: AdditionalPathGrantDenialRequest,
) -> Result<AdditionalPathGrantDenial, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_additional_path_grant_deny(&bound_config, &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_queue(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
) -> Result<WorkspaceQueueProjection, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_workspace_queue(&bound_config)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn ad_hoc_delegation_proposal(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: AdHocDelegationProposalRequest,
) -> Result<WorkspaceQueueAcknowledgement, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_ad_hoc_delegation_proposal(&bound_config, &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workspace_queue_decision(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: WorkspaceQueueDecisionRequest,
) -> Result<WorkspaceQueueAcknowledgement, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    record_native_performance_mark("R1", "end", serde_json::json!({"outcome": "pass"}))?;
    execute_workspace_queue_decision(&bound_config, &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn workstation_action(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: WorkstationActionRequest,
) -> Result<WorkstationActionAcknowledgement, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_workstation_action(&bound_config, &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
async fn workstation_session_run(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: WorkstationSessionRunRequest,
) -> Result<WorkstationSessionRunProjection, BridgeFailure> {
    let config = binding.bound_config(config.inner())?;
    run_blocking_bridge(move || execute_workstation_session_run(&config, &request)).await
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn session_artifact(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: SessionArtifactRequest,
) -> Result<SessionArtifactProjection, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_session_artifact(&bound_config, &request)
}

/// PROTOTYPE ONLY: exercise a Rust-owned decision slice against a read-only
/// import of the live Python snapshot. The command never writes canonical state.
#[cfg(all(feature = "desktop", feature = "rust-orchestrator-prototype"))]
#[tauri::command]
fn rust_orchestrator_prototype(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: RustOrchestratorPrototypeRequest,
) -> Result<RustOrchestratorPrototypeResponse, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    let started = Instant::now();
    let before_snapshot = execute_snapshot(&bound_config)?;
    let before_authority = rust_prototype_python_authority(&before_snapshot);
    let (state, receipt, message) = match request.operation.as_str() {
        "load" | "rollback" => (
            rust_prototype_import_snapshot(config.inner(), &before_snapshot)?,
            None,
            if request.operation == "rollback" {
                "Rust shadow state was discarded and re-imported from live Python authority."
                    .to_owned()
            } else {
                "Live Python authority was imported into the Rust shadow without a write."
                    .to_owned()
            },
        ),
        "reset-selection" => (
            rust_orchestrator_prototype_model::PrototypeState::selection_required(
                config
                    .target_repo
                    .parent()
                    .unwrap_or(config.target_repo.as_path())
                    .to_string_lossy()
                    .into_owned(),
            ),
            None,
            "Rust shadow returned to a distinct Starting Location; Python remained active."
                .to_owned(),
        ),
        "apply" => {
            let state = request.state.ok_or_else(|| BridgeFailure {
                code: "contract-failure".to_owned(),
                message: "Rust shadow apply requires the displayed prototype state.".to_owned(),
                recoverable: true,
            })?;
            let action = request.action.ok_or_else(|| BridgeFailure {
                code: "contract-failure".to_owned(),
                message: "Rust shadow apply requires one typed action.".to_owned(),
                recoverable: true,
            })?;
            let transition = rust_orchestrator_prototype_model::apply(&state, action)
                .map_err(rust_prototype_failure)?;
            let message = transition.receipt.message.clone();
            (transition.state, Some(transition.receipt), message)
        }
        _ => {
            return Err(BridgeFailure {
                code: "contract-failure".to_owned(),
                message: format!(
                    "Unknown Rust Orchestrator prototype operation: {}",
                    request.operation
                ),
                recoverable: true,
            });
        }
    };
    let after_snapshot = execute_snapshot(&bound_config)?;
    let after_authority = rust_prototype_python_authority(&after_snapshot);
    Ok(RustOrchestratorPrototypeResponse {
        schema_version: 1,
        mode: "rust-shadow".to_owned(),
        state,
        receipt,
        python_authority: after_authority.clone(),
        python_unchanged_during_request: before_authority == after_authority,
        canonical_writes_performed: false,
        elapsed_micros: started.elapsed().as_micros(),
        message,
    })
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn mission_drafts(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
) -> Result<MissionDraftProjection, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_mission_drafts(&bound_config)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn mission_draft_create(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: MissionDraftCreateRequest,
) -> Result<MissionDraftAcknowledgement, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_mission_draft_create(&bound_config, &request)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn mission_draft_decision(
    config: tauri::State<'_, BridgeConfig>,
    binding: tauri::State<'_, WorkspaceBinding>,
    request: MissionDraftDecisionRequest,
) -> Result<MissionDraftAcknowledgement, BridgeFailure> {
    let bound_config = binding.bound_config(config.inner())?;
    execute_mission_draft_decision(&bound_config, &request)
}

#[cfg(feature = "desktop")]
pub fn run() {
    let config = BridgeConfig::from_environment();
    let binding = WorkspaceBinding::from_config(&config);
    let builder = tauri::Builder::default().manage(config).manage(binding);
    #[cfg(not(feature = "rust-orchestrator-prototype"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        performance_mark,
        alfredo_launch_context,
        coding_workspace_select,
        mission_choice,
        workspace_snapshot,
        agent_capabilities,
        workspace_updates,
        workspace_action,
        workspace_scope,
        workspace_mission_switch,
        agent_console_message,
        agent_console_response,
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
        additional_path_grant_deny,
        workspace_queue,
        ad_hoc_delegation_proposal,
        workspace_queue_decision,
        workstation_action,
        workstation_session_run,
        session_artifact,
        mission_drafts,
        mission_draft_create,
        mission_draft_decision
    ]);
    #[cfg(feature = "rust-orchestrator-prototype")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        performance_mark,
        alfredo_launch_context,
        coding_workspace_select,
        mission_choice,
        workspace_snapshot,
        agent_capabilities,
        workspace_updates,
        workspace_action,
        workspace_scope,
        workspace_mission_switch,
        agent_console_message,
        agent_console_response,
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
        additional_path_grant_deny,
        workspace_queue,
        ad_hoc_delegation_proposal,
        workspace_queue_decision,
        workstation_action,
        workstation_session_run,
        session_artifact,
        rust_orchestrator_prototype,
        mission_drafts,
        mission_draft_create,
        mission_draft_decision
    ]);
    builder
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
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn isolated_process_stream_rejects_invalid_utf8() {
        let error = decode_isolated_process_stream(vec![0xff], "stdout")
            .expect_err("invalid UTF-8 must not enter typed bridge decoding");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("stdout"));
    }

    #[test]
    fn performance_writer_preserves_native_monotonic_sample_identity() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("alfredo-performance-rust-{unique}"));
        fs::create_dir_all(&root).expect("measurement fixture should be created");
        let path = root.join("raw.jsonl");
        let identity = PerformanceIdentity {
            run_id: "rust-run-001".to_owned(),
            sample_id: "rust-sample-001".to_owned(),
            cohort_id: "startup-process-cold".to_owned(),
            correlation_id: "startup-rust-001".to_owned(),
            fixture_id: "minimal-ready-v1".to_owned(),
            fixture_sha256: "c4cef5ccc043bb6476e6e07195f979ea722e20abf3890c10d06de8ad1628839b"
                .to_owned(),
            source_sha256: "a57b5956d8222b2ba001365fbff13e74350051ff13d5ba9dbe7fbeede203e721"
                .to_owned(),
            artifact_sha256: "6e68b39f69f605a9218e74dd48f091460ccf15768b922d5b7dfda9ec1e84c2ac"
                .to_owned(),
            variant: "python".to_owned(),
            workflow: "startup".to_owned(),
            mode: "process-cold".to_owned(),
            desktop_pid: None,
            desktop_session_id: None,
        };

        write_performance_mark(
            &path,
            &identity,
            "native-shell",
            "native-shell:123",
            "S5",
            "start",
            "1000000",
            serde_json::json!({"outcome": "pass"}),
        )
        .expect("start mark should be written");
        write_performance_mark(
            &path,
            &identity,
            "native-shell",
            "native-shell:123",
            "S5",
            "end",
            "2500000",
            serde_json::json!({"outcome": "pass"}),
        )
        .expect("end mark should be written");

        let records: Vec<serde_json::Value> = fs::read_to_string(&path)
            .expect("raw evidence should be readable")
            .lines()
            .map(|line| serde_json::from_str(line).expect("mark should be JSON"))
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["sample_id"], "rust-sample-001");
        assert_eq!(records[0]["stage"], "S5");
        assert_eq!(records[0]["boundary"], "start");
        assert_eq!(records[1]["boundary"], "end");
        assert_eq!(records[1]["monotonic_ns"], "2500000");

        fs::remove_dir_all(root).expect("measurement fixture should be removed");
    }

    #[test]
    fn performance_control_file_binds_a_warm_sample_to_the_desktop_process() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("alfredo-performance-control-{unique}"));
        fs::create_dir_all(&root).expect("measurement fixture should be created");
        let output = root.join("raw.jsonl");
        let control = root.join("control.json");
        fs::write(
            &control,
            serde_json::to_vec(&serde_json::json!({
                "jsonl_path": output,
                "run_id": "rust-run-001",
                "sample_id": "rust-warm-sample-001",
                "cohort_id": "queue-defer-process-warm",
                "correlation_id": "rust-warm-001",
                "fixture_id": "pending-ad-hoc-v1",
                "fixture_sha256": "c4cef5ccc043bb6476e6e07195f979ea722e20abf3890c10d06de8ad1628839b",
                "source_sha256": "a57b5956d8222b2ba001365fbff13e74350051ff13d5ba9dbe7fbeede203e721",
                "artifact_sha256": "6e68b39f69f605a9218e74dd48f091460ccf15768b922d5b7dfda9ec1e84c2ac",
                "variant": "python",
                "workflow": "queue-defer",
                "mode": "process-warm",
                "desktop_pid": 4101,
                "desktop_session_id": "desktop-one"
            }))
            .expect("measurement control should serialize"),
        )
        .expect("measurement control should be written");

        let (_, identity) = performance_identity_from_control(&control)
            .expect("measurement control should parse")
            .expect("measurement control should be armed");
        assert_eq!(identity.sample_id, "rust-warm-sample-001");
        assert_eq!(identity.desktop_pid, Some(4101));
        assert_eq!(identity.desktop_session_id.as_deref(), Some("desktop-one"));

        fs::remove_dir_all(root).expect("measurement fixture should be removed");
    }

    #[test]
    fn performance_command_is_an_explicit_noop_without_measurement_environment() {
        let acknowledgement = performance_mark(PerformanceMarkRequest {
            stage: "S3".to_owned(),
            boundary: "start".to_owned(),
            clock: "frontend".to_owned(),
            monotonic_ns: "1".to_owned(),
            clock_id: "frontend:test".to_owned(),
            detail: serde_json::json!({"outcome": "pass"}),
        })
        .expect("an ordinary desktop run must not require measurement");

        assert!(!acknowledgement.recorded);
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn gui_smoke_marker_records_frontend_and_backend_readiness_context() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("alfredo-gui-smoke-{unique}"));
        let workspace = root.join("workspace");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&workspace).expect("workspace fixture should be created");
        let mut config = BridgeConfig::for_repository(root.clone());
        config.target_repo = workspace.clone();
        config.runtime_root = runtime_root.clone();
        let context = AlfredoLaunchContext {
            schema_version: 1,
            selected_agent: "qwen3-14b".to_owned(),
            selected_model: "qwen3:14b".to_owned(),
            starting_location: workspace.to_string_lossy().into_owned(),
            coding_workspace: None,
            active_mission: None,
            phase: "selection-required".to_owned(),
            runtime_root: runtime_root.to_string_lossy().into_owned(),
            recent_workspaces: vec![],
            revision: 0,
            known_missions: vec![],
        };

        write_gui_smoke_ready(&config, &context).expect("GUI smoke marker should be written");
        let marker: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(runtime_root.join("gui-smoke-ready.json"))
                .expect("GUI smoke marker should be readable"),
        )
        .expect("GUI smoke marker should be valid JSON");

        assert_eq!(marker["schema_version"], 1);
        assert_eq!(marker["status"], "ready");
        assert_eq!(marker["phase"], "selection-required");
        assert_eq!(
            marker["starting_location"].as_str(),
            Some(workspace.to_string_lossy().as_ref())
        );
        assert!(marker["coding_workspace"].is_null());
        assert!(marker["active_mission"].is_null());
        assert_eq!(
            marker["backend_root"].as_str(),
            Some(root.to_string_lossy().as_ref())
        );
        assert!(marker["process_id"].as_u64().is_some());
        fs::remove_dir_all(root).expect("GUI smoke fixture should be removed");
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn workspace_binding_restores_the_exact_mission_from_canonical_journey_state() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("alfredo-mission-restore-{unique}"));
        let backend_root = root.join("backend");
        let workspace = root.join("workspace");
        let tracker = workspace.join(".alfredo/missions/modernization");
        let issues = tracker.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&backend_root).expect("backend fixture");
        fs::create_dir_all(&issues).expect("mission fixture");
        fs::create_dir_all(&runtime_root).expect("runtime fixture");
        fs::write(tracker.join("PRD.md"), "# Modernization Mission\n")
            .expect("Mission PRD fixture");
        let canonical_tracker = tracker.canonicalize().expect("tracker root");
        let canonical_issues = issues.canonicalize().expect("issues root");
        let mut config = BridgeConfig::for_repository(backend_root);
        config.runtime_root = runtime_root.clone();
        let starting_location = bridge_starting_location(&config);
        let catalog = runtime_root.join("workspace-mission-catalogs/catalog.json");
        fs::create_dir_all(catalog.parent().expect("catalog parent")).expect("catalog directory");
        fs::write(
            &catalog,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 1,
                "missions": [{
                    "mission_id": "modernization",
                    "tracker_dir": canonical_tracker.to_string_lossy(),
                    "issues_dir": canonical_issues.to_string_lossy()
                }]
            }))
            .expect("catalog should serialize"),
        )
        .expect("catalog should be persisted");
        let journey = serde_json::json!({
            "schema_version": 1,
            "sessions": [{
                "starting_location": starting_location.to_string_lossy(),
                "coding_workspace": workspace.to_string_lossy(),
                "revision": 2,
                "active_mission": "modernization",
                "missions": [{
                    "id": "modernization",
                    "title": "Modernization Mission",
                    "tracker_dir": canonical_tracker.to_string_lossy(),
                    "issues_dir": canonical_issues.to_string_lossy()
                }],
                "mission_catalog": catalog.to_string_lossy(),
                "selection": {
                    "correlation_id": "selection-1",
                    "selection_mode": "existing"
                },
                "receipts": {}
            }]
        });
        fs::write(
            runtime_root.join("workspace-sessions.json"),
            serde_json::to_vec(&journey).expect("journey should serialize"),
        )
        .expect("journey should be persisted");

        let binding = WorkspaceBinding::from_config(&config);
        let context = build_launch_context_with_binding(&config, &binding);
        assert_eq!(context.phase, "workspace-ready");
        assert_eq!(context.revision, 2);
        assert_eq!(context.active_mission.as_deref(), Some("modernization"));
        assert_eq!(context.known_missions[0].id, "modernization");

        let mut bound = binding
            .bound_config(&config)
            .expect("restored Mission should bind backend configuration");
        assert_eq!(
            bound.target_repo,
            workspace.canonicalize().expect("workspace root")
        );
        assert_eq!(bound.mission_id, "modernization");
        assert_eq!(bound.tracker_dir, canonical_tracker);
        assert_eq!(bound.mission_catalog, Some(catalog));
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repository root");
        bound.backend_root = repository_root.clone();
        bound.agent_config = Some(repository_root.join(".albert/agents.json"));
        let snapshot = execute_snapshot(&bound)
            .expect("catalog containing the Active Mission should produce a snapshot");
        assert_eq!(
            snapshot
                .active_mission
                .as_ref()
                .map(|mission| mission.id.as_str()),
            Some("modernization")
        );
        assert_eq!(snapshot.missions.len(), 1);
        assert_eq!(snapshot.missions[0].id, "modernization");
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn desktop_bridge_mission_choice_preserves_structured_failure_and_new_identity() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("alfredo-mission-choice-{unique}"));
        let starting_location = root.join("projects");
        let coding_workspace = starting_location.join("project");
        let tracker = coding_workspace.join(".agent/issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&tracker).expect("workspace fixture");
        fs::write(tracker.join("PRD.md"), "# Existing Mission\n").expect("PRD fixture");
        let git = Command::new("git")
            .args(["init", "--quiet"])
            .arg(&coding_workspace)
            .output()
            .expect("git should start");
        assert!(
            git.status.success(),
            "{}",
            String::from_utf8_lossy(&git.stderr)
        );
        let backend_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("backend root");
        let mut config = BridgeConfig::for_repository(backend_root);
        config.runtime_root = runtime_root;
        let request = CodingWorkspaceSelectionRequest {
            correlation_id: "mission-choice-selection".to_owned(),
            workspace_path: coding_workspace.to_string_lossy().into_owned(),
            selection_mode: "existing".to_owned(),
        };
        let binding = WorkspaceBinding::default();
        assert!(binding
            .reserve_selection(&request)
            .expect("selection should reserve")
            .is_none());
        let selection = execute_coding_workspace_select(&config, &starting_location, &request)
            .expect("workspace should be acknowledged");
        binding
            .acknowledge(&request, &selection)
            .expect("native binding should accept workspace");

        let missing = execute_mission_choice(
            &config,
            &starting_location,
            &coding_workspace,
            &MissionChoiceRequest {
                correlation_id: "mission-choice-missing".to_owned(),
                expected_revision: 1,
                choice: "resume".to_owned(),
                mission_id: "missing".to_owned(),
                mission_title: String::new(),
            },
        )
        .expect_err("unknown Mission must remain structured");
        assert_eq!(missing.code, "mission-not-found");

        let created = execute_mission_choice(
            &config,
            &starting_location,
            &coding_workspace,
            &MissionChoiceRequest {
                correlation_id: "mission-choice-new".to_owned(),
                expected_revision: 1,
                choice: "new".to_owned(),
                mission_id: "modernization".to_owned(),
                mission_title: "Modernization Mission".to_owned(),
            },
        )
        .expect("new Mission should be acknowledged");
        assert_eq!(created.choice, "new");
        assert_eq!(created.active_mission, "modernization");
        assert_eq!(created.revision, 2);
        assert!(coding_workspace
            .join(".alfredo/missions/modernization/PRD.md")
            .is_file());
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn desktop_bridge_resumes_the_exact_known_mission() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("alfredo-mission-resume-{unique}"));
        let starting_location = root.join("projects");
        let coding_workspace = starting_location.join("project");
        let tracker = coding_workspace.join(".agent/issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&tracker).expect("workspace fixture");
        fs::write(tracker.join("PRD.md"), "# Existing Mission\n").expect("PRD fixture");
        let git = Command::new("git")
            .args(["init", "--quiet"])
            .arg(&coding_workspace)
            .output()
            .expect("git should start");
        assert!(
            git.status.success(),
            "{}",
            String::from_utf8_lossy(&git.stderr)
        );
        let backend_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("backend root");
        let mut config = BridgeConfig::for_repository(backend_root);
        config.runtime_root = runtime_root;
        let selection_request = CodingWorkspaceSelectionRequest {
            correlation_id: "mission-resume-selection".to_owned(),
            workspace_path: coding_workspace.to_string_lossy().into_owned(),
            selection_mode: "existing".to_owned(),
        };

        execute_coding_workspace_select(&config, &starting_location, &selection_request)
            .expect("workspace should be acknowledged");
        let resumed = execute_mission_choice(
            &config,
            &starting_location,
            &coding_workspace,
            &MissionChoiceRequest {
                correlation_id: "mission-resume-choice".to_owned(),
                expected_revision: 1,
                choice: "resume".to_owned(),
                mission_id: "agent-issues".to_owned(),
                mission_title: String::new(),
            },
        )
        .expect("known Mission should resume");

        assert_eq!(resumed.choice, "resume");
        assert_eq!(resumed.active_mission, "agent-issues");
        assert_eq!(resumed.revision, 2);
        assert_eq!(resumed.missions.len(), 1);
        assert_eq!(resumed.missions[0].id, "agent-issues");
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn appimage_python_environment_does_not_poison_backend_snapshot() {
        const CHILD_PROCESS: &str = "ALFREDO_APPIMAGE_PYTHON_ENV_TEST_CHILD";
        if std::env::var(CHILD_PROCESS).as_deref() == Ok("1") {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("alfredo-appimage-python-env-{unique}"));
            let target_repo = root.join("workspace");
            let tracker_dir = root.join("tracker");
            let runtime_root = root.join("runtime");
            fs::create_dir_all(&target_repo).expect("workspace fixture should be created");
            fs::create_dir_all(&tracker_dir).expect("tracker fixture should be created");
            let backend_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("backend root should resolve");
            let config = BridgeConfig {
                python: "python3".to_owned(),
                backend_root,
                target_repo: target_repo.clone(),
                tracker_dir,
                issues_dir: None,
                runtime_root,
                mission_id: "appimage-python-environment".to_owned(),
                agent_config: None,
                mission_catalog: None,
            };

            let snapshot = execute_snapshot(&config)
                .expect("AppImage launch environment must not poison the host Python backend");
            assert_eq!(
                snapshot.workspace_session.workspace_path,
                target_repo.to_string_lossy()
            );
            shutdown_backends();
            fs::remove_dir_all(root)
                .expect("AppImage Python environment fixture should be removed");
            return;
        }

        let output = Command::new(std::env::current_exe().expect("test executable should resolve"))
            .arg("--exact")
            .arg("tests::appimage_python_environment_does_not_poison_backend_snapshot")
            .arg("--nocapture")
            .env(CHILD_PROCESS, "1")
            .env("PYTHONHOME", "/missing/appimage/usr")
            .env("PYTHONPATH", "/missing/appimage/usr/share/pyshared")
            .output()
            .expect("poisoned AppImage child test should launch");
        assert!(
            output.status.success(),
            "poisoned AppImage child failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn blocking_bridge_job_does_not_stall_followup_ipc_work() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let slow = tauri::async_runtime::spawn(run_blocking_bridge(move || {
            entered_tx
                .send(())
                .expect("slow bridge should announce that it started");
            release_rx
                .recv()
                .expect("test should release the slow bridge");
            Ok::<_, BridgeFailure>("slow")
        }));

        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("slow bridge should enter its blocking worker");

        let (fast_tx, fast_rx) = std::sync::mpsc::channel();
        let fast = tauri::async_runtime::spawn(async move {
            let result = run_blocking_bridge(|| Ok::<_, BridgeFailure>("snapshot")).await;
            fast_tx
                .send(result)
                .expect("fast bridge result receiver should remain available");
        });
        let fast_result = fast_rx.recv_timeout(Duration::from_secs(2));

        release_tx
            .send(())
            .expect("slow bridge receiver should remain available");
        let slow_result = tauri::async_runtime::block_on(slow)
            .expect("slow async task should join")
            .expect("slow blocking bridge should succeed");
        tauri::async_runtime::block_on(fast).expect("fast async task should join");

        assert_eq!(slow_result, "slow");
        assert_eq!(
            fast_result
                .expect("followup bridge work should finish before slow work is released")
                .expect("followup bridge work should succeed"),
            "snapshot"
        );
    }

    #[test]
    fn parses_agent_capability_catalog() {
        let output = ProcessOutput {
            success: true,
            stdout: r#"{
                "schema_version": 1,
                "default_agent_id": "qwen3-14b",
                "skills": [{
                    "name": "diagnose",
                    "description": "Diagnose hard bugs.",
                    "source": "/workspace/.agents/skills/diagnose/SKILL.md",
                    "invocation": "/use diagnose"
                }],
                "commands": [{
                    "name": "/run",
                    "usage": "/run <command>",
                    "description": "Run a governed command.",
                    "category": "execution"
                }],
                "agents": [{
                    "id": "qwen3-14b",
                    "role": "frontier",
                    "provider": "ollama",
                    "runner": "ollama",
                    "model": "qwen3:14b",
                    "routing": "controller",
                    "availability": "available",
                    "availability_reason": ""
                }]
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let catalog: AgentCapabilityCatalog =
            decode_backend_json(output, "agent capability catalog").expect("catalog should decode");

        assert_eq!(catalog.default_agent_id, "qwen3-14b");
        assert_eq!(catalog.skills[0].invocation, "/use diagnose");
        assert_eq!(catalog.commands[0].name, "/run");
    }

    #[test]
    fn desktop_bridge_acknowledges_an_exact_coding_workspace_without_a_mission() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("alfredo-workspace-select-{unique}"));
        let starting_location = root.join("projects");
        let coding_workspace = starting_location.join("existing");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&coding_workspace).expect("coding workspace");
        let git = Command::new("git")
            .args(["init", "--quiet"])
            .arg(&coding_workspace)
            .output()
            .expect("git should start");
        assert!(
            git.status.success(),
            "{}",
            String::from_utf8_lossy(&git.stderr)
        );
        let backend_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("backend root");
        let mut config = BridgeConfig::for_repository(backend_root);
        config.runtime_root = runtime_root;

        let request = CodingWorkspaceSelectionRequest {
            correlation_id: "workspace-select-rust-1".to_owned(),
            workspace_path: coding_workspace.to_string_lossy().into_owned(),
            selection_mode: "existing".to_owned(),
        };
        let binding = WorkspaceBinding::default();
        assert!(binding
            .reserve_selection(&request)
            .expect("the first selection should reserve the effect")
            .is_none());
        let pending = binding
            .reserve_selection(&request)
            .expect_err("a concurrent request must not repeat the effect");
        assert_eq!(pending.code, "workspace-selection-pending");
        let pending_conflict = binding
            .reserve_selection(&CodingWorkspaceSelectionRequest {
                correlation_id: request.correlation_id.clone(),
                workspace_path: starting_location
                    .join("different")
                    .to_string_lossy()
                    .into_owned(),
                selection_mode: "create".to_owned(),
            })
            .expect_err("a pending correlation must retain its exact boundary");
        assert_eq!(pending_conflict.code, "correlation-conflict");
        let acknowledgement =
            execute_coding_workspace_select(&config, &starting_location, &request)
                .expect("selection should be acknowledged");

        assert_eq!(acknowledgement.schema_version, 1);
        assert_eq!(acknowledgement.outcome, "acknowledged");
        assert_eq!(
            acknowledgement.coding_workspace,
            coding_workspace
                .canonicalize()
                .expect("canonical workspace")
                .to_string_lossy()
        );
        assert!(acknowledgement.active_mission.is_none());

        binding
            .acknowledge(&request, &acknowledgement)
            .expect("acknowledged workspace should become the native binding");
        let replay = binding
            .reserve_selection(&request)
            .expect("exact correlation replay should remain valid")
            .expect("accepted correlation should replay");
        assert!(replay.replayed);
        let changed_request = CodingWorkspaceSelectionRequest {
            correlation_id: request.correlation_id.clone(),
            workspace_path: starting_location
                .join("different")
                .to_string_lossy()
                .into_owned(),
            selection_mode: "create".to_owned(),
        };
        let conflict = binding
            .reserve_selection(&changed_request)
            .expect_err("changed selection boundary must not reuse a correlation");
        assert_eq!(conflict.code, "correlation-conflict");
        let second_selection = CodingWorkspaceSelectionRequest {
            correlation_id: "workspace-select-rust-2".to_owned(),
            workspace_path: starting_location
                .join("different")
                .to_string_lossy()
                .into_owned(),
            selection_mode: "create".to_owned(),
        };
        let already_selected = binding
            .reserve_selection(&second_selection)
            .expect_err("an acknowledged process binding must not be retargeted");
        assert_eq!(already_selected.code, "workspace-already-selected");
        let context = build_launch_context_with_binding(&config, &binding);
        assert_eq!(context.phase, "mission-choice-required");
        assert_eq!(
            context.coding_workspace.as_deref(),
            Some(
                coding_workspace
                    .canonicalize()
                    .expect("canonical workspace")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        let failure = binding
            .require_active_mission()
            .expect_err("Mission-qualified commands must remain blocked");
        assert_eq!(failure.code, "mission-selection-required");
        assert!(failure.recoverable);
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn parses_finished_workstation_session_lifecycle() {
        let output = ProcessOutput {
            success: true,
            stdout: r#"{
                "schema_version": 1,
                "mission_id": "command-deck",
                "session_id": "session-ISS-01-1",
                "issue_id": "ISS-01",
                "status": "evidence-ready",
                "runner_started_at": "2026-07-10T08:00:00Z",
                "runner_ended_at": "2026-07-10T08:00:01Z",
                "runner_exit_status": 0,
                "evidence_valid": true
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let lifecycle: WorkstationSessionRunProjection =
            decode_backend_json(output, "Workstation session lifecycle")
                .expect("session lifecycle should decode");

        assert_eq!(lifecycle.session_id, "session-ISS-01-1");
        assert_eq!(lifecycle.runner_exit_status, Some(0));
        assert!(lifecycle.evidence_valid);
    }

    #[test]
    fn parses_bounded_session_artifact_without_a_host_path() {
        let output = ProcessOutput {
            success: true,
            stdout: r#"{
                "schema_version": 1,
                "mission_id": "command-deck",
                "session_id": "session-ISS-01-1",
                "artifact_id": "review_diff",
                "label": "Review diff",
                "media_type": "text/x-diff",
                "content": "--- a/app.py\n+++ b/app.py\n+fixed\n",
                "byte_count": 36,
                "content_limit_bytes": 128000,
                "truncated": false
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let artifact: SessionArtifactProjection =
            decode_backend_json(output, "session artifact projection")
                .expect("session artifact should decode");

        assert_eq!(artifact.artifact_id, "review_diff");
        assert_eq!(artifact.media_type, "text/x-diff");
        assert!(artifact.content.contains("+fixed"));
        assert_eq!(artifact.content_limit_bytes, 128_000);
        assert!(!artifact.truncated);
    }

    #[test]
    fn parses_typed_controller_coding_task_route() {
        let output = ProcessOutput {
            success: true,
            stdout: r#"{
                "message": {
                    "message_id": "console-000002",
                    "sequence": 2,
                    "role": "assistant",
                    "content": "I can route that bounded task.",
                    "scope": {
                        "kind": "working-directory",
                        "target_id": "/workspace/albert",
                        "label": "albert",
                        "mission_id": null
                    },
                    "outcome": "model-commentary",
                    "source": "frontier-model"
                },
                "route": {
                    "intent": "coding-task",
                    "task_request": "Fix workspace polling.",
                    "acceptance_criteria": [
                        "Polling recovers after a transient failure."
                    ]
                }
            }"#
            .to_owned(),
            stderr: String::new(),
        };

        let response: AgentConsoleResponseProjection =
            decode_backend_json(output, "Agent Console response")
                .expect("controller response should decode");

        assert_eq!(response.message.message_id, "console-000002");
        assert_eq!(
            response.route.intent,
            AgentConsoleResponseIntent::CodingTask
        );
        assert_eq!(response.route.task_request, "Fix workspace polling.");
        assert_eq!(response.route.acceptance_criteria.len(), 1);
    }

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
                            "work_type": "AFK",
                            "tracker_status": "ready-for-agent",
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
                },
                "missions": [{
                    "id": "command-deck",
                    "title": "Command Deck Mission",
                    "issue_count": 2,
                    "is_active": true,
                    "sessions": [{
                        "session_id": "session-ISS-01-1",
                        "issue_id": "ISS-01",
                        "assigned_agent": "qwen-coder-local",
                        "status": "evidence-ready",
                        "last_activity_at": "2026-07-12T08:31:45+00:00",
                        "review_outcome": "Needs repair",
                        "review_next_action": "same-local-agent-repair",
                        "repair_action_available": true
                    }],
                    "attention": []
                }]
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
        assert_eq!(snapshot.mission_board.issue_slices[0].work_type, "AFK");
        assert_eq!(
            snapshot.mission_board.issue_slices[0].tracker_status,
            "ready-for-agent"
        );
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
        assert_eq!(
            snapshot.missions[0].sessions[0].review_outcome,
            "Needs repair"
        );
        assert_eq!(
            snapshot.missions[0].sessions[0].review_next_action,
            "same-local-agent-repair"
        );
        assert_eq!(
            snapshot.missions[0].sessions[0].last_activity_at,
            "2026-07-12T08:31:45+00:00"
        );
        assert!(snapshot.missions[0].sessions[0].repair_action_available);
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
    fn repository_config_targets_the_console_first_workstation_redesign() {
        let backend_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("backend root");

        let config = BridgeConfig::for_repository(backend_root.clone());

        assert_eq!(config.target_repo, backend_root);
        assert!(config
            .tracker_dir
            .ends_with(".scratch/alfredo-console-first-workstation-redesign"));
        assert!(config.issues_dir.is_none());
        assert_eq!(
            config.mission_id,
            "alfredo-console-first-workstation-redesign"
        );
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
    fn environment_registry_is_the_authority_for_desktop_capabilities() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("alfredo-bridge-registry-{unique}"));
        let agent_config = root.join("custom-agents.json");
        fs::create_dir_all(&root).expect("registry fixture root");
        fs::write(
            &agent_config,
            r#"{
                "agents": [{
                    "id": "custom-controller",
                    "role": "frontier",
                    "provider": "fake",
                    "runner": "fake",
                    "model": "deterministic-custom",
                    "routing": "controller"
                }]
            }"#,
        )
        .expect("custom registry");
        let prior_agent_config = std::env::var_os("ALFREDO_AGENT_CONFIG");
        std::env::set_var("ALFREDO_AGENT_CONFIG", &agent_config);

        let config = BridgeConfig::from_environment();

        if let Some(prior_agent_config) = prior_agent_config {
            std::env::set_var("ALFREDO_AGENT_CONFIG", prior_agent_config);
        } else {
            std::env::remove_var("ALFREDO_AGENT_CONFIG");
        }
        assert_eq!(config.agent_config.as_deref(), Some(agent_config.as_path()));
        let catalog = execute_agent_capabilities(&config)
            .expect("desktop capabilities should use the launcher registry");
        assert_eq!(catalog.default_agent_id, "custom-controller");
        assert_eq!(catalog.agents.len(), 1);
        assert_eq!(catalog.agents[0].id, "custom-controller");
        fs::remove_dir_all(root).expect("temporary registry fixture should be removed");
    }

    #[test]
    fn launch_context_surfaces_selection_required_starting_location() {
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
        std::env::set_var("ALFREDO_STARTING_LOCATION", &selected_workspace);
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
        std::env::remove_var("ALFREDO_STARTING_LOCATION");
        assert_eq!(context.schema_version, 1);
        assert_eq!(context.selected_agent, "qwen3.6-27b");
        assert_eq!(context.selected_model, "qwen3.6:27b");
        assert_eq!(
            context.starting_location,
            selected_workspace.to_string_lossy().to_string()
        );
        assert!(context.coding_workspace.is_none());
        assert!(context.active_mission.is_none());
        assert_eq!(context.phase, "selection-required");
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
                    "request_id": "path-grant-request-000001",
                    "path": "/external/docs",
                    "access_level": "read",
                    "duration_seconds": 900,
                    "granted_by": "mission-commander",
                    "granted_at": "2026-06-27T08:00:00Z",
                    "expires_at": "2026-06-27T08:15:00Z"
                }],
                "path_grant_requests": [{
                    "request_id": "path-grant-request-000001",
                    "correlation_id": "terminal-rust-1",
                    "mission_id": "command-deck",
                    "path": "/external/docs",
                    "access_level": "read",
                    "duration_seconds": 900,
                    "requester": "mission-commander",
                    "requested_at": "2026-06-27T07:59:00Z",
                    "reason": "External documentation requires an Additional Path Grant.",
                    "affected_action": "python3 docs/check.py",
                    "status": "granted"
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
        assert_eq!(
            projection.path_grant_requests[0].request_id,
            "path-grant-request-000001"
        );
        assert_eq!(projection.path_grant_requests[0].status, "granted");
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

        assert_eq!(
            result["status"], "completed",
            "Shell Terminal stderr: {}",
            result["stderr"]
        );
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
        assert_eq!(
            result.status, "completed",
            "Shell Terminal stderr: {}",
            result.stderr
        );
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
        assert_eq!(
            result.status, "completed",
            "Shell Terminal stderr: {}",
            result.stderr
        );
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
        let blocked = ShellTerminalCommandRequest {
            correlation_id: "terminal-path-grant-bridge-1".to_owned(),
            command: "python3 -m unittest --help".to_owned(),
            working_directory: config.target_repo.display().to_string(),
            requested_paths: vec![external_path.display().to_string()],
            requester: "mission-commander".to_owned(),
            access_level: "write".to_owned(),
        };
        execute_shell_terminal_submit(&config, &blocked)
            .expect_err("external command path should require typed authority");
        let pending_projection =
            execute_shell_terminal(&config).expect("typed grant request should inspect");
        let request_id = pending_projection.path_grant_requests[0].request_id.clone();
        let request = AdditionalPathGrantRequest {
            correlation_id: "path-grant-bridge-1".to_owned(),
            request_id: request_id.clone(),
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
        assert_eq!(grant.request_id, request_id);
        assert_eq!(projection.grants[0].grant_id, grant.grant_id);
        assert_eq!(projection.path_grant_requests[0].status, "granted");
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_denies_contextual_path_grant_through_python_backend() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-path-denial-{unique}"));
        let target_repo = root.join("target");
        let external_path = root.join("external-docs");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&external_path).expect("external path");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Path Grant Denial Bridge\n").expect("PRD");
        fs::write(
            issues_dir.join("01-terminal.md"),
            "Status: ready-for-agent\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nPath grant denial bridge.\n\n## Acceptance criteria\n\n- [ ] Denial is durable.\n\n## Blocked by\n\nNone - can start immediately\n",
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
            mission_id: "path-grant-denial-bridge".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };
        let request = AdditionalPathGrantDenialRequest {
            correlation_id: "path-grant-denial-bridge-1".to_owned(),
            request_id: "contextual-grant-bridge-1".to_owned(),
            expected_revision: 0,
            path: external_path.display().to_string(),
            access_level: "read".to_owned(),
            duration_seconds: 300,
            requester: "mission-commander".to_owned(),
            reason: "The blocked command requested external documentation.".to_owned(),
            affected_action: "python3 docs/check.py".to_owned(),
        };

        let denial = execute_additional_path_grant_deny(&config, &request)
            .expect("path grant request should be denied");
        let projection = execute_shell_terminal(&config).expect("terminal should inspect");
        let journal = execute_activity_journal(&config, &ActivityJournalFilters::default())
            .expect("journal should inspect");

        assert_eq!(denial.denial_id, "path-grant-denial-000001");
        assert_eq!(denial.request_id, "contextual-grant-bridge-1");
        assert!(projection.grants.is_empty());
        assert_eq!(projection.grant_denials[0].denial_id, denial.denial_id);
        assert_eq!(
            journal.entries[0].action_type,
            "additional-path-grant-denied"
        );
        assert_eq!(journal.entries[0].correlation_id, request.correlation_id);
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
                action_type: "conversation-scope-change".to_owned(),
                actor: "mission-commander".to_owned(),
                expected_revision: 1,
                target: WorkspaceScopeTarget {
                    kind: "conversation-scope".to_owned(),
                    id: "ISS-01".to_owned(),
                },
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
                scope_mission_id: "agent-console".to_owned(),
            },
        )
        .expect("message should append");
        let response = execute_console_response(
            &config,
            &AgentConsoleResponseRequest {
                expected_revision: 2,
                message_id: "console-000001".to_owned(),
                scope_kind: "issue-slice".to_owned(),
                scope_target: "ISS-01".to_owned(),
                scope_label: "Console".to_owned(),
                scope_mission_id: "agent-console".to_owned(),
                agent_id: None,
            },
        )
        .expect("controller response should append");

        let restored = execute_console_history(&config).expect("history should restore");

        assert_eq!(restored.messages.len(), 2);
        assert_eq!(restored.messages[0].message_id, "console-000001");
        assert_eq!(restored.messages[0].scope.kind, "issue-slice");
        assert_eq!(restored.messages[0].scope.target_id, "ISS-01");
        assert_eq!(response.message.role, "assistant");
        assert_eq!(response.message.outcome, "model-commentary");
        assert_eq!(response.message.action_outcome, "no-action");
        assert!(response
            .message
            .action_message
            .starts_with("No action taken."));
        assert!(response.message.correlation_id.is_empty());
        assert!(response.message.action_phase.is_empty());
        assert!(response
            .message
            .content
            .contains("Untrusted reply prose was not retained"));
        assert_eq!(
            response.route.intent,
            AgentConsoleResponseIntent::Discussion
        );
        assert!(response.route.task_request.is_empty());
        assert!(response.route.acceptance_criteria.is_empty());
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
        let target_repo = target_repo.canonicalize().expect("canonical target repo");
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
                scope_mission_id: String::new(),
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
        assert_eq!(queue.items[0].proposal_correlation_id, "tauri-ad-hoc-1");
        assert!(queue.items[0].decision_correlation_id.is_empty());
        assert_eq!(
            queue.items[0].proposed_changes["allowed_paths"][0],
            "docs/smoke-tests.md"
        );
        execute_workspace_queue_decision(
            &config,
            &WorkspaceQueueDecisionRequest {
                correlation_id: "tauri-ad-hoc-approve-1".to_owned(),
                action_type: "workspace-queue-decision".to_owned(),
                actor: "mission-commander".to_owned(),
                expected_revision: acknowledgement.revision,
                target: Some(WorkspaceQueueDecisionTarget {
                    kind: "workspace-queue-item".to_owned(),
                    id: acknowledgement.item_id.clone(),
                }),
                item_id: acknowledgement.item_id,
                decision: "approve".to_owned(),
                reason: "Approved for typed bridge verification.".to_owned(),
            },
        )
        .expect("ad hoc proposal should be approved");
        let approved_queue = execute_workspace_queue(&config).expect("queue should restore");
        let snapshot = execute_snapshot(&config).expect("session summary should restore");
        assert_eq!(
            approved_queue.items[0].decision_correlation_id,
            "tauri-ad-hoc-approve-1"
        );
        assert_eq!(
            snapshot.missions[0].sessions[0].launch_correlation_id,
            "tauri-ad-hoc-approve-1"
        );
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_submits_workstation_launch_action() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-workstation-action-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Workstation Action Bridge\n").expect("PRD");
        fs::write(
            issues_dir.join("01-launch.md"),
            "Status: approved\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nLaunch through the workstation bridge.\n\n## Acceptance criteria\n\n- [ ] Launch acknowledgement is decoded.\n\n## Blocked by\n\nNone - can start immediately\n",
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
            mission_id: "workstation-action-bridge".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };

        let acknowledgement = execute_workstation_action(
            &config,
            &WorkstationActionRequest {
                correlation_id: "tauri-workstation-launch-1".to_owned(),
                action_type: "issue-launch".to_owned(),
                actor: "mission-commander".to_owned(),
                expected_revision: 1,
                target: WorkstationActionTarget {
                    kind: "issue-slice".to_owned(),
                    id: "ISS-01".to_owned(),
                },
                mission_id: "workstation-action-bridge".to_owned(),
                issue_id: "ISS-01".to_owned(),
                session_id: String::new(),
                agent_id: String::new(),
                reason: String::new(),
                allowed_paths: vec!["src".to_owned()],
                command_policy: std::collections::BTreeMap::new(),
            },
        )
        .expect("workstation action should be acknowledged");
        let snapshot = execute_snapshot(&config).expect("snapshot should inspect");

        assert_eq!(acknowledgement.action_type, "issue-launch");
        assert_eq!(acknowledgement.issue_id, "ISS-01");
        assert_eq!(acknowledgement.session_id, "session-ISS-01-1");
        assert_eq!(acknowledgement.revision, 2);
        assert_eq!(
            snapshot.missions[0].sessions[0].session_id,
            "session-ISS-01-1"
        );
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[test]
    fn desktop_bridge_submits_review_decision_metadata_and_rejects_mismatch() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("albert-tauri-review-decision-{unique}"));
        let target_repo = root.join("target");
        let tracker_dir = root.join("tracker");
        let issues_dir = tracker_dir.join("issues");
        let runtime_root = root.join("runtime");
        fs::create_dir_all(&target_repo).expect("target repo");
        fs::create_dir_all(&issues_dir).expect("issues dir");
        fs::write(tracker_dir.join("PRD.md"), "# Review Decision Bridge\n").expect("PRD");
        fs::write(
            issues_dir.join("01-review.md"),
            "Status: approved\nType: AFK\n\n## Parent\n\nPRD.md\n\n## What to build\n\nReview through the desktop bridge.\n\n## Acceptance criteria\n\n- [ ] Review metadata reaches the backend.\n\n## Blocked by\n\nNone - can start immediately\n",
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
            mission_id: "review-decision-bridge".to_owned(),
            agent_config: None,
            mission_catalog: None,
        };

        let launch = execute_workstation_action(
            &config,
            &WorkstationActionRequest {
                correlation_id: "tauri-review-launch-1".to_owned(),
                action_type: "issue-launch".to_owned(),
                actor: "mission-commander".to_owned(),
                expected_revision: 1,
                target: WorkstationActionTarget {
                    kind: "issue-slice".to_owned(),
                    id: "ISS-01".to_owned(),
                },
                mission_id: "review-decision-bridge".to_owned(),
                issue_id: "ISS-01".to_owned(),
                session_id: String::new(),
                agent_id: String::new(),
                reason: String::new(),
                allowed_paths: vec!["src".to_owned()],
                command_policy: std::collections::BTreeMap::new(),
            },
        )
        .expect("launch should create a queued session");
        let cancelled = execute_workstation_action(
            &config,
            &WorkstationActionRequest {
                correlation_id: "tauri-review-cancel-1".to_owned(),
                action_type: "session-cancel".to_owned(),
                actor: "mission-commander".to_owned(),
                expected_revision: launch.revision,
                target: WorkstationActionTarget {
                    kind: "agent-session".to_owned(),
                    id: launch.session_id.clone(),
                },
                mission_id: "review-decision-bridge".to_owned(),
                issue_id: "ISS-01".to_owned(),
                session_id: launch.session_id.clone(),
                agent_id: String::new(),
                reason: "Create terminal evidence for the bridge review.".to_owned(),
                allowed_paths: Vec::new(),
                command_policy: std::collections::BTreeMap::new(),
            },
        )
        .expect("cancel should make the session reviewable");
        let review = execute_review_decision(
            &config,
            &ReviewDecisionRequest {
                correlation_id: "tauri-review-repair-1".to_owned(),
                action_type: "review-decision".to_owned(),
                actor: "mission-commander".to_owned(),
                expected_revision: cancelled.revision,
                target: ReviewDecisionTarget {
                    kind: "agent-session".to_owned(),
                    id: launch.session_id.clone(),
                },
                mission_id: "review-decision-bridge".to_owned(),
                session_id: launch.session_id.clone(),
                decision: "repair".to_owned(),
                reason: "Needs focused repair.".to_owned(),
                failure_type: None,
            },
        )
        .expect("review decision should be acknowledged");

        let failure = execute_review_decision(
            &config,
            &ReviewDecisionRequest {
                correlation_id: "tauri-review-target-mismatch-1".to_owned(),
                action_type: "review-decision".to_owned(),
                actor: "mission-commander".to_owned(),
                expected_revision: review.revision,
                target: ReviewDecisionTarget {
                    kind: "agent-session".to_owned(),
                    id: "session-other".to_owned(),
                },
                mission_id: "review-decision-bridge".to_owned(),
                session_id: launch.session_id,
                decision: "repair".to_owned(),
                reason: "Needs focused repair.".to_owned(),
                failure_type: None,
            },
        )
        .expect_err("mismatched target metadata should be rejected");
        let snapshot = execute_snapshot(&config).expect("snapshot should inspect");

        assert_eq!(review.review_outcome, "Needs repair");
        assert_eq!(review.session_id, "session-ISS-01-1");
        assert_eq!(failure.code, "backend-startup-failure");
        assert!(failure
            .message
            .contains("Review decision action target id must match session id"));
        assert_eq!(snapshot.revision, review.revision);
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
                scope_mission_id: String::new(),
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
                scope_mission_id: String::new(),
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
                scope_mission_id: String::new(),
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
