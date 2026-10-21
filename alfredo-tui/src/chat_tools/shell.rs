//! Mutating tier (issue #236, ADR 0004): run_command runs one shell command after
//! owner Approval. By default it runs in the bubblewrap Sandbox shared with worker
//! checks ([`crate::worker::sandbox_argv`]): Coding Workspace writable, `.git` and
//! system roots read-only, network off. `escalate: true` runs it on the host
//! (network on) behind a visibly different card. Both run in their own process
//! group: a timeout kills the whole tree; output is capped. A sandbox that cannot
//! start is reported with a suggestion to escalate; nothing is retried on the host.
use super::{display, resolve, text_arg, Tier, Tool};
use crate::execution::{
    self, ExecutionAuthority, ExecutionLimits, ExecutionRequest, ExecutionSandbox,
    ProcessOutcomeStatus, RustExecutionProvider,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Default and largest timeout, in seconds.
const DEFAULT_TIMEOUT: u64 = 120;
const MAX_TIMEOUT: u64 = 600;
/// Largest command text, in bytes.
const MAX_COMMAND: usize = 8 * 1024;
/// Output kept from the process before it is stopped (the model sees less; see `cap`).
const OUTPUT_LIMIT: usize = 1024 * 1024;
/// Shown to the model when a sandboxed command fails.
const ESCALATE_HINT: &str = "note: ran sandboxed (network off; only the Coding Workspace is writable; .git and tools outside /usr are not available). If it failed for that reason, call run_command again with escalate: true; the owner must approve running it on the host.";

pub const RUN_COMMAND: Tool = Tool {
    name: "run_command",
    tier: Tier::Mutating,
    description: "Run one shell command (sh -c) in the Coding Workspace, such as tests or a build. Needs the owner's approval each call; the owner sees the exact command and cwd. Runs sandboxed by default: network off, only the working tree writable, .git read-only. Set escalate: true only when the command needs the network or the real host (git push, installing packages); the owner sees a host warning and may deny. Returns the exit code and output.",
    parameters: || {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Shell command, run with sh -c"},
                "cwd": {"type": "string", "description": "Directory relative to the repository root (default: the root)"},
                "escalate": {"type": "boolean", "description": "Run on the host with network on instead of the sandbox (default false)"},
                "timeout_seconds": {"type": "integer", "description": "Kill the command and its children after this many seconds (default 120, at most 600)"}
            },
            "required": ["command"]
        })
    },
    run: run_command,
};

/// A validated call.
struct Plan {
    command: String,
    cwd: PathBuf,
    shown_cwd: String,
    escalate: bool,
    timeout: u64,
}

/// What the approval card shows for a run_command call.
pub(crate) struct Shown {
    pub summary: String,
    pub preview: String,
    pub escalated: bool,
}

/// Card content, or why the call is refused before asking. `None` for other tools.
pub(crate) fn preview(root: &Path, name: &str, arguments: &Value) -> Option<Result<Shown, String>> {
    if name != RUN_COMMAND.name {
        return None;
    }
    Some(plan(root, arguments).map(|plan| {
        let place = if plan.escalate {
            "HOST · network on · outside the sandbox"
        } else {
            "sandboxed · network off · only the Coding Workspace writable"
        };
        let mut preview = format!(
            "cwd {} · {place} · timeout {}s",
            plan.shown_cwd, plan.timeout
        );
        for line in plan.command.lines() {
            preview.push_str(&format!("\n$ {line}"));
        }
        Shown {
            summary: format!(
                "{} {}",
                if plan.escalate { "run on HOST" } else { "run" },
                plan.command
            ),
            preview,
            escalated: plan.escalate,
        }
    }))
}

fn plan(root: &Path, arguments: &Value) -> Result<Plan, String> {
    let command = text_arg(arguments, "command")?
        .filter(|c| !c.trim().is_empty())
        .ok_or("argument command is required")?;
    if command.len() > MAX_COMMAND || command.contains('\0') {
        return Err(format!(
            "command must be at most {MAX_COMMAND} bytes of text"
        ));
    }
    let escalate = match arguments.get("escalate") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err("argument escalate must be true or false".into()),
    };
    let timeout = match arguments.get("timeout_seconds") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT,
        Some(value) => value
            .as_u64()
            .filter(|t| (1..=MAX_TIMEOUT).contains(t))
            .ok_or(format!(
                "argument timeout_seconds must be a whole number from 1 to {MAX_TIMEOUT}"
            ))?,
    };
    let cwd = match text_arg(arguments, "cwd")?.filter(|c| !c.trim().is_empty()) {
        Some(cwd) => resolve(root, cwd)?,
        None => root
            .canonicalize()
            .map_err(|_| "Coding Workspace is not available".to_string())?,
    };
    if !cwd.is_dir() {
        return Err("cwd is not a directory".into());
    }
    let shown_cwd = match display(root, &cwd) {
        shown if shown.is_empty() => ".".to_string(),
        shown => shown,
    };
    Ok(Plan {
        command: command.to_string(),
        cwd,
        shown_cwd,
        escalate,
        timeout,
    })
}

fn run_command(root: &Path, arguments: &Value) -> Result<String, String> {
    let plan = plan(root, arguments)?;
    if plan.escalate {
        on_host(&plan)
    } else {
        sandboxed(root, &plan, crate::doctor::BWRAP)
    }
}

/// How a run ended, from either launcher.
struct Ran {
    status: ProcessOutcomeStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn limits(timeout: u64) -> ExecutionLimits {
    ExecutionLimits {
        timeout_seconds: timeout as f64,
        output_limit_bytes: OUTPUT_LIMIT,
        address_space_bytes: 32 * 1024 * 1024 * 1024,
        file_size_bytes: 1024 * 1024 * 1024,
        open_file_limit: 4096,
        process_count_limit: 1024,
        descendant_grace_seconds: 1.0,
    }
}

fn request(
    argv: Vec<String>,
    cwd: String,
    plan: &Plan,
    sandbox: ExecutionSandbox,
) -> ExecutionRequest {
    ExecutionRequest {
        schema_version: execution::EXECUTION_SCHEMA_VERSION,
        request_id: "chat-run-command".into(),
        effect: "local-agent".into(),
        argv,
        working_directory: cwd,
        authority: ExecutionAuthority::LocalAgent {
            mission_id: "chat".into(),
            session_id: "chat".into(),
            session_revision: 1,
            runner_operation_id: "chat:run_command".into(),
            worktree_identity: "chat:workspace".into(),
            allowed_paths: Vec::new(),
        },
        limits: limits(plan.timeout),
        sandbox,
        environment: BTreeMap::new(),
        input_text: None,
        input_sha256: None,
        shell: false,
    }
}

fn sandbox_failure(reason: &str) -> String {
    format!(
        "the sandbox could not run the command ({}); nothing ran on the host. If the command must run, call run_command again with escalate: true and the owner will be asked to approve running it on the host.",
        reason.trim()
    )
}

/// Run in the bubblewrap Sandbox at `bwrap`: the worker check boundary rooted at the
/// Coding Workspace, then `cd` to the cwd inside it.
fn sandboxed(root: &Path, plan: &Plan, bwrap: &str) -> Result<String, String> {
    if !Path::new(bwrap).is_file() {
        return Err(sandbox_failure(&format!(
            "bubblewrap is not installed at {bwrap}"
        )));
    }
    let root = root
        .canonicalize()
        .map_err(|_| "Coding Workspace is not available".to_string())?;
    let root_text = root
        .to_str()
        .ok_or("Coding Workspace path is not UTF-8")?
        .to_string();
    let git = root.join(".git");
    let gitfile = git.exists().then(|| git.to_string_lossy().into_owned());
    let mut argv =
        crate::worker::sandbox_argv(&root_text, gitfile.as_deref(), &limits(plan.timeout));
    argv[0] = bwrap.to_string();
    argv.extend([
        "/bin/sh".into(),
        "-c".into(),
        "cd -- \"$1\" && exec /bin/sh -c \"$2\"".into(),
        "alfredo".into(),
        plan.cwd.to_string_lossy().into_owned(),
        plan.command.clone(),
    ]);
    let mut request = request(
        argv,
        root_text.clone(),
        plan,
        ExecutionSandbox {
            mode: "bubblewrap".into(),
            readable_roots: vec![root_text.clone()],
            writable_roots: vec![root_text],
            readonly_bindings: gitfile.iter().map(|g| (g.clone(), g.clone())).collect(),
        },
    );
    request.environment = BTreeMap::from([
        ("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()),
        ("HOME".into(), "/tmp".into()),
        ("CI".into(), "1".into()),
    ]);
    let receipt = RustExecutionProvider::new()
        .execute(&request)
        .map_err(|failure| sandbox_failure(&failure.message))?;
    if execution::sandbox_unavailable_receipt(&receipt) {
        let reason = if receipt.stderr.trim().is_empty() {
            receipt.error_message.clone()
        } else {
            receipt.stderr.clone()
        };
        return Err(sandbox_failure(&reason));
    }
    let status = match receipt.status.as_str() {
        "completed" | "failed" => ProcessOutcomeStatus::Completed,
        "timed-out" => ProcessOutcomeStatus::TimedOut,
        "output-limit" => ProcessOutcomeStatus::OutputLimit,
        "cancelled" => ProcessOutcomeStatus::Cancelled,
        _ => ProcessOutcomeStatus::Unknown,
    };
    if status == ProcessOutcomeStatus::Unknown {
        return Err(format!(
            "the sandboxed command did not finish cleanly: {}",
            receipt.error_message
        ));
    }
    report(
        plan,
        Ran {
            status,
            exit_code: receipt.exit_code,
            stdout: receipt.stdout,
            stderr: receipt.stderr,
        },
    )
}

/// Run on the host (owner-approved escalation): the owner's environment, network on.
fn on_host(plan: &Plan) -> Result<String, String> {
    let cwd = plan.cwd.to_string_lossy().into_owned();
    let mut request = request(
        vec!["/bin/sh".into(), "-c".into(), plan.command.clone()],
        cwd,
        plan,
        ExecutionSandbox {
            mode: "none".into(),
            readable_roots: Vec::new(),
            writable_roots: Vec::new(),
            readonly_bindings: Vec::new(),
        },
    );
    request.environment = std::env::vars().collect();
    let outcome = execution::run_on_host(&request)
        .map_err(|error| format!("could not start the command: {}", error.failure.message))?;
    if outcome.status == ProcessOutcomeStatus::Unknown {
        return Err(format!(
            "the command did not finish cleanly: {}",
            outcome.error_message
        ));
    }
    report(
        plan,
        Ran {
            status: outcome.status,
            exit_code: outcome.exit_code,
            stdout: String::from_utf8_lossy(&outcome.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&outcome.stderr).into_owned(),
        },
    )
}

/// Exit code first, then output; a timeout is an error (the tree was killed).
fn report(plan: &Plan, ran: Ran) -> Result<String, String> {
    let place = if plan.escalate { "host" } else { "sandboxed" };
    let output = format!(
        "stdout:\n{}\nstderr:\n{}",
        ran.stdout.trim_end(),
        ran.stderr.trim_end()
    );
    match ran.status {
        ProcessOutcomeStatus::TimedOut => Err(format!(
            "timed out after {}s ({place}); the command and its child processes were killed\n{output}",
            plan.timeout
        )),
        ProcessOutcomeStatus::Cancelled => Err(format!("cancelled ({place})\n{output}")),
        status => {
            let code = ran
                .exit_code
                .map_or("killed by a signal".to_string(), |c| format!("exit code {c}"));
            let mut text = format!("{code} · {place} · cwd {}", plan.shown_cwd);
            if status == ProcessOutcomeStatus::OutputLimit {
                text.push_str(&format!(
                    "\n[stopped: output passed {OUTPUT_LIMIT} bytes; the process tree was killed]"
                ));
            }
            if !plan.escalate && ran.exit_code != Some(0) {
                text.push('\n');
                text.push_str(ESCALATE_HINT);
            }
            Ok(format!("{text}\n{output}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_bubblewrap_suggests_escalation_and_runs_nothing() {
        let root =
            std::env::temp_dir().join(format!("alfredo-shell-nobwrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let plan = plan(&root, &json!({"command": "touch ran"})).unwrap();
        let error = sandboxed(&root, &plan, "/nonexistent/bwrap").unwrap_err();
        assert!(error.contains("escalate: true"), "{error}");
        assert!(error.contains("nothing ran on the host"), "{error}");
        assert!(!root.join("ran").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
