//! Compose accepted immutable results without changing a workspace branch or worktree.
use crate::{
    tasks::{DependencyInput, Snapshot, TaskStatus, TaskStore},
    worker::{git, verify_candidate, Evidence},
};
use std::path::Path;

pub async fn prepare(
    store: &TaskStore,
    snapshot: &Snapshot,
    task_id: u64,
    mut baseline: String,
) -> Result<(String, Vec<DependencyInput>), String> {
    let task = snapshot
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .ok_or("Unknown task")?;
    if task.status != TaskStatus::Approved || task.policy.is_none() {
        return Err("Task needs explicit policy and approval".into());
    }
    if let Some(parent_id) = task.repair_of {
        // A repair uses the exact baseline and inputs its parent actually ran with.
        // Later dependency resolutions must never silently rebase retained work.
        let parent = snapshot
            .tasks
            .iter()
            .find(|parent| parent.id == parent_id)
            .ok_or("Missing repair parent")?;
        let run = parent.run.as_ref().ok_or("Missing repair parent run")?;
        if baseline != run.baseline
            || run
                .inputs
                .iter()
                .map(|input| input.task)
                .collect::<Vec<_>>()
                != task.dependencies
        {
            return Err("Repair baseline and inputs must match its parent's recorded run".into());
        }
        for input in &run.inputs {
            let source = snapshot
                .tasks
                .iter()
                .find(|source| source.id == input.source_id())
                .ok_or("Missing recorded repair input source")?;
            let source_run = source
                .run
                .as_ref()
                .ok_or("Recorded repair input has no run")?;
            if source.status != TaskStatus::Accepted
                || source_run.id != input.run
                || source_run.evidence_sha256.as_ref() != Some(&input.evidence_sha256)
            {
                return Err("Recorded repair input is no longer the exact accepted result".into());
            }
            let evidence: Evidence = serde_json::from_str(&store.evidence(source.id)?)
                .map_err(|_| "Invalid recorded repair input evidence")?;
            if verify_candidate(&snapshot.workspace, &evidence).await? != input.candidate
                || !ancestor(&snapshot.workspace, &input.candidate, &baseline).await
            {
                return Err("Repair baseline does not contain its recorded accepted input".into());
            }
        }
        return Ok((baseline, run.inputs.clone()));
    }
    if task.dependencies.is_empty() {
        return Ok((baseline, vec![]));
    }
    let mut inputs = Vec::new();
    // Verify every input before creating any merge objects.
    for id in &task.dependencies {
        let parent = snapshot.dependency_source(*id)?;
        let evidence: Evidence = serde_json::from_str(&store.evidence(parent.id)?)
            .map_err(|_| "Invalid dependency evidence")?;
        let candidate = verify_candidate(&snapshot.workspace, &evidence).await?;
        let run = parent.run.as_ref().ok_or("Dependency has no run")?;
        inputs.push(DependencyInput {
            task: *id,
            source_task: (parent.id != *id).then_some(parent.id),
            run: run.id.clone(),
            evidence_sha256: run
                .evidence_sha256
                .clone()
                .ok_or("Missing dependency digest")?,
            candidate,
        });
    }
    let config = git(&snapshot.workspace, &["config", "--local", "--list"]).await?;
    if config.lines().any(|line| line.starts_with("merge.")) {
        return Err(
            "Custom merge configuration needs qualification before dependency composition".into(),
        );
    }
    for input in &inputs {
        if ancestor(&snapshot.workspace, &input.candidate, &baseline).await {
            continue;
        }
        if ancestor(&snapshot.workspace, &baseline, &input.candidate).await {
            baseline = input.candidate.clone();
            continue;
        }
        let tree = git(
            &snapshot.workspace,
            &["merge-tree", "--write-tree", &baseline, &input.candidate],
        )
        .await
        .map_err(|error| {
            format!(
                "Dependency #{} could not compose cleanly; task remains unstarted: {error}",
                input.task
            )
        })?;
        let tree = tree.lines().next().ok_or("Merge returned no tree")?;
        if tree.len() != 40 || !tree.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid composed tree identity".into());
        }
        baseline = git(
            &snapshot.workspace,
            &[
                "-c",
                "user.name=Alfredo",
                "-c",
                "user.email=alfredo@localhost",
                "-c",
                "commit.gpgSign=false",
                "commit-tree",
                tree,
                "-p",
                &baseline,
                "-p",
                &input.candidate,
                "-m",
                "Alfredo accepted dependency composition",
            ],
        )
        .await?
        .trim()
        .to_string();
    }
    for input in &inputs {
        if !ancestor(&snapshot.workspace, &input.candidate, &baseline).await {
            return Err("Composed baseline does not contain an accepted input".into());
        }
    }
    git(
        &snapshot.workspace,
        &[
            "update-ref",
            &format!("refs/alfredo/bases/{baseline}"),
            &baseline,
        ],
    )
    .await?;
    Ok((baseline, inputs))
}

async fn ancestor(workspace: &Path, ancestor: &str, descendant: &str) -> bool {
    git(
        workspace,
        &["merge-base", "--is-ancestor", ancestor, descendant],
    )
    .await
    .is_ok()
}
