use super::Changes;
use crate::tasks::TaskStore;
use std::path::{Path, PathBuf};

/// Compose accepted candidates on the recorded baseline with the same object-only
/// merge used for dependency baselines, then create one new local branch ref.
pub(super) async fn integration_branch(
    store: TaskStore,
    workspace: PathBuf,
    baseline: String,
    sources: Vec<u64>,
    name: String,
    goal: String,
) -> Result<String, String> {
    let mut candidates = Vec::new();
    for source in sources {
        let reader = store.clone();
        let raw = tokio::task::spawn_blocking(move || reader.evidence(source))
            .await
            .map_err(|_| "Evidence reader stopped".to_string())??;
        let evidence: crate::worker::Evidence =
            serde_json::from_str(&raw).map_err(|_| "Malformed accepted evidence")?;
        candidates.push((
            source,
            crate::worker::verify_candidate(&workspace, &evidence).await?,
        ));
    }
    let composed = crate::dependencies::compose(&workspace, baseline, &candidates).await?;
    // The tip names the goal; its tree is the composed tree, so the content is unchanged.
    let subject = crate::worker::commit_line(&goal, 72);
    let subject = if subject.is_empty() {
        "Alfredo autopilot".to_string()
    } else {
        subject
    };
    let message = format!(
        "{subject}\n\nAlfredo autopilot integration of {} accepted task{}",
        candidates.len(),
        if candidates.len() == 1 { "" } else { "s" }
    );
    let commit = crate::worker::git(
        &workspace,
        &[
            "-c",
            "user.name=Alfredo",
            "-c",
            "user.email=alfredo@localhost",
            "-c",
            "commit.gpgSign=false",
            "commit-tree",
            &format!("{composed}^{{tree}}"),
            "-p",
            &composed,
            "-m",
            &message,
        ],
    )
    .await?
    .trim()
    .to_string();
    let reference = format!("refs/heads/{name}");
    match crate::worker::git(&workspace, &["show-ref", "--verify", "--hash", &reference]).await {
        Ok(current) if current.trim() == commit => {}
        // A retry after a restart rebuilds the tip with a new timestamp. Accept a ref
        // sitting on the same composed tip (goal commit on top) or at it (a branch
        // made before the goal commit existed).
        Ok(current)
            if current.trim() == composed
                || crate::worker::git(
                    &workspace,
                    &["rev-parse", &format!("{}^", current.trim())],
                )
                .await
                .is_ok_and(|parent| parent.trim() == composed) =>
        {
            return Ok(current.trim().to_string());
        }
        Ok(_) => {
            return Err(format!(
                "{name} already exists and points elsewhere; it was not changed"
            ))
        }
        Err(_) => {
            crate::worker::git(
                &workspace,
                &[
                    "update-ref",
                    "--no-deref",
                    &reference,
                    &commit,
                    "0000000000000000000000000000000000000000",
                ],
            )
            .await?;
        }
    }
    Ok(commit)
}

/// The totals of `git diff --numstat baseline..commit` as JSON text, through the
/// hardened host Git helper; `None` when Git cannot say, so the row is left out.
pub(super) async fn branch_changes(
    workspace: &Path,
    baseline: &str,
    commit: &str,
) -> Option<String> {
    let range = format!("{baseline}..{commit}");
    let numstat = crate::worker::git(
        workspace,
        &[
            "diff",
            "--numstat",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            &range,
            "--",
        ],
    )
    .await
    .ok()?;
    serde_json::to_string(&Changes::from_numstat(&numstat)).ok()
}
