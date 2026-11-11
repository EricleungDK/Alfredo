//! Startup diagnostics through the same storage and provider paths as the terminal.
use crate::{conversations::ConversationStore, provider::Ollama, tasks::TaskStore};
use std::path::Path;

pub struct Report {
    pub passed: bool,
    pub text: String,
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// True when `path` is an executable regular file. The one probe behind the
/// doctor's installed-tool checks and the runtime sandbox preflight.
#[must_use]
pub fn worker_tool_ready(path: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            metadata.is_file()
        }
    })
}

/// Trusted sandbox executable that every coding worker is launched through.
pub const BWRAP: &str = "/usr/bin/bwrap";

/// Fix hint shown when coding workers cannot start because bubblewrap is missing.
pub const BWRAP_FIX: &str = "Coding workers need bubblewrap: sudo apt install bubblewrap";

/// Fix hint when bubblewrap is installed but the host refuses to let it build a sandbox.
pub const BWRAP_DENIED_FIX: &str = "Allow bubblewrap to create user namespaces per your distro policy (README Troubleshooting: Worker fails at sandbox start)";

/// Arguments of the no-op sandbox probe: the README's manual test. It is a separate
/// `true` run and never shapes the worker invocation.
const PROBE_ARGS: [&str; 7] = [
    "--ro-bind",
    "/",
    "/",
    "--unshare-user",
    "--unshare-pid",
    "--unshare-net",
    "true",
];
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The bubblewrap path the preflight inspects. `ALFREDO_TEST_BWRAP_PATH` redirects
/// only this probe (a test seam for hosts that have bubblewrap); it never changes
/// the executable workers run.
#[must_use]
pub fn probe_path() -> String {
    std::env::var("ALFREDO_TEST_BWRAP_PATH").unwrap_or_else(|_| BWRAP.to_owned())
}

/// The fix to quote for a sandbox-start failure: the install hint only when the
/// file is missing, otherwise the host-policy hint.
#[must_use]
pub fn sandbox_start_hint(installed: bool) -> &'static str {
    if installed {
        BWRAP_DENIED_FIX
    } else {
        BWRAP_FIX
    }
}

/// Run the no-op sandbox with `path`: `Err` carries bwrap's own words.
fn run_probe(path: &str) -> Result<(), String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(path)
        .args(PROBE_ARGS)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("bwrap could not start: {error}"))?;
    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("bwrap did not finish the sandbox probe within 5 s".into());
            }
            Err(error) => return Err(format!("bwrap probe failed: {error}")),
        }
    };
    if status.success() {
        return Ok(());
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.by_ref().take(2048).read_to_string(&mut stderr);
    }
    let words = clean_line(&stderr);
    Err(if words.is_empty() {
        format!("bwrap exited with {status} while creating a test sandbox")
    } else {
        words
    })
}

/// Control characters become spaces, runs collapse, and the text is bounded.
fn clean_line(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect()
}

/// Whether `path` can create a sandbox. `cached` shares one successful probe per path
/// for the life of the process (`/go`); a failed probe is never cached, so it is retried
/// on the next call. `--doctor` always probes fresh.
///
/// # Errors
///
/// Returns bwrap's own words (or why it could not start or finish) when the probe fails.
pub fn sandbox_probe_at(path: &str, cached: bool) -> Result<(), String> {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock, PoisonError};
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    if !cached {
        return run_probe(path);
    }
    let seen = SEEN.get_or_init(Default::default);
    if seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(path)
    {
        return Ok(());
    }
    run_probe(path)?;
    seen.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(path.to_owned());
    Ok(())
}

/// The refusal text for a bubblewrap that is installed but cannot start a sandbox.
#[must_use]
pub fn sandbox_denied_message(stderr: &str) -> String {
    format!(
        "{SANDBOX_REFUSAL_PREFIX} {}. {BWRAP_DENIED_FIX}",
        stderr.trim_end_matches(['.', '\n'])
    )
}

/// Start of every [`sandbox_denied_message`].
const SANDBOX_REFUSAL_PREFIX: &str = "Coding workers cannot start a sandbox:";

/// How the interactive TUI shows a [`sandbox_denied_message`] that the one-line footer
/// would cut before its fix: the footer leads with the fix, and the chat rows carry the
/// cause and the whole fix (#608). `None` for any other message.
#[must_use]
pub fn sandbox_refusal_view(message: &str, command: &str) -> Option<(String, Vec<String>)> {
    let cause = message
        .strip_prefix(SANDBOX_REFUSAL_PREFIX)?
        .strip_suffix(BWRAP_DENIED_FIX)?;
    Some((
        format!("{command} refused: allow bubblewrap user namespaces · see README Troubleshooting"),
        vec![
            format!(
                "✗ Not started · {SANDBOX_REFUSAL_PREFIX}{}",
                cause.trim_end()
            ),
            format!("  {BWRAP_DENIED_FIX}"),
        ],
    ))
}

/// Why coding workers cannot start, or `None` when the sandbox is usable: bubblewrap
/// is missing (install hint), or it exists but cannot create a sandbox (probed once
/// per process; host-policy hint).
#[must_use]
pub fn sandbox_blocker() -> Option<String> {
    let path = probe_path();
    sandbox_blocker_at(&path).or_else(|| {
        sandbox_probe_at(&path, true)
            .err()
            .map(|stderr| sandbox_denied_message(&stderr))
    })
}

/// The installed-tool half of the preflight: only a missing or non-executable file.
#[must_use]
pub fn sandbox_blocker_at(path: &str) -> Option<String> {
    (!worker_tool_ready(path)).then(|| BWRAP_FIX.to_owned())
}

/// Why the model server check failed.
pub enum ModelCheck {
    /// The catalog was read but does not list the model; carries what is installed.
    Missing(Vec<String>),
    Unreachable(String),
}

/// The endpoint is reachable and lists `model`; shared by `--doctor` and `--headless`.
pub async fn model_server_check(provider: &Ollama, model: &str) -> Result<(), ModelCheck> {
    match provider.models().await {
        Ok(models) if crate::provider::model_installed(&models, model) => Ok(()),
        Ok(models) => Err(ModelCheck::Missing(models)),
        Err(error) => Err(ModelCheck::Unreachable(error)),
    }
}

/// One-line setup error for a failed [`model_server_check`] (headless).
#[must_use]
pub fn model_check_message(model: &str, failure: &ModelCheck) -> String {
    match failure {
        ModelCheck::Missing(models) => format!(
            "model {} is not installed on this Ollama server (installed: {}). Install it or choose --model",
            clean(model),
            models.iter().take(12).map(|m| clean(m)).collect::<Vec<_>>().join(", ")
        ),
        ModelCheck::Unreachable(error) => format!(
            "cannot reach Ollama: {}. Check Ollama is running and --endpoint points to its HTTP origin",
            clean(error)
        ),
    }
}

pub async fn inspect(
    workspace: &Path,
    state: &Path,
    mission: &str,
    conversation: &str,
    initial_model: &str,
    explicit_model: bool,
    provider: &Ollama,
) -> Report {
    let mut passed = true;
    let mut lines = vec!["Alfredo startup diagnostics".to_string()];
    let mut model = initial_model.to_string();
    // Resolve the repository root first, as the startup path does: a subdirectory of a
    // repository opens its root, and the storage check only means something against it.
    let root_check = async {
        let root = crate::worker::git(workspace, &["rev-parse", "--show-toplevel"]).await?;
        Path::new(root.trim())
            .canonicalize()
            .map_err(|error| error.to_string())
    }
    .await;
    let storage = (|| -> Result<(), String> {
        let root = root_check.clone()?;
        let store = TaskStore::new(state, &root, mission)?;
        store.snapshot()?;
        let conversations = ConversationStore::open(&store, conversation)?;
        // An explicit --model is the run's model; otherwise the restored one is checked.
        if let (false, Some(snapshot)) = (explicit_model, conversations.load()?) {
            model.clone_from(&snapshot.sessions[snapshot.selected].model);
        }
        Ok(())
    })();
    match storage {
        Err(_) if root_check.is_err() => lines
            .push("SKIP storage: not checked until the workspace is a Git repository root".into()),
        Ok(()) => {
            lines.push("PASS storage: task state and named conversation can be opened".into());
        }
        Err(error) => {
            passed = false;
            lines.push(format!("FAIL storage: {}. Check --workspace, --state-dir and --conversation; preserve existing state when investigating.", clean(&error)));
        }
    }
    match model_server_check(provider, &model).await {
        Ok(()) => lines.push(format!(
            "PASS model catalog: {} is installed",
            clean(&model)
        )),
        Err(ModelCheck::Missing(models)) => {
            passed = false;
            lines.push(format!("FAIL model catalog: {} is not listed. Select an installed model or install the intended model on this Ollama server.", clean(&model)));
            lines.push(format!(
                "Installed models: {}",
                models
                    .iter()
                    .take(12)
                    .map(|m| clean(m))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            if models.len() > 12 {
                lines.push("Additional models omitted; use /models for the full catalog.".into());
            }
        }
        Err(ModelCheck::Unreachable(error)) => {
            passed = false;
            lines.push(format!("FAIL model server: {}. Check Ollama is running and --endpoint points to its HTTP origin.", clean(&error)));
        }
    }
    let git = match root_check {
        Err(error) => Err(error),
        Ok(root) => {
            async {
                let workspace = root.as_path();
                if !crate::worker::has_head_commit(workspace) {
                    return Err(crate::worker::NO_COMMITS.to_string());
                }
                let config =
                    crate::worker::git(workspace, &crate::worker::REPOSITORY_CONFIG_ARGS).await?;
                if let Some(key) = crate::worker::qualification_key(&config) {
                    return Err(crate::worker::config_refusal(key));
                }
                Ok::<_, String>(root)
            }
            .await
        }
    };
    match git {
        Ok(root) => lines.push(format!(
            "PASS worker workspace: {} (Git root with a committed baseline)",
            clean(&root.display().to_string())
        )),
        Err(error) => {
            passed = false;
            if error == crate::worker::NO_COMMITS
                || error.contains(crate::worker::NOT_A_REPOSITORY)
                || error.starts_with(crate::worker::CONFIG_REFUSAL_PREFIX)
            {
                lines.push(format!("FAIL worker workspace: {error}"));
            } else {
                lines.push(format!(
                    "FAIL worker workspace: {}. Coding needs a Git repository with a commit.",
                    clean(&error)
                ));
            }
        }
    }
    for path in ["/usr/bin/git", BWRAP, "/usr/bin/prlimit"] {
        let executable = worker_tool_ready(path);
        if executable {
            lines.push(format!("PASS installed worker tool: {path}"));
        } else {
            passed = false;
            lines.push(format!("FAIL installed worker tool: {path}. Install this dependency before coding execution."));
        }
    }
    let bwrap = probe_path();
    if worker_tool_ready(&bwrap) {
        match sandbox_probe_at(&bwrap, false) {
            Ok(()) => lines.push("PASS sandbox: bubblewrap created a test sandbox".into()),
            Err(stderr) => {
                passed = false;
                lines.push(format!("FAIL sandbox: {}", sandbox_denied_message(&stderr)));
            }
        }
    }
    if !cfg!(target_os = "linux") {
        passed = false;
        lines.push("FAIL worker platform: coding execution currently requires Linux.".into());
    }
    lines.push(match provider.num_ctx() {
        Some(window) => format!("NOTE context: --num-ctx {window} tokens is sent on every request; prompts estimated above it are refused before sending"),
        None => "NOTE context: not set: Ollama default applies; set --num-ctx for prompts above ~4k tokens".into(),
    });
    lines.push("Checks do not run inference or a sandboxed task, measure model speed, or prove available GPU memory. The sandbox check only creates an empty test sandbox.".into());
    lines.push("Storage checks may initialize private directories/lock files; they do not save conversations or change task receipts.".into());
    lines.push(
        if passed {
            "Preflight checks passed."
        } else {
            "Preflight found issues; follow the failing checks above."
        }
        .into(),
    );
    Report {
        passed,
        text: lines.join("\n"),
    }
}
