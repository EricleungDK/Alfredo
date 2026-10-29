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

/// Why coding workers cannot start, or `None` when the sandbox is usable.
/// `ALFREDO_TEST_BWRAP_PATH` redirects only this probe (a test seam for hosts
/// that have bubblewrap); it never changes the executable workers run.
pub fn sandbox_blocker() -> Option<String> {
    let path = std::env::var("ALFREDO_TEST_BWRAP_PATH").unwrap_or_else(|_| BWRAP.to_owned());
    sandbox_blocker_at(&path)
}

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
            model = snapshot.sessions[snapshot.selected].model.clone();
        }
        Ok(())
    })();
    match storage {
        Err(_) if root_check.is_err() => lines
            .push("SKIP storage: not checked until the workspace is a Git repository root".into()),
        Ok(()) => {
            lines.push("PASS storage: task state and named conversation can be opened".into())
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
    if !cfg!(target_os = "linux") {
        passed = false;
        lines.push("FAIL worker platform: coding execution currently requires Linux.".into());
    }
    lines.push(match provider.num_ctx() {
        Some(window) => format!("NOTE context: --num-ctx {window} tokens is sent on every request; prompts estimated above it are refused before sending"),
        None => "NOTE context: not set: Ollama default applies; set --num-ctx for prompts above ~4k tokens".into(),
    });
    lines.push("Checks do not run inference or a sandboxed task, measure model speed, or prove available GPU memory. Actual sandbox permissions are checked when a worker runs.".into());
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
