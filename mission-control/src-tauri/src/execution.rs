use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const EXECUTION_SCHEMA_VERSION: u32 = 1;
const MAX_REQUEST_ID_BYTES: usize = 256;
const MAX_ARGUMENT_BYTES: usize = 128_000;
const MAX_INPUT_BYTES: usize = 96_000;
const MAX_OUTPUT_BYTES: usize = 8_000_000;
const MAX_TIMEOUT_SECONDS: f64 = 3_600.0;
const MAX_ENVIRONMENT_ENTRIES: usize = 128;
const MAX_ENVIRONMENT_VALUE_BYTES: usize = 16 * 1024;
const MAX_ENVIRONMENT_TOTAL_BYTES: usize = 64 * 1024;
const MAX_PATH_ENTRIES: usize = 256;
const MAX_ADDRESS_SPACE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_FILE_SIZE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_OPEN_FILE_LIMIT: u64 = 65_536;
const MAX_PROCESS_COUNT_LIMIT: u64 = 4_096;
const OUTPUT_MESSAGE_RESERVE: usize = 256;
pub const MAX_PROTOCOL_LINE_BYTES: usize = 16 * 1024 * 1024;
const TRUSTED_EXECUTABLE_ROOTS: [&str; 4] = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"];
const IMPLICIT_READONLY_ROOTS: [&str; 6] = ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"];
const PROTECTED_READONLY_ROOTS: [&str; 7] =
    ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/dev"];
const ALLOWED_ENVIRONMENT_KEYS: [&str; 19] = [
    "CI",
    "COLORTERM",
    "HOME",
    "LANG",
    "LC_ALL",
    "LOGNAME",
    "NO_COLOR",
    "OLLAMA_HOST",
    "PATH",
    "PYTHONPATH",
    "SHELL",
    "TERM",
    "TMPDIR",
    "USER",
    "PIP_CACHE_DIR",
    "PYTHONDONTWRITEBYTECODE",
    "npm_config_cache",
    "ALBERT_SESSION_ID",
    "ALBERT_TASK_PACKET",
];

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredFailure {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}

impl StructuredFailure {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            recoverable: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimits {
    pub timeout_seconds: f64,
    pub output_limit_bytes: usize,
    pub address_space_bytes: u64,
    pub file_size_bytes: u64,
    pub open_file_limit: u64,
    pub process_count_limit: u64,
    pub descendant_grace_seconds: f64,
}

impl ExecutionLimits {
    fn validate(&self) -> Result<(), StructuredFailure> {
        if !self.timeout_seconds.is_finite()
            || self.timeout_seconds <= 0.0
            || self.timeout_seconds > MAX_TIMEOUT_SECONDS
        {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution timeout is outside the bounded range",
            ));
        }
        if self.output_limit_bytes == 0 || self.output_limit_bytes > MAX_OUTPUT_BYTES {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution output limit is outside the bounded range",
            ));
        }
        if self.address_space_bytes == 0
            || self.file_size_bytes == 0
            || self.open_file_limit == 0
            || self.process_count_limit == 0
        {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution resource limits must be positive",
            ));
        }
        if self.address_space_bytes > MAX_ADDRESS_SPACE_BYTES
            || self.file_size_bytes > MAX_FILE_SIZE_BYTES
            || self.open_file_limit > MAX_OPEN_FILE_LIMIT
            || self.process_count_limit > MAX_PROCESS_COUNT_LIMIT
        {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution resource limits exceed the bounded contract",
            ));
        }
        if !self.descendant_grace_seconds.is_finite()
            || self.descendant_grace_seconds < 0.0
            || self.descendant_grace_seconds > 60.0
        {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution descendant grace is outside the bounded range",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSandbox {
    pub mode: String,
    pub readable_roots: Vec<String>,
    pub writable_roots: Vec<String>,
    pub readonly_bindings: Vec<(String, String)>,
}

impl ExecutionSandbox {
    fn validate(&self) -> Result<(), StructuredFailure> {
        if self.mode != "bubblewrap" && self.mode != "none" {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution sandbox mode is invalid",
            ));
        }
        if self.mode != "bubblewrap" {
            return Err(StructuredFailure::new(
                "contract-failure",
                "host effects require the Bubblewrap sandbox",
            ));
        }
        validate_unique_paths(&self.readable_roots, "execution readable roots")?;
        validate_unique_paths(&self.writable_roots, "execution writable roots")?;
        if self.readonly_bindings.len() > MAX_PATH_ENTRIES {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution readonly bindings are too numerous",
            ));
        }
        for (source, destination) in &self.readonly_bindings {
            canonical_path(source, "execution readonly source")?;
            canonical_path(destination, "execution readonly destination")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ExecutionAuthority {
    LocalAgent {
        mission_id: String,
        session_id: String,
        session_revision: u64,
        runner_operation_id: String,
        worktree_identity: String,
        allowed_paths: Vec<String>,
    },
    Shell {
        mission_id: String,
        command_id: String,
        correlation_id: String,
        command: String,
        classification: String,
        requester: String,
        working_directory: String,
        requested_paths: Vec<String>,
        access_level: String,
        approval_actor: String,
    },
}

impl ExecutionAuthority {
    fn kind(&self) -> &'static str {
        match self {
            Self::LocalAgent { .. } => "local-agent",
            Self::Shell { .. } => "shell",
        }
    }

    fn validate(&self) -> Result<(), StructuredFailure> {
        match self {
            Self::LocalAgent {
                mission_id,
                session_id,
                session_revision: _,
                runner_operation_id,
                worktree_identity,
                allowed_paths,
            } => {
                validate_identity(mission_id, "Local Agent Mission identity")?;
                validate_identity(session_id, "Local Agent session identity")?;
                validate_identity(runner_operation_id, "Local Agent runner operation")?;
                validate_identity(worktree_identity, "Local Agent Worktree Identity")?;
                if allowed_paths.len() > MAX_PATH_ENTRIES {
                    return Err(StructuredFailure::new(
                        "contract-failure",
                        "Local Agent allowed paths are too numerous",
                    ));
                }
                for value in allowed_paths {
                    let path = Path::new(value);
                    if path.is_absolute() || value.is_empty() || value.contains('\0') {
                        return Err(StructuredFailure::new(
                            "contract-failure",
                            "Local Agent allowed paths must be relative",
                        ));
                    }
                    if path
                        .components()
                        .any(|component| component == Component::ParentDir)
                    {
                        return Err(StructuredFailure::new(
                            "contract-failure",
                            "Local Agent allowed paths must not escape the worktree",
                        ));
                    }
                }
                if allowed_paths.iter().enumerate().any(|(index, value)| {
                    allowed_paths[index + 1..]
                        .iter()
                        .any(|other| other == value)
                }) {
                    return Err(StructuredFailure::new(
                        "contract-failure",
                        "Local Agent allowed paths must be unique",
                    ));
                }
            }
            Self::Shell {
                mission_id,
                command_id,
                correlation_id,
                command,
                classification,
                requester,
                working_directory,
                requested_paths,
                access_level,
                approval_actor,
            } => {
                validate_identity(mission_id, "Shell Mission identity")?;
                validate_identity(command_id, "Shell command identity")?;
                validate_identity(correlation_id, "Shell correlation identity")?;
                if command.trim().is_empty() || command.contains('\0') {
                    return Err(StructuredFailure::new(
                        "contract-failure",
                        "Shell command text is invalid",
                    ));
                }
                if !matches!(
                    classification.as_str(),
                    "auto-allowed" | "frontier-approvable" | "human-required"
                ) {
                    return Err(StructuredFailure::new(
                        "contract-failure",
                        "Shell command classification is invalid",
                    ));
                }
                if requester.trim().is_empty() || approval_actor.contains('\0') {
                    return Err(StructuredFailure::new(
                        "contract-failure",
                        "Shell requester is invalid",
                    ));
                }
                canonical_path(working_directory, "Shell working directory")?;
                validate_absolute_paths(requested_paths, "Shell requested paths")?;
                if access_level != "read" && access_level != "write" {
                    return Err(StructuredFailure::new(
                        "contract-failure",
                        "Shell access level is invalid",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRequest {
    pub schema_version: u32,
    pub request_id: String,
    pub effect: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub authority: ExecutionAuthority,
    pub limits: ExecutionLimits,
    pub sandbox: ExecutionSandbox,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub input_text: Option<String>,
    #[serde(default)]
    pub input_sha256: Option<String>,
    #[serde(default)]
    pub shell: bool,
}

impl ExecutionRequest {
    pub fn from_value(value: Value) -> Result<Self, StructuredFailure> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            StructuredFailure::new(
                "contract-failure",
                format!("execution request could not be decoded: {error}"),
            )
        })?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), StructuredFailure> {
        if self.schema_version != EXECUTION_SCHEMA_VERSION {
            return Err(StructuredFailure::new(
                "contract-failure",
                "unsupported execution request schema",
            ));
        }
        validate_identity(&self.request_id, "execution request identity")?;
        if self.effect != "local-agent" && self.effect != "shell" {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution effect kind is invalid",
            ));
        }
        if self.shell {
            return Err(StructuredFailure::new(
                "contract-failure",
                "shell execution is not allowed; argv is required",
            ));
        }
        if self.argv.is_empty() {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution argv must not be empty",
            ));
        }
        for argument in &self.argv {
            if argument.is_empty() || argument.contains('\0') {
                return Err(StructuredFailure::new(
                    "contract-failure",
                    "execution argv contains an invalid argument",
                ));
            }
            if argument.len() > MAX_ARGUMENT_BYTES {
                return Err(StructuredFailure::new(
                    "contract-failure",
                    "execution argv argument is too long",
                ));
            }
        }
        canonical_path(&self.working_directory, "execution working directory")?;
        self.authority.validate()?;
        if self.effect != self.authority.kind() {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution authority kind does not match effect",
            ));
        }
        self.limits.validate()?;
        self.sandbox.validate()?;
        if self.environment.len() > MAX_ENVIRONMENT_ENTRIES {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution environment is too large",
            ));
        }
        let mut environment_bytes: usize = 0;
        for (key, value) in &self.environment {
            if key.is_empty()
                || !key.chars().enumerate().all(|(index, character)| {
                    (index == 0 && (character == '_' || character.is_ascii_alphabetic()))
                        || (index > 0 && (character == '_' || character.is_ascii_alphanumeric()))
                })
                || value.contains('\0')
                || !ALLOWED_ENVIRONMENT_KEYS.contains(&key.as_str())
                || value.as_bytes().len() > MAX_ENVIRONMENT_VALUE_BYTES
            {
                return Err(StructuredFailure::new(
                    "contract-failure",
                    "execution environment is invalid",
                ));
            }
            environment_bytes = environment_bytes
                .saturating_add(key.as_bytes().len())
                .saturating_add(value.as_bytes().len());
            if environment_bytes > MAX_ENVIRONMENT_TOTAL_BYTES {
                return Err(StructuredFailure::new(
                    "contract-failure",
                    "execution environment exceeds the bounded size",
                ));
            }
        }
        let input_digest = sha256_text(self.input_text.as_deref().unwrap_or(""));
        if let Some(input) = &self.input_text {
            if input.contains('\0') || input.as_bytes().len() > MAX_INPUT_BYTES {
                return Err(StructuredFailure::new(
                    "contract-failure",
                    "execution input is invalid or exceeds the bounded size",
                ));
            }
        }
        if let Some(expected) = &self.input_sha256 {
            if expected != &input_digest {
                return Err(StructuredFailure::new(
                    "contract-failure",
                    "execution input digest is invalid",
                ));
            }
        }
        Ok(())
    }

    pub fn request_digest(&self) -> Result<String, StructuredFailure> {
        self.validate()?;
        let mut value = serde_json::to_value(self).map_err(|error| {
            StructuredFailure::new(
                "contract-failure",
                format!("execution request could not be encoded: {error}"),
            )
        })?;
        if let Value::Object(object) = &mut value {
            object.insert(
                "input_sha256".to_owned(),
                Value::String(sha256_text(self.input_text.as_deref().unwrap_or(""))),
            );
            object.insert("shell".to_owned(), Value::Bool(false));
            object.remove("input_text");
        }
        Ok(sha256_text(&canonical_json(&value)))
    }

    pub fn to_value(&self, include_input: bool) -> Result<Value, StructuredFailure> {
        self.validate()?;
        let mut value = serde_json::to_value(self).map_err(|error| {
            StructuredFailure::new(
                "contract-failure",
                format!("execution request could not be encoded: {error}"),
            )
        })?;
        if let Value::Object(object) = &mut value {
            object.insert(
                "input_sha256".to_owned(),
                Value::String(sha256_text(self.input_text.as_deref().unwrap_or(""))),
            );
            object.insert("shell".to_owned(), Value::Bool(false));
            if !include_input {
                object.remove("input_text");
            }
        }
        Ok(value)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReceipt {
    pub schema_version: u32,
    pub request_id: String,
    pub request_digest: String,
    pub effect: String,
    pub status: String,
    pub started_at: String,
    pub ended_at: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    pub effect_started: bool,
    pub reconciliation_required: bool,
    pub error_code: String,
    pub error_message: String,
    pub receipt_id: String,
    pub owner_pid: Option<u32>,
    pub owner_identity: String,
    pub process_pid: Option<u32>,
    pub process_identity: String,
    pub provider: String,
}

impl ExecutionReceipt {
    fn validate(&self) -> Result<(), StructuredFailure> {
        if self.schema_version != EXECUTION_SCHEMA_VERSION {
            return Err(StructuredFailure::new(
                "contract-failure",
                "unsupported execution receipt schema",
            ));
        }
        validate_identity(&self.request_id, "execution receipt request identity")?;
        validate_digest(&self.request_digest, "execution receipt request digest")?;
        if !matches!(
            self.status.as_str(),
            "executing"
                | "completed"
                | "failed"
                | "cancelled"
                | "timed-out"
                | "output-limit"
                | "start-failed"
                | "outcome-unknown"
        ) {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution receipt status is invalid",
            ));
        }
        if self.effect != "local-agent" && self.effect != "shell" {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution receipt effect is invalid",
            ));
        }
        if !self.started_at.ends_with('Z') || !self.ended_at.ends_with('Z') {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution receipt timestamp is invalid",
            ));
        }
        if self.stdout.contains('\0') || self.stderr.contains('\0') {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution receipt output is invalid",
            ));
        }
        if self.stdout_bytes != self.stdout.as_bytes().len()
            || self.stderr_bytes != self.stderr.as_bytes().len()
            || self.stdout_sha256 != sha256_text(&self.stdout)
            || self.stderr_sha256 != sha256_text(&self.stderr)
        {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution receipt output digest is invalid",
            ));
        }
        validate_digest(&self.stdout_sha256, "execution receipt stdout digest")?;
        validate_digest(&self.stderr_sha256, "execution receipt stderr digest")?;
        Ok(())
    }

    fn make(
        request: &ExecutionRequest,
        status: &str,
        exit_code: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        effect_started: bool,
        reconciliation_required: bool,
        error_code: impl Into<String>,
        error_message: impl Into<String>,
        process_pid: Option<u32>,
        process_identity_value: String,
    ) -> Result<Self, StructuredFailure> {
        let stdout = String::from_utf8(stdout).map_err(|error| {
            StructuredFailure::new(
                "invalid-utf8",
                format!("execution stdout was not valid UTF-8: {error}"),
            )
        })?;
        let stderr = String::from_utf8(stderr).map_err(|error| {
            StructuredFailure::new(
                "invalid-utf8",
                format!("execution stderr was not valid UTF-8: {error}"),
            )
        })?;
        let request_digest = request.request_digest()?;
        let started_at = utc_timestamp();
        let ended_at = utc_timestamp();
        let receipt_id = format!(
            "execution-receipt:{}",
            sha256_text(&format!(
                "{}\n{}\n{}\n{}\n{}",
                request.request_id, request_digest, started_at, ended_at, status
            ))
        );
        let receipt = Self {
            schema_version: EXECUTION_SCHEMA_VERSION,
            request_id: request.request_id.clone(),
            request_digest,
            effect: request.effect.clone(),
            status: status.to_owned(),
            started_at,
            ended_at,
            exit_code,
            stdout_bytes: stdout.as_bytes().len(),
            stderr_bytes: stderr.as_bytes().len(),
            stdout_sha256: sha256_text(&stdout),
            stderr_sha256: sha256_text(&stderr),
            stdout,
            stderr,
            effect_started,
            reconciliation_required,
            error_code: error_code.into(),
            error_message: error_message.into(),
            receipt_id,
            owner_pid: Some(std::process::id()),
            owner_identity: process_identity(std::process::id()),
            process_pid,
            process_identity: process_identity_value,
            provider: "rust-shadow".to_owned(),
        };
        receipt.validate()?;
        Ok(receipt)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProcessOutcomeStatus {
    Completed,
    Cancelled,
    TimedOut,
    OutputLimit,
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessOutcome {
    pub status: ProcessOutcomeStatus,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub process_pid: Option<u32>,
    pub process_identity: String,
    pub effect_started: bool,
    pub error_code: String,
    pub error_message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LaunchError {
    pub failure: StructuredFailure,
    pub effect_started: bool,
    pub process_pid: Option<u32>,
    pub process_identity: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessBinding {
    pub pid: u32,
    pub identity: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlSignal {
    Cancelled(String),
}

pub struct ExecutionCallbacks<'a> {
    pub process_started: Option<&'a mut dyn FnMut(ProcessBinding) -> Result<(), ControlSignal>>,
    pub poll: Option<&'a mut dyn FnMut() -> Result<(), ControlSignal>>,
}

impl<'a> ExecutionCallbacks<'a> {
    pub fn none() -> Self {
        Self {
            process_started: None,
            poll: None,
        }
    }
}

pub trait ProcessLauncher: Send + Sync {
    fn run(
        &self,
        request: &ExecutionRequest,
        callbacks: &mut ExecutionCallbacks<'_>,
    ) -> Result<ProcessOutcome, LaunchError>;
}

pub struct RustExecutionProvider {
    launcher: Box<dyn ProcessLauncher>,
}

impl RustExecutionProvider {
    pub fn new() -> Self {
        Self {
            launcher: Box::new(SystemProcessLauncher),
        }
    }

    pub fn validate_request(request: &ExecutionRequest) -> Result<(), StructuredFailure> {
        request.validate()?;
        validate_prepared_argv(request)
    }

    pub fn execute(
        &self,
        request: &ExecutionRequest,
    ) -> Result<ExecutionReceipt, StructuredFailure> {
        self.execute_with_callbacks(request, &mut ExecutionCallbacks::none())
    }

    pub fn execute_with_callbacks(
        &self,
        request: &ExecutionRequest,
        callbacks: &mut ExecutionCallbacks<'_>,
    ) -> Result<ExecutionReceipt, StructuredFailure> {
        Self::validate_request(request)?;
        let outcome = match self.launcher.run(request, callbacks) {
            Ok(outcome) => outcome,
            Err(error) => {
                if error.effect_started {
                    return ExecutionReceipt::make(
                        request,
                        "outcome-unknown",
                        None,
                        Vec::new(),
                        Vec::new(),
                        true,
                        true,
                        "outcome-unknown",
                        error.failure.message,
                        error.process_pid,
                        error.process_identity,
                    );
                }
                return ExecutionReceipt::make(
                    request,
                    "start-failed",
                    Some(127),
                    Vec::new(),
                    Vec::new(),
                    false,
                    false,
                    error.failure.code,
                    error.failure.message,
                    error.process_pid,
                    error.process_identity,
                );
            }
        };
        let (status, exit_code, reconciliation_required, error_code, error_message) =
            match outcome.status {
                ProcessOutcomeStatus::Completed => {
                    if outcome.exit_code == Some(0) {
                        (
                            "completed",
                            outcome.exit_code,
                            false,
                            outcome.error_code,
                            outcome.error_message,
                        )
                    } else {
                        (
                            "failed",
                            outcome.exit_code,
                            false,
                            outcome.error_code,
                            outcome.error_message,
                        )
                    }
                }
                ProcessOutcomeStatus::Cancelled => (
                    "cancelled",
                    None,
                    false,
                    "cancelled".to_owned(),
                    if outcome.error_message.is_empty() {
                        "Process cancellation was requested.".to_owned()
                    } else {
                        outcome.error_message
                    },
                ),
                ProcessOutcomeStatus::TimedOut => (
                    "timed-out",
                    Some(124),
                    false,
                    "timeout".to_owned(),
                    "Process timed out after the bounded timeout.".to_owned(),
                ),
                ProcessOutcomeStatus::OutputLimit => (
                    "output-limit",
                    Some(125),
                    false,
                    "output-limit".to_owned(),
                    "Process output exceeded the bounded output limit.".to_owned(),
                ),
                ProcessOutcomeStatus::Unknown => (
                    "outcome-unknown",
                    None,
                    true,
                    if outcome.error_code.is_empty() {
                        "outcome-unknown".to_owned()
                    } else {
                        outcome.error_code
                    },
                    outcome.error_message,
                ),
            };
        ExecutionReceipt::make(
            request,
            status,
            exit_code,
            outcome.stdout,
            outcome.stderr,
            outcome.effect_started,
            reconciliation_required,
            error_code,
            error_message,
            outcome.process_pid,
            outcome.process_identity,
        )
    }

    #[cfg(test)]
    fn with_test_outcome(outcome: ProcessOutcome) -> Self {
        Self {
            launcher: Box::new(TestLauncher {
                outcome: Ok(outcome),
            }),
        }
    }

    #[cfg(test)]
    fn with_test_failure(error: LaunchError) -> Self {
        Self {
            launcher: Box::new(TestLauncher {
                outcome: Err(error),
            }),
        }
    }
}

impl Default for RustExecutionProvider {
    fn default() -> Self {
        Self::new()
    }
}

struct SystemProcessLauncher;

impl ProcessLauncher for SystemProcessLauncher {
    fn run(
        &self,
        request: &ExecutionRequest,
        callbacks: &mut ExecutionCallbacks<'_>,
    ) -> Result<ProcessOutcome, LaunchError> {
        let mut command = Command::new(&request.argv[0]);
        command
            .args(&request.argv[1..])
            .current_dir(&request.working_directory)
            .env_clear()
            .envs(&request.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_process(&mut command, &request.limits);
        let mut child = command.spawn().map_err(|error| LaunchError {
            failure: StructuredFailure::new(
                "provider-start-failed",
                format!("Rust execution provider could not start: {error}"),
            ),
            effect_started: false,
            process_pid: None,
            process_identity: String::new(),
        })?;
        let pid = child.id();
        let identity = process_identity(pid);
        let binding = ProcessBinding {
            pid,
            identity: identity.clone(),
        };
        if let Some(callback) = callbacks.process_started.as_mut() {
            if let Err(ControlSignal::Cancelled(message)) = callback(binding) {
                let (status, error_code, error_message) = match terminate_child(
                    &mut child,
                    pid,
                    &identity,
                    request.limits.descendant_grace_seconds,
                ) {
                    Ok(()) => (
                        ProcessOutcomeStatus::Cancelled,
                        "cancelled".to_owned(),
                        message,
                    ),
                    Err(error) => (
                        ProcessOutcomeStatus::Unknown,
                        "cleanup-uncertain".to_owned(),
                        error,
                    ),
                };
                return Ok(ProcessOutcome {
                    status,
                    exit_code: None,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    process_pid: Some(pid),
                    process_identity: identity,
                    effect_started: true,
                    error_code,
                    error_message,
                });
            }
        }
        let total_output = Arc::new(AtomicUsize::new(0));
        let output_limited = Arc::new(AtomicBool::new(false));
        let stdout = spawn_capture(
            child.stdout.take().ok_or_else(|| LaunchError {
                failure: StructuredFailure::new(
                    "provider-io-failure",
                    "Rust execution provider did not expose stdout.",
                ),
                effect_started: true,
                process_pid: Some(pid),
                process_identity: identity.clone(),
            })?,
            request
                .limits
                .output_limit_bytes
                .saturating_sub(OUTPUT_MESSAGE_RESERVE),
            total_output.clone(),
            output_limited.clone(),
        );
        let stderr = spawn_capture(
            child.stderr.take().ok_or_else(|| LaunchError {
                failure: StructuredFailure::new(
                    "provider-io-failure",
                    "Rust execution provider did not expose stderr.",
                ),
                effect_started: true,
                process_pid: Some(pid),
                process_identity: identity.clone(),
            })?,
            request
                .limits
                .output_limit_bytes
                .saturating_sub(OUTPUT_MESSAGE_RESERVE),
            total_output,
            output_limited.clone(),
        );
        let input_thread = child.stdin.take().map(|mut stdin| {
            let input = request.input_text.clone();
            thread::spawn(move || -> Result<(), String> {
                if let Some(input) = input {
                    if let Err(error) = stdin.write_all(input.as_bytes()) {
                        if error.kind() != io::ErrorKind::BrokenPipe {
                            return Err(format!(
                                "Rust execution provider could not write input: {error}"
                            ));
                        }
                    }
                }
                Ok(())
            })
        });
        let started = Instant::now();
        let mut final_status = None;
        let mut exit_code = None;
        let mut leader_exited_at = None;
        loop {
            if let Some(callback) = callbacks.poll.as_mut() {
                if let Err(ControlSignal::Cancelled(message)) = callback() {
                    if let Err(error) = terminate_child(
                        &mut child,
                        pid,
                        &identity,
                        request.limits.descendant_grace_seconds,
                    ) {
                        final_status = Some((ProcessOutcomeStatus::Unknown, error));
                        break;
                    }
                    final_status = Some((ProcessOutcomeStatus::Cancelled, message));
                    break;
                }
            }
            if output_limited.load(Ordering::Relaxed) {
                final_status = Some(match terminate_child(
                    &mut child,
                    pid,
                    &identity,
                    request.limits.descendant_grace_seconds,
                ) {
                    Ok(()) => (
                        ProcessOutcomeStatus::OutputLimit,
                        format!(
                            "Process output exceeded the {}-byte aggregate limit and was terminated.",
                            request.limits.output_limit_bytes
                        ),
                    ),
                    Err(error) => (ProcessOutcomeStatus::Unknown, error),
                });
                break;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    exit_code = process_exit_code(status);
                    if leader_exited_at.is_none() {
                        leader_exited_at = Some(Instant::now());
                    }
                    if stdout.is_finished() && stderr.is_finished() {
                        #[cfg(unix)]
                        if process_group_is_live(pid) {
                            // The leader exited but a descendant still owns the
                            // process group; keep the bounded grace timer active.
                        } else {
                            break;
                        }
                        #[cfg(not(unix))]
                        break;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    let cleanup_error = terminate_child(
                        &mut child,
                        pid,
                        &identity,
                        request.limits.descendant_grace_seconds,
                    )
                    .err();
                    return Err(LaunchError {
                        failure: StructuredFailure::new(
                            "provider-wait-failure",
                            format!(
                                "Rust execution provider could not observe the process: {error}{}",
                                cleanup_error
                                    .map(|detail| format!("; cleanup: {detail}"))
                                    .unwrap_or_default()
                            ),
                        ),
                        effect_started: true,
                        process_pid: Some(pid),
                        process_identity: identity,
                    });
                }
            }
            if let Some(exited_at) = leader_exited_at {
                if exited_at.elapsed().as_secs_f64() >= request.limits.descendant_grace_seconds {
                    final_status = Some(match terminate_child(
                        &mut child,
                        pid,
                        &identity,
                        request.limits.descendant_grace_seconds,
                    ) {
                        Ok(()) => (
                            ProcessOutcomeStatus::TimedOut,
                            format!(
                                "Process descendants timed out {:?} seconds after the leader exited and were terminated.",
                                request.limits.descendant_grace_seconds
                            ),
                        ),
                        Err(error) => (ProcessOutcomeStatus::Unknown, error),
                    });
                    break;
                }
            }
            if started.elapsed().as_secs_f64() >= request.limits.timeout_seconds {
                final_status = Some(
                    match terminate_child(
                        &mut child,
                        pid,
                        &identity,
                        request.limits.descendant_grace_seconds,
                    ) {
                        Ok(()) => (
                            ProcessOutcomeStatus::TimedOut,
                            format!(
                                "Process timed out after {:?} seconds.",
                                request.limits.timeout_seconds
                            ),
                        ),
                        Err(error) => (ProcessOutcomeStatus::Unknown, error),
                    },
                );
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        if final_status.is_none() && exit_code.is_none() {
            exit_code = child.wait().ok().and_then(process_exit_code);
        }
        let mut stdout = join_capture(stdout, "stdout").map_err(|error| LaunchError {
            failure: StructuredFailure::new("provider-io-failure", error),
            effect_started: true,
            process_pid: Some(pid),
            process_identity: identity.clone(),
        })?;
        let mut stderr = join_capture(stderr, "stderr").map_err(|error| LaunchError {
            failure: StructuredFailure::new("provider-io-failure", error),
            effect_started: true,
            process_pid: Some(pid),
            process_identity: identity.clone(),
        })?;
        if let Some(input_thread) = input_thread {
            match input_thread.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    return Err(LaunchError {
                        failure: StructuredFailure::new("provider-io-failure", error),
                        effect_started: true,
                        process_pid: Some(pid),
                        process_identity: identity,
                    });
                }
                Err(_) => {
                    return Err(LaunchError {
                        failure: StructuredFailure::new(
                            "provider-io-failure",
                            "Rust execution provider input thread failed.",
                        ),
                        effect_started: true,
                        process_pid: Some(pid),
                        process_identity: identity,
                    });
                }
            }
        }
        let (mut status, mut error_message) = final_status.unwrap_or_else(|| {
            if output_limited.load(Ordering::Relaxed) {
                (
                    ProcessOutcomeStatus::OutputLimit,
                    format!(
                        "Process output exceeded the {}-byte aggregate limit and was terminated.",
                        request.limits.output_limit_bytes
                    ),
                )
            } else {
                (ProcessOutcomeStatus::Completed, String::new())
            }
        });
        if std::str::from_utf8(&stdout).is_err() || std::str::from_utf8(&stderr).is_err() {
            stdout.clear();
            stderr.clear();
            status = ProcessOutcomeStatus::OutputLimit;
            error_message = "Process output was not valid UTF-8 and was rejected.".to_owned();
        }
        if !error_message.is_empty()
            && matches!(
                status,
                ProcessOutcomeStatus::TimedOut | ProcessOutcomeStatus::OutputLimit
            )
        {
            append_bounded_diagnostic(
                &mut stderr,
                &error_message,
                request
                    .limits
                    .output_limit_bytes
                    .saturating_sub(stdout.len()),
            );
        }
        Ok(ProcessOutcome {
            status,
            exit_code,
            stdout,
            stderr,
            process_pid: Some(pid),
            process_identity: identity,
            effect_started: true,
            error_code: String::new(),
            error_message,
        })
    }
}

fn append_bounded_diagnostic(output: &mut Vec<u8>, diagnostic: &str, budget: usize) {
    let separator = if output.is_empty() { "" } else { "\n" };
    let message = format!("{separator}{diagnostic}");
    let available = budget.saturating_sub(output.len());
    if message.len() <= available {
        output.extend_from_slice(message.as_bytes());
    } else if available > 0 {
        output.extend_from_slice(&message.as_bytes()[..available]);
    }
}

fn spawn_capture<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    total: Arc<AtomicUsize>,
    output_limited: Arc<AtomicBool>,
) -> thread::JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || -> io::Result<Vec<u8>> {
        let mut output = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let prior = total.fetch_add(count, Ordering::Relaxed);
            let allowed = limit.saturating_sub(prior);
            output.extend_from_slice(&buffer[..count.min(allowed)]);
            if prior.saturating_add(count) > limit {
                output_limited.store(true, Ordering::Relaxed);
                break;
            }
        }
        Ok(output)
    })
}

fn join_capture(
    handle: thread::JoinHandle<io::Result<Vec<u8>>>,
    stream: &str,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if !handle.is_finished() {
        return Err(format!(
            "Rust execution provider {stream} capture did not drain"
        ));
    }
    match handle.join() {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(format!(
            "Rust execution provider could not read {stream}: {error}"
        )),
        Err(_) => Err(format!(
            "Rust execution provider {stream} capture thread failed"
        )),
    }
}

fn terminate_child(
    child: &mut Child,
    pid: u32,
    expected_identity: &str,
    grace_seconds: f64,
) -> Result<(), String> {
    let leader_was_reaped = child
        .try_wait()
        .map_err(|error| format!("cleanup wait failed: {error}"))?
        .is_some();
    #[cfg(unix)]
    {
        if !leader_was_reaped
            && (expected_identity.is_empty() || process_identity(pid) != expected_identity)
        {
            return Err("process identity could not be verified for cleanup".to_owned());
        }
        let process_group = unsafe { libc::getpgid(pid as libc::pid_t) };
        if !leader_was_reaped && process_group != pid as libc::pid_t {
            return Err("process group identity could not be verified for cleanup".to_owned());
        }
        if !leader_was_reaped && unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(format!("cleanup SIGTERM failed: {error}"));
            }
        }
        if leader_was_reaped && !process_group_is_live(pid) {
            return Ok(());
        }
        if leader_was_reaped {
            unsafe {
                let _ = libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
            }
        }
    }
    #[cfg(not(unix))]
    if child.kill().is_err() {
        return Err("process cleanup failed".to_owned());
    }

    let deadline = Instant::now() + Duration::from_secs_f64(grace_seconds.min(60.0));
    while Instant::now() < deadline {
        if child
            .try_wait()
            .map_err(|error| format!("cleanup wait failed: {error}"))?
            .is_some()
        {
            #[cfg(unix)]
            if process_group_is_live(pid) {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            return Ok(());
        }
        #[cfg(unix)]
        if !process_group_is_live(pid) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }

    #[cfg(unix)]
    {
        if !leader_was_reaped && process_identity(pid) != expected_identity {
            return Err("process identity changed before forced cleanup".to_owned());
        }
        if unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(format!("cleanup SIGKILL failed: {error}"));
            }
        }
    }
    #[cfg(not(unix))]
    child
        .kill()
        .map_err(|error| format!("cleanup kill failed: {error}"))?;
    let wait_deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < wait_deadline {
        if child
            .try_wait()
            .map_err(|error| format!("cleanup wait failed: {error}"))?
            .is_some()
        {
            #[cfg(unix)]
            if process_group_is_live(pid) {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err("process cleanup did not prove quiescence".to_owned())
}

#[cfg(unix)]
fn process_group_is_live(pid: u32) -> bool {
    let result = unsafe { libc::kill(-(pid as libc::pid_t), 0) };
    if result == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn configure_process(command: &mut Command, limits: &ExecutionLimits) {
    #[cfg(unix)]
    {
        let _ = limits;
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) != 0 {
                    let error = io::Error::last_os_error();
                    return Err(io::Error::new(
                        error.kind(),
                        format!("setpgid failed: {error}"),
                    ));
                }
                Ok(())
            });
        }
    }
}

fn process_exit_code(status: std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(code) = status.code() {
            return Some(code);
        }
        return status.signal().map(|signal| -signal);
    }
    #[cfg(not(unix))]
    {
        status.code()
    }
}

#[cfg(test)]
struct TestLauncher {
    outcome: Result<ProcessOutcome, LaunchError>,
}

#[cfg(test)]
impl ProcessLauncher for TestLauncher {
    fn run(
        &self,
        _request: &ExecutionRequest,
        _callbacks: &mut ExecutionCallbacks<'_>,
    ) -> Result<ProcessOutcome, LaunchError> {
        self.outcome.clone()
    }
}

fn validate_identity(value: &str, label: &str) -> Result<(), StructuredFailure> {
    if value.is_empty()
        || value.len() > MAX_REQUEST_ID_BYTES
        || value.contains('\0')
        || value
            .chars()
            .any(|character| !(character.is_ascii_alphanumeric() || ".:/_=-".contains(character)))
    {
        return Err(StructuredFailure::new(
            "contract-failure",
            format!("{label} is invalid"),
        ));
    }
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> Result<(), StructuredFailure> {
    if value.len() != 64
        || value
            .chars()
            .any(|character| !character.is_ascii_hexdigit() || character.is_ascii_uppercase())
    {
        return Err(StructuredFailure::new(
            "contract-failure",
            format!("{label} is invalid"),
        ));
    }
    Ok(())
}

fn validate_unique_paths(values: &[String], label: &str) -> Result<(), StructuredFailure> {
    if values.len() > MAX_PATH_ENTRIES {
        return Err(StructuredFailure::new(
            "contract-failure",
            format!("{label} are too numerous"),
        ));
    }
    for value in values {
        canonical_path(value, label)?;
    }
    for (index, value) in values.iter().enumerate() {
        if values[index + 1..].iter().any(|other| other == value) {
            return Err(StructuredFailure::new(
                "contract-failure",
                format!("{label} must be unique"),
            ));
        }
    }
    Ok(())
}

fn validate_absolute_paths(values: &[String], label: &str) -> Result<(), StructuredFailure> {
    validate_unique_paths(values, label)
}

fn lexical_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn canonical_path(value: &str, label: &str) -> Result<String, StructuredFailure> {
    let path = Path::new(value);
    if !path.is_absolute() || value.is_empty() || value.contains('\0') {
        return Err(StructuredFailure::new(
            "contract-failure",
            format!("{label} must be a canonical absolute path"),
        ));
    }
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| lexical_path(path));
    if canonical.to_string_lossy() != value {
        return Err(StructuredFailure::new(
            "contract-failure",
            format!("{label} must be canonical and absolute"),
        ));
    }
    Ok(value.to_owned())
}

fn is_trusted_helper(path: &str, name: &str) -> bool {
    let candidate = Path::new(path);
    if !candidate.is_absolute()
        || candidate.file_name().and_then(|value| value.to_str()) != Some(name)
        || !TRUSTED_EXECUTABLE_ROOTS
            .iter()
            .map(Path::new)
            .any(|root| candidate.parent() == Some(root))
    {
        return false;
    }
    if candidate.exists() {
        return candidate.is_file()
            && fs::metadata(candidate)
                .map(|metadata| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        metadata.permissions().mode() & 0o111 != 0
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = metadata;
                        true
                    }
                })
                .unwrap_or(false);
    }
    cfg!(test)
}

fn is_protected_root(path: &str) -> bool {
    path == "/"
        || PROTECTED_READONLY_ROOTS
            .iter()
            .map(Path::new)
            .any(|root| Path::new(path).starts_with(root))
}

fn has_exact_pair(pairs: &[(String, String)], source: &str, destination: &str) -> bool {
    pairs.iter().any(|(actual_source, actual_destination)| {
        actual_source == source && actual_destination == destination
    })
}

fn is_implicit_readonly_mount(source: &str, destination: &str) -> bool {
    source == destination && IMPLICIT_READONLY_ROOTS.contains(&source)
}

fn validate_resource_wrapper<'a>(
    command: &'a [String],
    limits: &ExecutionLimits,
) -> Result<&'a [String], StructuredFailure> {
    if command.is_empty() {
        return Err(StructuredFailure::new(
            "contract-failure",
            "execution Bubblewrap command is empty",
        ));
    }
    if !is_trusted_helper(&command[0], "prlimit") {
        return Err(StructuredFailure::new(
            "contract-failure",
            "execution Bubblewrap resource boundary is missing",
        ));
    }
    let expected = [
        format!("--as={}", limits.address_space_bytes),
        format!("--fsize={}", limits.file_size_bytes),
        format!("--nofile={}", limits.open_file_limit),
        format!("--nproc={}", limits.process_count_limit),
    ];
    if command.len() < expected.len() + 3
        || command[1..5]
            .iter()
            .zip(expected.iter())
            .any(|(actual, expected)| actual != expected)
        || command[5] != "--"
    {
        return Err(StructuredFailure::new(
            "contract-failure",
            "execution prlimit wrapper does not match the request limits",
        ));
    }
    Ok(&command[6..])
}

fn validate_prepared_argv(request: &ExecutionRequest) -> Result<(), StructuredFailure> {
    if request.sandbox.mode != "bubblewrap" {
        return Err(StructuredFailure::new(
            "contract-failure",
            "execution provider requires the Bubblewrap sandbox",
        ));
    }
    let executable = Path::new(&request.argv[0]);
    canonical_path(&request.argv[0], "execution Bubblewrap executable")?;
    if !is_trusted_helper(&request.argv[0], "bwrap") {
        return Err(StructuredFailure::new(
            "contract-failure",
            "execution provider requires a trusted Bubblewrap executable",
        ));
    }
    if executable.file_name().and_then(|name| name.to_str()) != Some("bwrap") {
        return Err(StructuredFailure::new(
            "contract-failure",
            "execution provider requires a prepared Bubblewrap argv",
        ));
    }
    let separator = request
        .argv
        .iter()
        .position(|argument| argument == "--")
        .ok_or_else(|| {
            StructuredFailure::new(
                "contract-failure",
                "execution Bubblewrap argv must contain one non-empty command boundary",
            )
        })?;
    if separator <= 1 || separator + 1 >= request.argv.len() {
        return Err(StructuredFailure::new(
            "contract-failure",
            "execution Bubblewrap argv must contain one non-empty command boundary",
        ));
    }
    let prefix = &request.argv[1..separator];
    let mut readonly_mounts = Vec::new();
    let mut writable_mounts = Vec::new();
    let mut directories = Vec::new();
    let mut chdir: Option<String> = None;
    let mut flags = BTreeMap::<&str, usize>::new();
    let mut index = 0;
    while index < prefix.len() {
        let argument = prefix[index].as_str();
        match argument {
            "--die-with-parent" | "--new-session" | "--unshare-user" | "--unshare-pid" => {
                *flags.entry(argument).or_default() += 1;
                index += 1;
            }
            "--tmpfs" => {
                let value = prefix.get(index + 1).ok_or_else(|| {
                    StructuredFailure::new(
                        "contract-failure",
                        "Bubblewrap tmpfs mount is incomplete",
                    )
                })?;
                if value != "/" && value != "/tmp" {
                    return Err(StructuredFailure::new(
                        "contract-failure",
                        "Bubblewrap tmpfs mount is outside the prepared boundary",
                    ));
                }
                *flags
                    .entry(if value == "/" {
                        "--tmpfs:/"
                    } else {
                        "--tmpfs:/tmp"
                    })
                    .or_default() += 1;
                index += 2;
            }
            "--dev" | "--proc" => {
                let value = prefix.get(index + 1).ok_or_else(|| {
                    StructuredFailure::new(
                        "contract-failure",
                        "Bubblewrap special mount is incomplete",
                    )
                })?;
                let expected = if argument == "--dev" { "/dev" } else { "/proc" };
                if value != expected {
                    return Err(StructuredFailure::new(
                        "contract-failure",
                        "Bubblewrap special mount does not match the prepared boundary",
                    ));
                }
                *flags.entry(argument).or_default() += 1;
                index += 2;
            }
            "--ro-bind" | "--bind" => {
                let source = prefix.get(index + 1).ok_or_else(|| {
                    StructuredFailure::new("contract-failure", "Bubblewrap bind source is missing")
                })?;
                let destination = prefix.get(index + 2).ok_or_else(|| {
                    StructuredFailure::new(
                        "contract-failure",
                        "Bubblewrap bind destination is missing",
                    )
                })?;
                let implicit_readonly_mount =
                    argument == "--ro-bind" && is_implicit_readonly_mount(source, destination);
                if !implicit_readonly_mount {
                    canonical_path(source, "Bubblewrap bind source")?;
                    canonical_path(destination, "Bubblewrap bind destination")?;
                }
                if argument == "--ro-bind" {
                    readonly_mounts.push((source.clone(), destination.clone()));
                } else {
                    if is_protected_root(destination) {
                        return Err(StructuredFailure::new(
                            "contract-failure",
                            "Bubblewrap writable mount crosses a protected root",
                        ));
                    }
                    writable_mounts.push((source.clone(), destination.clone()));
                }
                index += 3;
            }
            "--dir" => {
                let value = prefix.get(index + 1).ok_or_else(|| {
                    StructuredFailure::new("contract-failure", "Bubblewrap directory is missing")
                })?;
                canonical_path(value, "Bubblewrap directory")?;
                directories.push(value.clone());
                index += 2;
            }
            "--chdir" => {
                let value = prefix.get(index + 1).ok_or_else(|| {
                    StructuredFailure::new(
                        "contract-failure",
                        "Bubblewrap working directory is missing",
                    )
                })?;
                canonical_path(value, "Bubblewrap working directory")?;
                if chdir.replace(value.clone()).is_some() {
                    return Err(StructuredFailure::new(
                        "contract-failure",
                        "Bubblewrap working directory is duplicated",
                    ));
                }
                index += 2;
            }
            _ => {
                return Err(StructuredFailure::new(
                    "contract-failure",
                    format!("unsupported Bubblewrap option: {argument}"),
                ));
            }
        }
    }
    for flag in [
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
    ] {
        if flags.get(flag).copied() != Some(1) {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution Bubblewrap argv must retain the complete process boundary",
            ));
        }
    }
    if flags.get("--tmpfs:/").copied() != Some(1)
        || flags.get("--tmpfs:/tmp").copied() != Some(1)
        || flags.get("--dev").copied() != Some(1)
        || flags.get("--proc").copied() != Some(1)
        || chdir.as_deref() != Some(request.working_directory.as_str())
    {
        return Err(StructuredFailure::new(
            "contract-failure",
            "execution Bubblewrap argv does not match the prepared namespace boundary",
        ));
    }
    let writable_roots: std::collections::HashSet<_> =
        request.sandbox.writable_roots.iter().collect();
    for root in &request.sandbox.writable_roots {
        if is_protected_root(root) || !has_exact_pair(&writable_mounts, root, root) {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution Bubblewrap argv does not match writable roots",
            ));
        }
    }
    for root in &request.sandbox.readable_roots {
        if !writable_roots.contains(root) && !has_exact_pair(&readonly_mounts, root, root) {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution Bubblewrap argv does not match readable roots",
            ));
        }
    }
    for (source, destination) in &request.sandbox.readonly_bindings {
        if !has_exact_pair(&readonly_mounts, source, destination) {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution Bubblewrap argv does not match readonly bindings",
            ));
        }
    }
    for (source, destination) in &readonly_mounts {
        let system_mount = is_implicit_readonly_mount(source, destination);
        let declared_mount = request
            .sandbox
            .readable_roots
            .iter()
            .any(|root| root == source && root == destination)
            || request.sandbox.readonly_bindings.iter().any(
                |(expected_source, expected_destination)| {
                    expected_source == source && expected_destination == destination
                },
            );
        if !system_mount && !declared_mount {
            let command =
                validate_resource_wrapper(&request.argv[separator + 1..], &request.limits)?;
            let implicit = command.iter().any(|argument| argument == destination)
                && !is_protected_root(destination)
                && source == destination;
            if !implicit {
                return Err(StructuredFailure::new(
                    "contract-failure",
                    "execution Bubblewrap argv contains an undeclared host read",
                ));
            }
        }
    }
    for directory in directories {
        let command = validate_resource_wrapper(&request.argv[separator + 1..], &request.limits)?;
        if !command
            .iter()
            .any(|argument| argument.starts_with(&format!("{directory}/")))
        {
            return Err(StructuredFailure::new(
                "contract-failure",
                "execution Bubblewrap directory is not bound to an approved executable",
            ));
        }
    }
    let command = validate_resource_wrapper(&request.argv[separator + 1..], &request.limits)?;
    if command.is_empty() {
        return Err(StructuredFailure::new(
            "contract-failure",
            "execution Bubblewrap command is empty",
        ));
    }
    Ok(())
}

fn sha256_text(value: &str) -> String {
    sha256_bytes(value.as_bytes())
}

fn sha256_bytes(value: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value);
    format!("{:x}", hasher.finalize())
}

fn json_string(value: &str) -> String {
    let mut encoded = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{08}' => encoded.push_str("\\b"),
            '\u{0c}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character.is_ascii_control() => {
                encoded.push_str(&format!("\\u{:04x}", character as u32))
            }
            character if character.is_ascii() => encoded.push(character),
            character => {
                let mut units = [0_u16; 2];
                for unit in character.encode_utf16(&mut units).iter() {
                    encoded.push_str(&format!("\\u{:04x}", unit));
                }
            }
        }
    }
    encoded.push('"');
    encoded
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => canonical_number(value),
        Value::String(value) => json_string(value),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(values) => {
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!("{}:{}", json_string(key), canonical_json(value)))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn canonical_number(value: &serde_json::Number) -> String {
    let raw = value.to_string();
    let (mantissa, exponent) = raw
        .split_once('e')
        .or_else(|| raw.split_once('E'))
        .unwrap_or((&raw, ""));
    if exponent.is_empty() {
        return raw;
    }
    let exponent_value = exponent.parse::<i32>().unwrap_or(0);
    let sign = if exponent_value < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent_value.unsigned_abs())
}

fn utc_timestamp() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = duration.as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }).div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
        - day_of_era / 146_096)
        .div_euclid(365);
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2).div_euclid(153);
    let day = day_of_year - (153 * month_part + 2).div_euclid(5) + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    let hour = day_seconds / 3_600;
    let minute = day_seconds.rem_euclid(3_600) / 60;
    let second = day_seconds.rem_euclid(60);
    let millis = duration.subsec_millis();
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn process_identity(pid: u32) -> String {
    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{pid}/stat");
        if let Ok(contents) = fs::read_to_string(path) {
            if let Some(remainder) = contents.rsplit_once(')').map(|(_, remainder)| remainder) {
                let fields: Vec<_> = remainder.split_whitespace().collect();
                if fields.len() > 19 {
                    return format!("linux:{pid}:{}", fields[19]);
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int,
            )
        };
        if size == std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int {
            let info = unsafe { info.assume_init() };
            return format!(
                "macos:{pid}:{}:{}",
                info.pbi_start_tvsec, info.pbi_start_tvusec
            );
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_and_rust_use_the_same_request_digest_and_normalized_receipt_shape() {
        let request = test_request("shadow-rust-digest");
        assert_eq!(
            request.request_digest().unwrap(),
            "f0ff5caad0df80a6ef5165fd51c14a09e8cf5e67f53f41a3e341738507bc9a12"
        );
        let receipt = RustExecutionProvider::with_test_outcome(ProcessOutcome {
            status: ProcessOutcomeStatus::Completed,
            exit_code: Some(0),
            stdout: b"same output".to_vec(),
            stderr: Vec::new(),
            process_pid: None,
            process_identity: String::new(),
            effect_started: true,
            error_code: String::new(),
            error_message: String::new(),
        })
        .execute(&request)
        .expect("test provider should return a receipt");
        assert_eq!(receipt.status, "completed");
        assert_eq!(receipt.stdout_bytes, 11);
        assert_eq!(receipt.stdout, "same output");
        assert_eq!(receipt.provider, "rust-shadow");
    }

    #[test]
    fn validation_rejects_shell_parsing_and_unprepared_sandbox_boundaries() {
        let mut request = test_request("shadow-rust-validation");
        request.shell = true;
        assert_eq!(
            RustExecutionProvider::validate_request(&request)
                .expect_err("shell parsing must fail")
                .code,
            "contract-failure"
        );

        let mut request = test_request("shadow-rust-sandbox");
        request.argv = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            "echo unsafe".to_owned(),
        ];
        assert!(RustExecutionProvider::validate_request(&request).is_err());
    }

    #[test]
    fn request_schema_resources_and_undeclared_mounts_fail_closed() {
        let request = test_request("shadow-rust-schema");
        let mut value = serde_json::to_value(&request).expect("request should encode");
        value
            .as_object_mut()
            .expect("request should be an object")
            .remove("schema_version");
        assert!(ExecutionRequest::from_value(value).is_err());

        let mut request = test_request("shadow-rust-resource");
        request.limits.address_space_bytes = MAX_ADDRESS_SPACE_BYTES + 1;
        assert!(request.validate().is_err());

        let mut request = test_request("shadow-rust-mount");
        let separator = request
            .argv
            .iter()
            .position(|argument| argument == "--")
            .expect("test request should have a command boundary");
        request.argv.splice(
            separator..separator,
            [
                "--ro-bind".to_owned(),
                "/tmp/undeclared-shadow-read".to_owned(),
                "/tmp/undeclared-shadow-read".to_owned(),
            ],
        );
        assert!(RustExecutionProvider::validate_request(&request).is_err());
    }

    #[test]
    fn validation_requires_the_python_resource_wrapper() {
        let mut request = test_request("shadow-rust-resource-wrapper");
        let separator = request
            .argv
            .iter()
            .position(|argument| argument == "--")
            .expect("test request should have a command boundary");
        request.argv.truncate(separator + 1);
        request.argv.push("/bin/true".to_owned());

        assert!(RustExecutionProvider::validate_request(&request).is_err());
    }

    #[test]
    fn validation_accepts_python_contract_resource_and_environment_bounds() {
        let mut request = test_request("shadow-rust-python-bounds");
        request.limits.address_space_bytes = 16 * 1024 * 1024 * 1024;
        request.limits.file_size_bytes = 3 * 1024 * 1024 * 1024;
        request.limits.open_file_limit = 2_048;
        request.limits.process_count_limit = 512;
        let separator = request
            .argv
            .iter()
            .position(|argument| argument == "--")
            .expect("test request should have a command boundary");
        request.argv[separator + 2] = format!("--as={}", request.limits.address_space_bytes);
        request.argv[separator + 3] = format!("--fsize={}", request.limits.file_size_bytes);
        request.argv[separator + 4] = format!("--nofile={}", request.limits.open_file_limit);
        request.argv[separator + 5] = format!("--nproc={}", request.limits.process_count_limit);
        request
            .environment
            .insert("PIP_CACHE_DIR".to_owned(), "/tmp/pip-cache".to_owned());
        request
            .environment
            .insert("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned());
        request
            .environment
            .insert("npm_config_cache".to_owned(), "/tmp/npm-cache".to_owned());

        RustExecutionProvider::validate_request(&request)
            .expect("Python-valid limits and environment must remain contract-valid");
    }

    #[test]
    fn validation_rejects_non_adjacent_duplicate_allowed_paths() {
        let mut request = test_request("shadow-rust-duplicate-paths");
        let ExecutionAuthority::LocalAgent { allowed_paths, .. } = &mut request.authority else {
            panic!("test request should use Local Agent authority");
        };
        *allowed_paths = vec!["src".to_owned(), "tests".to_owned(), "src".to_owned()];

        assert!(RustExecutionProvider::validate_request(&request).is_err());
    }

    #[test]
    fn provider_maps_timeout_output_limit_cancellation_and_unknown_results() {
        for (status, expected, error_code) in [
            (ProcessOutcomeStatus::TimedOut, "timed-out", "timeout"),
            (
                ProcessOutcomeStatus::OutputLimit,
                "output-limit",
                "output-limit",
            ),
            (ProcessOutcomeStatus::Cancelled, "cancelled", "cancelled"),
            (
                ProcessOutcomeStatus::Unknown,
                "outcome-unknown",
                "outcome-unknown",
            ),
        ] {
            let receipt = RustExecutionProvider::with_test_outcome(ProcessOutcome {
                status,
                exit_code: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
                process_pid: Some(42),
                process_identity: "test:42".to_owned(),
                effect_started: true,
                error_code: error_code.to_owned(),
                error_message: "test outcome".to_owned(),
            })
            .execute(&test_request("shadow-rust-status"))
            .expect("typed status should be returned");
            assert_eq!(receipt.status, expected);
            assert_eq!(receipt.error_code, error_code);
            assert!(receipt.reconciliation_required == (expected == "outcome-unknown"));
        }
    }

    #[test]
    fn provider_maps_start_failure_without_claiming_an_external_effect() {
        let receipt = RustExecutionProvider::with_test_failure(LaunchError {
            failure: StructuredFailure {
                code: "provider-start-failed".to_owned(),
                message: "missing bwrap".to_owned(),
                recoverable: true,
            },
            effect_started: false,
            process_pid: None,
            process_identity: String::new(),
        })
        .execute(&test_request("shadow-rust-start-failure"))
        .expect("start failure is a typed receipt");
        assert_eq!(receipt.status, "start-failed");
        assert!(!receipt.effect_started);
        assert!(!receipt.reconciliation_required);
    }

    #[test]
    fn output_and_process_cleanup_are_bounded_by_the_request_contract() {
        let request = test_request("shadow-rust-bounds");
        assert_eq!(request.limits.output_limit_bytes, 4096);
        assert_eq!(request.limits.timeout_seconds, 2.0);
        let outcome = ProcessOutcome {
            status: ProcessOutcomeStatus::OutputLimit,
            exit_code: None,
            stdout: vec![b'x'; 4096],
            stderr: Vec::new(),
            process_pid: Some(99),
            process_identity: "test:99".to_owned(),
            effect_started: true,
            error_code: "output-limit".to_owned(),
            error_message: "bounded".to_owned(),
        };
        let receipt = RustExecutionProvider::with_test_outcome(outcome)
            .execute(&request)
            .expect("bounded output should be typed");
        assert_eq!(receipt.stdout_bytes, 4096);
        assert_eq!(receipt.status, "output-limit");
    }

    #[cfg(unix)]
    #[test]
    fn system_launcher_closes_stdin_and_captures_completed_output() {
        if trusted_bwrap_path().is_none() {
            return;
        }
        let (request, root) = system_test_request(
            "shadow-rust-system-complete",
            vec!["/usr/bin/printf".to_owned(), "same output".to_owned()],
            ExecutionLimits {
                timeout_seconds: 2.0,
                output_limit_bytes: 4096,
                address_space_bytes: 8 * 1024 * 1024 * 1024,
                file_size_bytes: 2 * 1024 * 1024 * 1024,
                open_file_limit: 1024,
                process_count_limit: 256,
                descendant_grace_seconds: 0.1,
            },
        );
        let receipt = RustExecutionProvider::new()
            .execute(&request)
            .expect("system provider should return a receipt");
        assert_eq!(receipt.status, "completed");
        assert_eq!(receipt.stdout, "same output");
        assert!(!receipt.reconciliation_required);
        fs::remove_dir_all(root).expect("test fixture should be removable");
    }

    #[cfg(unix)]
    #[test]
    fn system_launcher_enforces_timeout_and_output_limits() {
        if trusted_bwrap_path().is_none() {
            return;
        }
        let (timeout_request, timeout_root) = system_test_request(
            "shadow-rust-system-timeout",
            vec!["/bin/sh".to_owned(), "-c".to_owned(), "sleep 2".to_owned()],
            ExecutionLimits {
                timeout_seconds: 0.05,
                output_limit_bytes: 4096,
                address_space_bytes: 8 * 1024 * 1024 * 1024,
                file_size_bytes: 2 * 1024 * 1024 * 1024,
                open_file_limit: 1024,
                process_count_limit: 256,
                descendant_grace_seconds: 0.05,
            },
        );
        let timeout_receipt = RustExecutionProvider::new()
            .execute(&timeout_request)
            .expect("timeout should be a typed receipt");
        assert_eq!(timeout_receipt.status, "timed-out");
        assert!(!timeout_receipt.reconciliation_required);

        let (output_request, output_root) = system_test_request(
            "shadow-rust-system-output-limit",
            vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "while :; do printf x; done".to_owned(),
            ],
            ExecutionLimits {
                timeout_seconds: 2.0,
                output_limit_bytes: 128,
                address_space_bytes: 8 * 1024 * 1024 * 1024,
                file_size_bytes: 2 * 1024 * 1024 * 1024,
                open_file_limit: 1024,
                process_count_limit: 256,
                descendant_grace_seconds: 0.05,
            },
        );
        let output_receipt = RustExecutionProvider::new()
            .execute(&output_request)
            .expect("output limit should be a typed receipt");
        assert_eq!(output_receipt.status, "output-limit");
        assert!(output_receipt.stdout_bytes <= 128);
        fs::remove_dir_all(timeout_root).expect("timeout fixture should be removable");
        fs::remove_dir_all(output_root).expect("output fixture should be removable");
    }

    #[cfg(unix)]
    #[test]
    fn system_launcher_cancels_after_process_binding_and_cleans_up() {
        if trusted_bwrap_path().is_none() {
            return;
        }
        let (request, root) = system_test_request(
            "shadow-rust-system-cancel",
            vec!["/bin/sh".to_owned(), "-c".to_owned(), "sleep 2".to_owned()],
            ExecutionLimits {
                timeout_seconds: 2.0,
                output_limit_bytes: 4096,
                address_space_bytes: 8 * 1024 * 1024 * 1024,
                file_size_bytes: 2 * 1024 * 1024 * 1024,
                open_file_limit: 1024,
                process_count_limit: 256,
                descendant_grace_seconds: 0.05,
            },
        );
        let mut process_started = |binding: ProcessBinding| {
            assert!(binding.pid > 0);
            Err(ControlSignal::Cancelled("test cancellation".to_owned()))
        };
        let mut callbacks = ExecutionCallbacks {
            process_started: Some(&mut process_started),
            poll: None,
        };
        let receipt = RustExecutionProvider::new()
            .execute_with_callbacks(&request, &mut callbacks)
            .expect("cancellation should be a typed receipt");
        assert_eq!(receipt.status, "cancelled");
        assert!(receipt.effect_started);
        assert!(!receipt.reconciliation_required);
        fs::remove_dir_all(root).expect("cancel fixture should be removable");
    }

    #[cfg(unix)]
    fn system_test_request(
        request_id: &str,
        command: Vec<String>,
        limits: ExecutionLimits,
    ) -> (ExecutionRequest, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "alfredo-execution-shadow-{}-{}",
            std::process::id(),
            request_id.replace(':', "-")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("system fixture root should be created");
        let root = fs::canonicalize(root).expect("system fixture root should be canonical");
        let bwrap = trusted_bwrap_path().expect("test Bubblewrap should be available");
        let root = root.to_string_lossy().into_owned();
        let mut argv = vec![
            bwrap,
            "--die-with-parent".to_owned(),
            "--new-session".to_owned(),
            "--unshare-user".to_owned(),
            "--unshare-pid".to_owned(),
            "--tmpfs".to_owned(),
            "/".to_owned(),
            "--dev".to_owned(),
            "/dev".to_owned(),
            "--proc".to_owned(),
            "/proc".to_owned(),
            "--tmpfs".to_owned(),
            "/tmp".to_owned(),
        ];
        for system_root in ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"] {
            let path = Path::new(system_root);
            if path.exists() {
                argv.extend([
                    "--ro-bind".to_owned(),
                    system_root.to_owned(),
                    system_root.to_owned(),
                ]);
            }
        }
        argv.extend([
            "--bind".to_owned(),
            root.clone(),
            root.clone(),
            "--chdir".to_owned(),
            root.clone(),
            "--".to_owned(),
            "/usr/bin/prlimit".to_owned(),
            format!("--as={}", limits.address_space_bytes),
            format!("--fsize={}", limits.file_size_bytes),
            format!("--nofile={}", limits.open_file_limit),
            format!("--nproc={}", limits.process_count_limit),
            "--".to_owned(),
        ]);
        argv.extend(command);
        let request = ExecutionRequest {
            schema_version: EXECUTION_SCHEMA_VERSION,
            request_id: request_id.to_owned(),
            effect: "local-agent".to_owned(),
            argv,
            working_directory: root.clone(),
            authority: ExecutionAuthority::LocalAgent {
                mission_id: "shadow-system-mission".to_owned(),
                session_id: "shadow-system-session".to_owned(),
                session_revision: 1,
                runner_operation_id: format!("runner:{request_id}"),
                worktree_identity: "managed:shadow-system".to_owned(),
                allowed_paths: vec!["src".to_owned()],
            },
            limits,
            sandbox: ExecutionSandbox {
                mode: "bubblewrap".to_owned(),
                readable_roots: vec![root.clone()],
                writable_roots: vec![root.clone()],
                readonly_bindings: Vec::new(),
            },
            environment: BTreeMap::new(),
            input_text: None,
            input_sha256: None,
            shell: false,
        };
        (request, PathBuf::from(root))
    }

    #[cfg(unix)]
    fn trusted_bwrap_path() -> Option<String> {
        TRUSTED_EXECUTABLE_ROOTS
            .iter()
            .map(|root| Path::new(root).join("bwrap"))
            .find(|path| path.is_file())
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn test_request(request_id: &str) -> ExecutionRequest {
        let working_directory = "/private/tmp/alfredo-shadow-fixture".to_owned();
        ExecutionRequest {
            schema_version: 1,
            request_id: request_id.to_owned(),
            effect: "local-agent".to_owned(),
            argv: vec![
                "/usr/bin/bwrap".to_owned(),
                "--die-with-parent".to_owned(),
                "--new-session".to_owned(),
                "--unshare-user".to_owned(),
                "--unshare-pid".to_owned(),
                "--tmpfs".to_owned(),
                "/".to_owned(),
                "--dev".to_owned(),
                "/dev".to_owned(),
                "--proc".to_owned(),
                "/proc".to_owned(),
                "--tmpfs".to_owned(),
                "/tmp".to_owned(),
                "--ro-bind".to_owned(),
                "/bin".to_owned(),
                "/bin".to_owned(),
                "--chdir".to_owned(),
                working_directory.clone(),
                "--bind".to_owned(),
                working_directory.clone(),
                working_directory.clone(),
                "--".to_owned(),
                "/usr/bin/prlimit".to_owned(),
                "--as=8589934592".to_owned(),
                "--fsize=2147483648".to_owned(),
                "--nofile=1024".to_owned(),
                "--nproc=256".to_owned(),
                "--".to_owned(),
                "/bin/true".to_owned(),
            ],
            working_directory: working_directory.clone(),
            authority: ExecutionAuthority::LocalAgent {
                mission_id: "shadow-mission".to_owned(),
                session_id: "shadow-session".to_owned(),
                session_revision: 3,
                runner_operation_id: "runner:shadow-session:3".to_owned(),
                worktree_identity: "managed:shadow-session".to_owned(),
                allowed_paths: vec!["src".to_owned()],
            },
            limits: ExecutionLimits {
                timeout_seconds: 2.0,
                output_limit_bytes: 4096,
                address_space_bytes: 8 * 1024 * 1024 * 1024,
                file_size_bytes: 2 * 1024 * 1024 * 1024,
                open_file_limit: 1024,
                process_count_limit: 256,
                descendant_grace_seconds: 1.0,
            },
            sandbox: ExecutionSandbox {
                mode: "bubblewrap".to_owned(),
                readable_roots: vec![working_directory.clone()],
                writable_roots: vec![working_directory],
                readonly_bindings: Vec::new(),
            },
            environment: Default::default(),
            input_text: None,
            input_sha256: None,
            shell: false,
        }
    }
}
