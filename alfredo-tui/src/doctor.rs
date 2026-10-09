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

pub async fn inspect(
    workspace: &Path,
    state: &Path,
    mission: &str,
    conversation: &str,
    initial_model: &str,
    provider: &Ollama,
) -> Report {
    let mut passed = true;
    let mut lines = vec!["Alfredo startup diagnostics".to_string()];
    let mut model = initial_model.to_string();
    let storage = (|| -> Result<(), String> {
        let store = TaskStore::new(state, workspace, mission)?;
        store.snapshot()?;
        let conversations = ConversationStore::open(&store, conversation)?;
        if let Some(snapshot) = conversations.load()? {
            model = snapshot.sessions[snapshot.selected].model.clone();
        }
        Ok(())
    })();
    match storage {
        Ok(()) => {
            lines.push("PASS storage: task state and named conversation can be opened".into())
        }
        Err(error) => {
            passed = false;
            lines.push(format!("FAIL storage: {}. Check --workspace, --state-dir and --conversation; preserve existing state when investigating.", clean(&error)));
        }
    }
    match provider.models().await {
        Ok(models) if crate::provider::model_installed(&models, &model) => lines.push(format!(
            "PASS model catalog: {} is installed",
            clean(&model)
        )),
        Ok(models) => {
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
        Err(error) => {
            passed = false;
            lines.push(format!("FAIL model server: {}. Check Ollama is running and --endpoint points to its HTTP origin.", clean(&error)));
        }
    }
    let git = async {
        let root = crate::worker::git(workspace, &["rev-parse", "--show-toplevel"]).await?;
        let expected = workspace
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if Path::new(root.trim())
            .canonicalize()
            .map_err(|error| error.to_string())?
            != expected
        {
            return Err("Choose the repository root with --workspace".to_string());
        }
        if !crate::worker::has_head_commit(workspace) {
            return Err(crate::worker::NO_COMMITS.to_string());
        }
        let config = crate::worker::git(workspace, &["config", "--local", "--list"]).await?;
        if config.lines().any(|line| {
            line.starts_with("filter.")
                || line.starts_with("include.")
                || line.starts_with("includeif.")
        }) {
            return Err(
                "Repository checkout filters/includes need qualification before worker execution"
                    .into(),
            );
        }
        Ok::<_, String>(())
    }
    .await;
    match git {
        Ok(()) => lines.push("PASS worker workspace: Git root with a committed baseline".into()),
        Err(error) => {
            passed = false;
            if error == crate::worker::NO_COMMITS {
                lines.push(format!("FAIL worker workspace: {error}"));
            } else {
                lines.push(format!("FAIL worker workspace: {}. Coding needs --workspace at a Git root with a commit; conversation-only use remains available.", clean(&error)));
            }
        }
    }
    for path in ["/usr/bin/git", "/usr/bin/bwrap", "/usr/bin/prlimit"] {
        let executable = std::fs::metadata(path).is_ok_and(|metadata| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                metadata.is_file()
            }
        });
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
