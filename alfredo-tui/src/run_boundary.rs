//! Immutable, synced check boundaries. Missing or legacy data is never proof.
use crate::{
    execution::{
        ExecutionAuthority, ExecutionCallbacks, ExecutionLimits, ExecutionReceipt,
        ExecutionRequest, ExecutionSandbox, RustExecutionProvider,
    },
    tasks::{regular_file, Task, TaskStatus},
    worker::Evidence,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Boundary {
    schema_version: u32,
    task: u64,
    run: String,
    baseline: String,
}
fn expected(task: &Task) -> Result<Boundary> {
    let run = task.run.as_ref().ok_or("Missing run claim")?;
    Ok(Boundary {
        schema_version: 1,
        task: task.id,
        run: run.id.clone(),
        baseline: run.baseline.clone(),
    })
}
fn directory(path: &Path) -> Result<()> {
    for path in [path, path.parent().ok_or("Missing runs directory")?] {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_dir() {
            return Err("Run boundary directory is not a regular directory".into());
        }
    }
    Ok(())
}
fn write_once(path: &Path, value: &impl Serialize) -> Result<()> {
    write_bounded(path, value, 4096).map(|_| ())
}
fn write_bounded(path: &Path, value: &impl Serialize, maximum: usize) -> Result<Vec<u8>> {
    let parent = path.parent().ok_or("Missing boundary parent")?;
    directory(parent)?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    if bytes.len() > maximum {
        return Err("Run artifact exceeds its serialized bound".into());
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(all(test, target_os = "linux"))]
    publication_crash_tests::pause_result_write(path, &bytes, false);
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    #[cfg(all(test, target_os = "linux"))]
    publication_crash_tests::pause_result_write(path, &bytes, true);
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn verify_start(path: &Path, task: &Task) -> Result<()> {
    directory(path)?;
    let mut bytes = Vec::new();
    regular_file(&path.join("execution-boundary.json"), false, false)?
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4096 {
        return Err("Run boundary exceeds bounds".into());
    }
    let recorded: Boundary = serde_json::from_slice(&bytes).map_err(|_| "Invalid run boundary")?;
    if recorded != expected(task)? {
        return Err("Run boundary does not match the claimed task/run/baseline".into());
    }
    Ok(())
}
fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
        Ok(_) => {
            Err("Recorded evidence or check-launch intent exists; absence is not proven".into())
        }
    }
}
/// Persist before any worker preparation. A partial marker is not recoverable proof.
pub fn record_start(path: &Path, task: &Task) -> Result<()> {
    write_once(&path.join("execution-boundary.json"), &expected(task)?)
}
/// Must complete durably before spawning the check. Any existing intent blocks reconstruction.
pub fn record_check_intent(path: &Path, task: &Task) -> Result<()> {
    verify_start(path, task)?;
    write_once(&path.join("check-launch-intent.json"), &expected(task)?)
}
/// Read-only projection. Caller must also hold the stopped worker's owner lock.
pub(crate) fn interrupted_before_check(path: &Path, task: &Task) -> Result<Evidence> {
    if task.status != TaskStatus::Running {
        return Err("Task is not running".into());
    }
    verify_start(path, task)?;
    absent(&path.join("evidence.json"))?;
    absent(&path.join("check-launch-intent.json"))?;
    absent(&path.join("check-result.json"))?;
    let run = task.run.as_ref().ok_or("Missing run claim")?;
    Ok(Evidence {
        agent: None,
        run: run.id.clone(), baseline: run.baseline.clone(), status: TaskStatus::Failed,
        detail: "Worker interrupted before check launch; partial work retained, patch not reconstructed. Propose a repair and approve it separately; no effects replayed".into(),
        patch: String::new(), candidate_commit: None, model_metrics: None, generation: None, check: None,
    })
}
/// Called only under the stopped-owner guard by explicit recovery. Never overwrites evidence.
pub(crate) fn record_interrupted(path: &Path, task: &Task) -> Result<()> {
    let evidence = interrupted_before_check(path, task)?;
    write_once(&path.join("evidence.json"), &evidence)
}

const SYSTEM_ROOTS: [&str; 6] = ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"];
const MAX_PATH: usize = 4096;
const MAX_OUTPUT: usize = 16 * 1024;
const MAX_ERROR: usize = 8192;
// JSON can expand one input byte into six escaped bytes. Include the supported
// policy maxima (32 arguments, 32 files), repeated paths and fixed metadata.
const MAX_INTENT: usize = 6 * (32 * 2048 + 32 * 512 + 20 * MAX_PATH) + 16 * 1024;
const MAX_RESULT: usize = 6 * (MAX_OUTPUT + MAX_ERROR + 2048) + 16 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckIntent {
    schema_version: u32,
    contract_version: u32,
    task: u64,
    run: String,
    baseline: String,
    mission: String,
    system_roots: Vec<String>,
    request_digest: String,
    request: ExecutionRequest,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckResult {
    schema_version: u32,
    intent_sha256: String,
    request_digest: String,
    receipt: ExecutionReceipt,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    regular_file(path, false, false)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > maximum {
        return Err("Run artifact exceeds its serialized bound".into());
    }
    Ok(bytes)
}

fn managed_worktree(path: &Path, task: &Task) -> Result<String> {
    directory(path)?;
    let run = task.run.as_ref().ok_or("Missing run claim")?;
    if !path.is_absolute()
        || path.canonicalize().map_err(|e| e.to_string())? != path
        || path.file_name().and_then(|name| name.to_str()) != Some(&run.id)
        || path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("runs")
        || run.id.len() > 100
        || !run.id.starts_with("task-")
        || !run
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || run.baseline.len() != 40
        || !run.baseline.bytes().all(|byte| byte.is_ascii_hexdigit())
        || task.id == 0
    {
        return Err("Check boundary does not name the canonical managed run".into());
    }
    let root = path
        .join("worktree")
        .to_str()
        .ok_or("Non-UTF-8 worktree")?
        .to_owned();
    if root.len() + "/.git".len() > MAX_PATH {
        return Err("Check worktree path exceeds its bound".into());
    }
    Ok(root)
}

// This is the immutable version-1 native check contract, intentionally independent
// of worker defaults and of which system roots exist on a later recovery host.
fn contract_request(
    root: String,
    task: &Task,
    mission: &str,
    system_roots: &[String],
) -> Result<ExecutionRequest> {
    if mission.trim().is_empty() || mission.len() > 120 || mission.chars().any(char::is_control) {
        return Err("Invalid check Mission identity".into());
    }
    let policy = task.policy.as_ref().ok_or("Missing work policy")?;
    policy.validate()?;
    let run = task.run.as_ref().ok_or("Missing run claim")?;
    let expected_roots: Vec<_> = SYSTEM_ROOTS
        .iter()
        .filter(|root| system_roots.iter().any(|recorded| recorded == **root))
        .map(|root| (*root).to_owned())
        .collect();
    if expected_roots != system_roots {
        return Err("Check system mounts do not match contract version 1".into());
    }
    let gitfile = format!("{root}/.git");
    let limits = ExecutionLimits {
        timeout_seconds: 60.0,
        output_limit_bytes: MAX_OUTPUT,
        address_space_bytes: 8 * 1024 * 1024 * 1024,
        file_size_bytes: 32 * 1024 * 1024,
        open_file_limit: 1024,
        process_count_limit: 128,
        descendant_grace_seconds: 1.0,
    };
    let mut argv: Vec<String> = [
        "/usr/bin/bwrap",
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-net",
        "--tmpfs",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for system in system_roots {
        argv.extend(["--ro-bind".into(), system.clone(), system.clone()]);
    }
    argv.extend([
        "--bind".into(),
        root.clone(),
        root.clone(),
        "--ro-bind".into(),
        gitfile.clone(),
        gitfile.clone(),
        "--chdir".into(),
        root.clone(),
        "--".into(),
        "/usr/bin/prlimit".into(),
        format!("--as={}", limits.address_space_bytes),
        format!("--fsize={}", limits.file_size_bytes),
        format!("--nofile={}", limits.open_file_limit),
        format!("--nproc={}", limits.process_count_limit),
        "--".into(),
    ]);
    argv.extend(policy.check.clone());
    Ok(ExecutionRequest {
        schema_version: 1,
        request_id: format!("check:{}", run.id),
        effect: "local-agent".into(),
        argv,
        working_directory: root.clone(),
        authority: ExecutionAuthority::LocalAgent {
            mission_id: mission.into(),
            session_id: run.id.clone(),
            session_revision: 1,
            runner_operation_id: format!("worker:{}", run.id),
            worktree_identity: format!("git:{}:{}", run.id, run.baseline),
            allowed_paths: policy.files.clone(),
        },
        limits,
        sandbox: ExecutionSandbox {
            mode: "bubblewrap".into(),
            readable_roots: vec![root.clone()],
            writable_roots: vec![root],
            readonly_bindings: vec![(gitfile.clone(), gitfile)],
        },
        environment: BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), "/tmp".into()),
            ("CI".into(), "1".into()),
        ]),
        input_text: None,
        input_sha256: None,
        shell: false,
    })
}

fn verify_intent(path: &Path, task: &Task, mission: &str, intent: &CheckIntent) -> Result<()> {
    let run = task.run.as_ref().ok_or("Missing run claim")?;
    if intent.schema_version != 2
        || intent.contract_version != 1
        || intent.task != task.id
        || intent.run != run.id
        || intent.baseline != run.baseline
        || intent.mission != mission
    {
        return Err("Check intent does not match the versioned task/run/Mission/baseline".into());
    }
    let expected = contract_request(
        managed_worktree(path, task)?,
        task,
        mission,
        &intent.system_roots,
    )?;
    if intent.request != expected
        || intent.request_digest != expected.request_digest().map_err(|e| e.message)?
    {
        return Err("Check intent differs from its exact authorized request".into());
    }
    Ok(())
}

fn timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 24
        || [4, 7].iter().any(|i| bytes[*i] != b'-')
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| ![4, 7, 10, 13, 16, 19, 23].contains(&i) && !b.is_ascii_digit())
    {
        return false;
    }
    let number = |start: usize, end: usize| value[start..end].parse::<u32>().unwrap_or(0);
    let year = number(0, 4);
    let month = number(5, 7);
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return false,
    };
    year > 0
        && (1..=days).contains(&number(8, 10))
        && number(11, 13) < 24
        && number(14, 16) < 60
        && number(17, 19) < 60
}

fn process_binding(pid: Option<u32>, identity: &str) -> bool {
    let Some(pid) = pid.filter(|pid| *pid > 0) else {
        return false;
    };
    if identity.len() > 128 {
        return false;
    }
    let fields: Vec<_> = identity.split(':').collect();
    matches!(fields.first(), Some(&"linux"))
        && fields.len() == 3
        && fields[1] == pid.to_string()
        && numeric_identity(fields[2])
        || matches!(fields.first(), Some(&"macos"))
            && fields.len() == 4
            && fields[1] == pid.to_string()
            && numeric_identity(fields[2])
            && numeric_identity(fields[3])
}

fn numeric_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 20
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}

fn verify_receipt(intent: &CheckIntent, receipt: &ExecutionReceipt) -> Result<()> {
    if receipt.schema_version != 1
        || receipt.provider != "rust-shadow"
        || receipt.request_id != intent.request.request_id
        || receipt.request_digest != intent.request_digest
        || receipt.effect != "local-agent"
        || !timestamp(&receipt.started_at)
        || !timestamp(&receipt.ended_at)
        || receipt.ended_at < receipt.started_at
        || !process_binding(receipt.owner_pid, &receipt.owner_identity)
        || receipt.error_code.len() > 128
        || receipt.error_message.len() > MAX_ERROR
        || receipt.error_code.chars().any(char::is_control)
        || receipt.error_message.contains('\0')
    {
        return Err("Check receipt identity or terminal metadata is invalid".into());
    }
    let expected_id = format!(
        "execution-receipt:{}",
        digest(
            format!(
                "{}\n{}\n{}\n{}\n{}",
                receipt.request_id,
                receipt.request_digest,
                receipt.started_at,
                receipt.ended_at,
                receipt.status
            )
            .as_bytes()
        )
    );
    if receipt.receipt_id != expected_id
        || receipt.stdout.len().saturating_add(receipt.stderr.len())
            > intent.request.limits.output_limit_bytes
        || receipt.stdout_bytes != receipt.stdout.len()
        || receipt.stderr_bytes != receipt.stderr.len()
        || receipt.stdout_sha256 != digest(receipt.stdout.as_bytes())
        || receipt.stderr_sha256 != digest(receipt.stderr.as_bytes())
        || receipt.stdout.contains('\0')
        || receipt.stderr.contains('\0')
    {
        return Err("Check receipt output or receipt digest is invalid".into());
    }
    let started =
        receipt.effect_started && process_binding(receipt.process_pid, &receipt.process_identity);
    let clean_exit = receipt.error_code.is_empty() && receipt.error_message.is_empty();
    let valid = match receipt.status.as_str() {
        "completed" => started && receipt.exit_code == Some(0) && clean_exit,
        "failed" => {
            started
                && receipt
                    .exit_code
                    .is_some_and(|code| code != 0 && (-255..=255).contains(&code))
                && clean_exit
        }
        "cancelled" => {
            started
                && receipt.exit_code.is_none()
                && receipt.error_code == "cancelled"
                && !receipt.error_message.is_empty()
        }
        "timed-out" => {
            started
                && receipt.exit_code == Some(124)
                && receipt.error_code == "timeout"
                && !receipt.error_message.is_empty()
        }
        "output-limit" => {
            started
                && receipt.exit_code == Some(125)
                && receipt.error_code == "output-limit"
                && !receipt.error_message.is_empty()
        }
        "start-failed" => {
            !receipt.effect_started
                && receipt.process_pid.is_none()
                && receipt.process_identity.is_empty()
                && receipt.exit_code == Some(127)
                && receipt.error_code == "provider-start-failed"
                && !receipt.error_message.is_empty()
                && receipt.stdout.is_empty()
                && receipt.stderr.is_empty()
        }
        // Preserve genuine uncertainty for normal worker diagnostics, but never
        // admit it through interrupted_after_check as recovery authority.
        "outcome-unknown" => {
            receipt.effect_started
                && receipt.exit_code.is_none()
                && receipt.reconciliation_required
                && receipt.process_pid.is_some_and(|pid| pid > 0)
                && (receipt.process_identity.is_empty()
                    || process_binding(receipt.process_pid, &receipt.process_identity))
                && matches!(
                    receipt.error_code.as_str(),
                    "outcome-unknown" | "cleanup-uncertain"
                )
                && !receipt.error_message.is_empty()
        }
        _ => false,
    };
    if !valid || (receipt.status != "outcome-unknown" && receipt.reconciliation_required) {
        return Err("Check receipt does not prove a supported provider outcome".into());
    }
    Ok(())
}

/// Invoke only inside the worker's blocking closure. A returned receipt has been
/// durably checkpointed; any publication/validation error forbids worker success.
pub fn execute_check(
    path: &Path,
    task: &Task,
    mission: &str,
    request: &ExecutionRequest,
    callbacks: &mut ExecutionCallbacks<'_>,
) -> Result<ExecutionReceipt> {
    if task.status != TaskStatus::Running {
        return Err("Task is not running".into());
    }
    verify_start(path, task)?;
    absent(&path.join("evidence.json"))?;
    absent(&path.join("check-result.json"))?;
    // Capture host-dependent choices now; recovery validates this recorded subset.
    let system_roots: Vec<String> = SYSTEM_ROOTS
        .into_iter()
        .filter(|root| Path::new(root).exists())
        .map(str::to_owned)
        .collect();
    if *request != contract_request(managed_worktree(path, task)?, task, mission, &system_roots)? {
        return Err("Check request differs from its exact authorized contract".into());
    }
    let run = task.run.as_ref().ok_or("Missing run claim")?;
    let intent = CheckIntent {
        schema_version: 2,
        contract_version: 1,
        task: task.id,
        run: run.id.clone(),
        baseline: run.baseline.clone(),
        mission: mission.into(),
        system_roots,
        request_digest: request.request_digest().map_err(|e| e.message)?,
        request: request.clone(),
    };
    verify_intent(path, task, mission, &intent)?;
    let bytes = write_bounded(&path.join("check-launch-intent.json"), &intent, MAX_INTENT)?;
    let receipt = RustExecutionProvider::new()
        .execute_with_callbacks(request, callbacks)
        .map_err(|e| e.message)?;
    verify_receipt(&intent, &receipt)?;
    write_bounded(
        &path.join("check-result.json"),
        &CheckResult {
            schema_version: 1,
            intent_sha256: digest(&bytes),
            request_digest: intent.request_digest,
            receipt: receipt.clone(),
        },
        MAX_RESULT,
    )?;
    Ok(receipt)
}

/// Read-only evidence projection. Ownership must be proven separately by caller.
pub(crate) fn interrupted_after_check(path: &Path, task: &Task, mission: &str) -> Result<Evidence> {
    if task.status != TaskStatus::Running {
        return Err("Task is not running".into());
    }
    verify_start(path, task)?;
    absent(&path.join("evidence.json"))?;
    let bytes = read_bounded(&path.join("check-launch-intent.json"), MAX_INTENT)?;
    let intent: CheckIntent =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid or legacy check intent")?;
    verify_intent(path, task, mission, &intent)?;
    let result: CheckResult =
        serde_json::from_slice(&read_bounded(&path.join("check-result.json"), MAX_RESULT)?)
            .map_err(|_| "Invalid check result checkpoint")?;
    if result.schema_version != 1
        || result.intent_sha256 != digest(&bytes)
        || result.request_digest != intent.request_digest
    {
        return Err("Check result does not bind the exact recorded intent".into());
    }
    verify_receipt(&intent, &result.receipt)?;
    if result.receipt.reconciliation_required || result.receipt.status == "outcome-unknown" {
        return Err("Check result remains uncertain; recovery refused".into());
    }
    Ok(Evidence {
        agent: None, run: intent.run, baseline: intent.baseline, status: TaskStatus::Failed,
        detail: "Worker interrupted after check; candidate not finalized. Check result retained; patch not reconstructed. Propose a repair and approve it separately; no effects replayed".into(),
        patch: String::new(), candidate_commit: None, model_metrics: None, generation: None,
        check: Some(result.receipt),
    })
}

/// Publish only under the stopped worker owner guard; existing evidence is immutable.
pub(crate) fn record_interrupted_after_check(
    path: &Path,
    task: &Task,
    mission: &str,
) -> Result<()> {
    let evidence = interrupted_after_check(path, task, mission)?;
    write_bounded(&path.join("evidence.json"), &evidence, MAX_RESULT + 4096).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{TaskRun, WorkPolicy};
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        path: PathBuf,
        task: Task,
        intent: CheckIntent,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "alfredo-check-boundary-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let path = root.join("runs/task-1-run");
            fs::create_dir_all(path.join("worktree")).unwrap();
            let root = root.canonicalize().unwrap();
            let path = path.canonicalize().unwrap();
            let task = Task {
                id: 1,
                title: "Check fixture".into(),
                model: "fixture".into(),
                dependencies: vec![],
                status: TaskStatus::Running,
                policy: Some(WorkPolicy {
                    files: vec!["result.txt".into()],
                    check: vec!["/usr/bin/true".into()],
                }),
                run: Some(TaskRun {
                    id: "task-1-run".into(),
                    baseline: "a".repeat(40),
                    inputs: vec![],
                    evidence_sha256: None,
                    detail: String::new(),
                }),
                repair_of: None,
            };
            let system_roots = vec!["/usr".into(), "/etc".into()];
            let request = contract_request(
                managed_worktree(&path, &task).unwrap(),
                &task,
                "mission",
                &system_roots,
            )
            .unwrap();
            let intent = CheckIntent {
                schema_version: 2,
                contract_version: 1,
                task: task.id,
                run: "task-1-run".into(),
                baseline: "a".repeat(40),
                mission: "mission".into(),
                system_roots,
                request_digest: request.request_digest().unwrap(),
                request,
            };
            record_start(&path, &task).unwrap();
            Self {
                root,
                path,
                task,
                intent,
            }
        }
        fn publish(&self, receipt: ExecutionReceipt) {
            let bytes = write_bounded(
                &self.path.join("check-launch-intent.json"),
                &self.intent,
                MAX_INTENT,
            )
            .unwrap();
            write_bounded(
                &self.path.join("check-result.json"),
                &CheckResult {
                    schema_version: 1,
                    intent_sha256: digest(&bytes),
                    request_digest: self.intent.request_digest.clone(),
                    receipt,
                },
                MAX_RESULT,
            )
            .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn reidentify(receipt: &mut ExecutionReceipt) {
        receipt.receipt_id = format!(
            "execution-receipt:{}",
            digest(
                format!(
                    "{}\n{}\n{}\n{}\n{}",
                    receipt.request_id,
                    receipt.request_digest,
                    receipt.started_at,
                    receipt.ended_at,
                    receipt.status
                )
                .as_bytes()
            )
        );
    }
    fn receipt(intent: &CheckIntent, status: &str) -> ExecutionReceipt {
        let mut receipt = ExecutionReceipt {
            schema_version: 1,
            request_id: intent.request.request_id.clone(),
            request_digest: intent.request_digest.clone(),
            effect: "local-agent".into(),
            status: status.into(),
            started_at: "2026-09-27T12:00:00.000Z".into(),
            ended_at: "2026-09-27T12:00:00.001Z".into(),
            exit_code: Some(0),
            stdout: "retained ✓\n".into(),
            stderr: String::new(),
            stdout_bytes: 13,
            stderr_bytes: 0,
            stdout_sha256: digest("retained ✓\n".as_bytes()),
            stderr_sha256: digest(b""),
            effect_started: true,
            reconciliation_required: false,
            error_code: String::new(),
            error_message: String::new(),
            receipt_id: String::new(),
            owner_pid: Some(101),
            owner_identity: "linux:101:12345".into(),
            process_pid: Some(102),
            process_identity: "linux:102:12346".into(),
            provider: "rust-shadow".into(),
        };
        receipt.stdout_bytes = receipt.stdout.len();
        match status {
            "failed" => receipt.exit_code = Some(1),
            "cancelled" => {
                receipt.exit_code = None;
                receipt.error_code = "cancelled".into();
                receipt.error_message = "User cancelled worker".into();
            }
            "timed-out" => {
                receipt.exit_code = Some(124);
                receipt.error_code = "timeout".into();
                receipt.error_message = "Process timed out after the bounded timeout.".into();
            }
            "output-limit" => {
                receipt.exit_code = Some(125);
                receipt.error_code = "output-limit".into();
                receipt.error_message = "Process output exceeded the bounded output limit.".into();
            }
            "start-failed" => {
                receipt.exit_code = Some(127);
                receipt.effect_started = false;
                receipt.process_pid = None;
                receipt.process_identity.clear();
                receipt.error_code = "provider-start-failed".into();
                receipt.error_message = "Cannot spawn provider".into();
                receipt.stdout.clear();
                receipt.stdout_bytes = 0;
                receipt.stdout_sha256 = digest(b"");
            }
            "outcome-unknown" => {
                receipt.exit_code = None;
                receipt.reconciliation_required = true;
                receipt.error_code = "outcome-unknown".into();
                receipt.error_message = "Cleanup uncertain".into();
            }
            _ => {}
        }
        reidentify(&mut receipt);
        receipt
    }

    #[test]
    fn supported_terminal_results_recover_only_failed_without_candidate() {
        for status in [
            "completed",
            "failed",
            "cancelled",
            "timed-out",
            "output-limit",
            "start-failed",
        ] {
            let fixture = Fixture::new();
            let check = receipt(&fixture.intent, status);
            verify_receipt(&fixture.intent, &check).unwrap();
            fixture.publish(check.clone());
            let evidence =
                interrupted_after_check(&fixture.path, &fixture.task, "mission").unwrap();
            assert_eq!(evidence.status, TaskStatus::Failed);
            assert!(evidence.patch.is_empty());
            assert!(evidence.candidate_commit.is_none());
            assert_eq!(evidence.check, Some(check));
            record_interrupted_after_check(&fixture.path, &fixture.task, "mission").unwrap();
            let saved = fs::read(fixture.path.join("evidence.json")).unwrap();
            assert!(
                record_interrupted_after_check(&fixture.path, &fixture.task, "mission").is_err()
            );
            assert_eq!(fs::read(fixture.path.join("evidence.json")).unwrap(), saved);
        }
    }

    #[test]
    fn uncertain_result_retains_diagnostics_but_cannot_recover() {
        let fixture = Fixture::new();
        let check = receipt(&fixture.intent, "outcome-unknown");
        verify_receipt(&fixture.intent, &check).unwrap();
        fixture.publish(check);
        let saved = fs::read(fixture.path.join("check-result.json")).unwrap();
        assert!(
            interrupted_after_check(&fixture.path, &fixture.task, "mission")
                .unwrap_err()
                .contains("uncertain")
        );
        assert!(record_interrupted_after_check(&fixture.path, &fixture.task, "mission").is_err());
        assert_eq!(
            fs::read(fixture.path.join("check-result.json")).unwrap(),
            saved
        );
        assert!(!fixture.path.join("evidence.json").exists());
    }

    #[test]
    fn receipt_validation_rejects_inconsistent_or_unbounded_metadata() {
        let fixture = Fixture::new();
        let original = receipt(&fixture.intent, "completed");
        let invalid = |edit: &dyn Fn(&mut ExecutionReceipt)| {
            let mut changed = original.clone();
            edit(&mut changed);
            assert!(
                verify_receipt(&fixture.intent, &changed).is_err(),
                "accepted {changed:?}"
            );
        };
        invalid(&|r| r.schema_version = 2);
        invalid(&|r| r.provider = "python".into());
        invalid(&|r| r.effect = "shell".into());
        invalid(&|r| r.request_id.push('x'));
        invalid(&|r| r.request_digest = "b".repeat(64));
        invalid(&|r| r.stdout_bytes -= 1);
        invalid(&|r| r.stderr_sha256 = "c".repeat(64));
        invalid(&|r| {
            r.stdout = "x".repeat(MAX_OUTPUT + 1);
            r.stdout_bytes = r.stdout.len();
            r.stdout_sha256 = digest(r.stdout.as_bytes());
        });
        invalid(&|r| {
            r.stdout = "\0".into();
            r.stdout_bytes = 1;
            r.stdout_sha256 = digest(b"\0");
        });
        invalid(&|r| r.effect_started = false);
        invalid(&|r| r.exit_code = Some(1));
        invalid(&|r| r.reconciliation_required = true);
        invalid(&|r| r.process_pid = None);
        invalid(&|r| r.process_identity = "linux:103:12346".into());
        invalid(&|r| r.owner_pid = Some(0));
        invalid(&|r| r.owner_identity.clear());
        invalid(&|r| r.error_code = "timeout".into());
        invalid(&|r| r.error_message = "x".repeat(MAX_ERROR + 1));
        invalid(&|r| {
            r.started_at = "2026-02-30T12:00:00.000Z".into();
            reidentify(r);
        });
        invalid(&|r| {
            r.ended_at = "2026-09-26T12:00:00.000Z".into();
            reidentify(r);
        });
        invalid(&|r| r.receipt_id.push('x'));
        for status in [
            "executing",
            "outcome-unknown",
            "cancelled",
            "timed-out",
            "output-limit",
            "start-failed",
            "failed",
        ] {
            invalid(&|r| {
                r.status = status.into();
                reidentify(r);
            });
        }
    }

    #[test]
    fn request_validation_pins_all_authority_and_execution_fields() {
        let mut fixture = Fixture::new();
        verify_intent(&fixture.path, &fixture.task, "mission", &fixture.intent).unwrap();
        let original = fixture.intent.request.clone();
        let mutations: [fn(&mut ExecutionRequest); 9] = [
            |r| r.argv.push("extra".into()),
            |r| r.working_directory.push_str("/other"),
            |r| {
                r.environment
                    .insert("HOME".into(), "/".into())
                    .map(|_| ())
                    .unwrap()
            },
            |r| r.limits.timeout_seconds = 61.0,
            |r| r.limits.output_limit_bytes += 1,
            |r| r.sandbox.writable_roots.push("/tmp".into()),
            |r| r.input_text = Some("input".into()),
            |r| r.shell = true,
            |r| {
                if let ExecutionAuthority::LocalAgent { allowed_paths, .. } = &mut r.authority {
                    allowed_paths.push("secret".into());
                }
            },
        ];
        for mutate in mutations {
            fixture.intent.request = original.clone();
            mutate(&mut fixture.intent.request);
            assert!(
                verify_intent(&fixture.path, &fixture.task, "mission", &fixture.intent).is_err()
            );
        }
        fixture.intent.request = original;
        assert!(verify_intent(&fixture.path, &fixture.task, "other", &fixture.intent).is_err());
        fixture.intent.contract_version = 2;
        assert!(verify_intent(&fixture.path, &fixture.task, "mission", &fixture.intent).is_err());
    }

    #[test]
    fn intent_bytes_and_new_artifact_bounds_are_independent_of_legacy_markers() {
        let mut fixture = Fixture::new();
        let policy = fixture.task.policy.as_mut().unwrap();
        policy.check = vec!["\"".repeat(2048); 32];
        policy.files = (0..32)
            .map(|i| format!("{i:02}{}", "a".repeat(510)))
            .collect();
        fixture.intent.request = contract_request(
            managed_worktree(&fixture.path, &fixture.task).unwrap(),
            &fixture.task,
            "mission",
            &fixture.intent.system_roots,
        )
        .unwrap();
        fixture.intent.request_digest = fixture.intent.request.request_digest().unwrap();
        verify_intent(&fixture.path, &fixture.task, "mission", &fixture.intent).unwrap();
        fixture.publish(receipt(&fixture.intent, "completed"));
        let intent_path = fixture.path.join("check-launch-intent.json");
        let mut bytes = fs::read(&intent_path).unwrap();
        assert!(bytes.len() > 4096 && bytes.len() <= MAX_INTENT);
        interrupted_after_check(&fixture.path, &fixture.task, "mission").unwrap();
        bytes.push(b'\n');
        fs::write(&intent_path, &bytes).unwrap();
        assert!(interrupted_after_check(&fixture.path, &fixture.task, "mission").is_err());
        assert_eq!(fs::read(&intent_path).unwrap(), bytes);
        let oversized_path = fixture.path.join("oversized.json");
        assert!(write_bounded(&oversized_path, &"x".repeat(100), 10).is_err());
        assert!(!oversized_path.exists());
    }

    #[test]
    fn legacy_and_partial_intents_never_gain_result_recovery() {
        let fixture = Fixture::new();
        record_check_intent(&fixture.path, &fixture.task).unwrap();
        assert!(interrupted_after_check(&fixture.path, &fixture.task, "mission").is_err());
        assert!(interrupted_before_check(&fixture.path, &fixture.task).is_err());
        fs::write(fixture.path.join("check-launch-intent.json"), b"{").unwrap();
        assert!(interrupted_after_check(&fixture.path, &fixture.task, "mission").is_err());
        assert!(!fixture.path.join("evidence.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_special_artifacts_are_refused_without_touching_targets() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let target = fixture.root.join("target");
        fs::write(&target, b"retained").unwrap();
        symlink(&target, fixture.path.join("check-result.json")).unwrap();
        assert!(read_bounded(&fixture.path.join("check-result.json"), MAX_RESULT).is_err());
        assert!(write_bounded(
            &fixture.path.join("check-result.json"),
            &fixture.intent,
            MAX_INTENT
        )
        .is_err());
        assert_eq!(fs::read(&target).unwrap(), b"retained");
        fs::create_dir(fixture.path.join("special")).unwrap();
        assert!(read_bounded(&fixture.path.join("special"), MAX_RESULT).is_err());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod publication_crash_tests {
    use super::*;
    use crate::tasks::{Action, Request, TaskStore, WorkPolicy};
    use std::{
        cell::Cell,
        io::{BufRead, BufReader},
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    const MISSION: &str = "publication-crash";
    const READY: &str = "CHECK_RESULT_PUBLICATION_CUT_REACHED";
    static NEXT: AtomicU64 = AtomicU64::new(0);
    thread_local! {
        // Only the ignored fixture enables this on its own execution thread.
        static AFTER_CREATE: Cell<Option<bool>> = const { Cell::new(None) };
    }

    pub(super) fn pause_result_write(path: &Path, bytes: &[u8], after_create: bool) {
        if path.file_name().and_then(|name| name.to_str()) != Some("check-result.json")
            || !AFTER_CREATE.with(|cut| cut.get() == Some(after_create))
        {
            return;
        }
        // These are the real provider's serialized bytes, before publication.
        let result: CheckResult = serde_json::from_slice(bytes).unwrap();
        assert_eq!(result.receipt.status, "completed");
        assert_eq!(result.receipt.exit_code, Some(0));
        assert!(result
            .receipt
            .stdout
            .contains("PUBLICATION_CHECK_COMPLETED"));
        println!("{READY}");
        std::io::stdout().flush().unwrap();
        loop {
            thread::park();
        }
    }

    fn open_store(root: &Path) -> TaskStore {
        TaskStore::new(&root.join("state"), &root.join("workspace"), MISSION).unwrap()
    }

    fn action(store: &TaskStore, action: Action) -> crate::tasks::Snapshot {
        let revision = store.snapshot().unwrap().revision;
        store
            .transact(Request {
                correlation: format!("publication-fixture-{revision}"),
                expected_revision: revision,
                action,
            })
            .unwrap()
            .0
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "alfredo-publication-crash-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("workspace")).unwrap();
            let fixture = Self(root);
            let store = open_store(&fixture.0);
            action(
                &store,
                Action::Propose {
                    title: "Count one completed check before interrupted publication".into(),
                    model: "fixture".into(),
                    dependencies: vec![],
                },
            );
            action(
                &store,
                Action::Permit {
                    task: 1,
                    policy: WorkPolicy {
                        files: vec!["count".into()],
                        check: vec![
                            "/usr/bin/python3".into(),
                            "-B".into(),
                            "-c".into(),
                            "from pathlib import Path; p=Path('count'); p.write_text(str(int(p.read_text())+1)); print('PUBLICATION_CHECK_COMPLETED')".into(),
                        ],
                    },
                },
            );
            action(&store, Action::Approve { task: 1 });
            fixture
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    #[ignore = "test-only subprocess for death during result publication"]
    fn result_publication_process_fixture() {
        let root = PathBuf::from(std::env::var_os("ALFREDO_PUBLICATION_CRASH_ROOT").unwrap());
        let after_create = match std::env::var("ALFREDO_PUBLICATION_CRASH_CUT")
            .unwrap()
            .as_str()
        {
            "before-create" => false,
            "after-create" => true,
            _ => panic!("Unknown publication crash cut"),
        };
        let store = open_store(&root);
        let _owner = store.claim_worker(1).unwrap();
        let running = action(
            &store,
            Action::Start {
                task: 1,
                baseline: "a".repeat(40),
                inputs: vec![],
            },
        );
        let task = &running.tasks[0];
        let directory = store.run_directory(&task.run.as_ref().unwrap().id).unwrap();
        let worktree = directory.join("worktree");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(worktree.join(".git"), "fixture git metadata\n").unwrap();
        fs::write(worktree.join("count"), "0").unwrap();
        record_start(&directory, task).unwrap();
        let request = crate::worker::check_request(&worktree, task, MISSION).unwrap();
        AFTER_CREATE.with(|cut| cut.set(Some(after_create)));
        execute_check(
            &directory,
            task,
            MISSION,
            &request,
            &mut ExecutionCallbacks::none(),
        )
        .unwrap();
        panic!("The real checkpoint writer did not reach its requested crash cut");
    }

    struct CrashedChild(Child);
    impl CrashedChild {
        fn spawn(fixture: &Fixture, cut: &str) -> Self {
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "run_boundary::publication_crash_tests::result_publication_process_fixture",
                    "--nocapture",
                ])
                .env("ALFREDO_PUBLICATION_CRASH_ROOT", &fixture.0)
                .env("ALFREDO_PUBLICATION_CRASH_CUT", cut)
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let mut child = Self(child);
            let stdout = child.0.stdout.take().unwrap();
            let (ready, wait) = mpsc::channel();
            thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    if line.is_ok_and(|line| line.contains(READY)) {
                        let _ = ready.send(());
                        break;
                    }
                }
            });
            wait.recv_timeout(Duration::from_secs(15))
                .expect("Child did not reach the result publication crash cut");
            child
        }

        fn kill(&mut self) {
            self.0.kill().unwrap();
            assert!(!self.0.wait().unwrap().success());
        }
    }
    impl Drop for CrashedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn death_after_provider_return_preserves_absent_or_incomplete_checkpoint_uncertainty() {
        for cut in ["before-create", "after-create"] {
            let fixture = Fixture::new();
            let mut child = CrashedChild::spawn(&fixture, cut);
            let store = open_store(&fixture.0);
            let running = store.snapshot().unwrap();
            let task = &running.tasks[0];
            assert_eq!(task.status, TaskStatus::Running);
            let directory = store.run_directory(&task.run.as_ref().unwrap().id).unwrap();
            let tasks_path = directory
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("tasks.json");
            let saved_state = fs::read(&tasks_path).unwrap();
            let result_path = directory.join("check-result.json");
            if cut == "before-create" {
                assert!(!result_path.exists());
            } else {
                // This is an incomplete file made by the production create_new,
                // with no fixture write, collision or substituted receipt.
                assert!(result_path.is_file());
                assert!(fs::read(&result_path).unwrap().is_empty());
            }
            assert_eq!(
                fs::read_to_string(directory.join("worktree/count")).unwrap(),
                "1"
            );
            assert!(!directory.join("evidence.json").exists());
            assert!(store.recover(1).unwrap_err().contains("still active"));
            let retained: Vec<_> = [
                "execution-boundary.json",
                "check-launch-intent.json",
                "check-result.json",
                "evidence.json",
                "worktree/count",
            ]
            .into_iter()
            .map(|name| (name, fs::read(directory.join(name)).ok()))
            .collect();
            child.kill();
            for _ in 0..2 {
                let reopened = open_store(&fixture.0);
                assert!(reopened.recover(1).unwrap_err().contains("Outcome unknown"));
                let still_running = reopened.snapshot().unwrap();
                assert_eq!(still_running.tasks[0].status, TaskStatus::Running);
                assert_eq!(still_running.revision, running.revision);
                assert!(!still_running
                    .receipts
                    .iter()
                    .any(|receipt| { matches!(receipt.request.action, Action::Finish { .. }) }));
                assert!(reopened.run_observations(&still_running)[&1].contains("outcome unknown"));
                assert_eq!(fs::read(&tasks_path).unwrap(), saved_state);
                for (name, bytes) in &retained {
                    assert_eq!(fs::read(directory.join(name)).ok(), *bytes, "{cut}: {name}");
                }
            }
        }
    }
}
