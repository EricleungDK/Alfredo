//! Local branch handoff. Git's exact ref is observable if the task receipt is interrupted.
use crate::{
    tasks::{Action, Request, Snapshot, TaskStatus, TaskStore},
    worker::{git, verify_candidate, Evidence},
};
use std::time::Duration;

pub fn name(task: u64, commit: &str) -> Result<String, String> {
    if task == 0 || commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid branch candidate identity".into());
    }
    Ok(format!("alfredo/task-{task}-{}", &commit[..12]))
}

pub async fn publish(
    store: TaskStore,
    task: u64,
    expected_revision: u64,
    correlation: String,
) -> Result<(Snapshot, String), String> {
    let snapshot = store.snapshot()?;
    let item = snapshot
        .tasks
        .iter()
        .find(|item| item.id == task)
        .ok_or("Unknown task")?;
    if item.status != TaskStatus::Accepted {
        return Err("Review and accept this task before /branch".into());
    }
    let evidence: Evidence =
        serde_json::from_str(&store.evidence(task)?).map_err(|_| "Malformed branch evidence")?;
    let commit = verify_candidate(&snapshot.workspace, &evidence).await?;
    let name = name(task, &commit)?;
    let action = Action::Branch {
        task,
        name: name.clone(),
        commit: commit.clone(),
    };
    if let Some(receipt) = snapshot
        .receipts
        .iter()
        .find(|receipt| receipt.request.correlation == correlation)
    {
        if receipt.request.action != action {
            return Err("Branch correlation belongs to another request".into());
        }
    } else if snapshot.revision != expected_revision {
        return Err("Task state changed; refresh before creating a review branch".into());
    }
    store.preflight_branch(Request {
        correlation: correlation.clone(),
        expected_revision: snapshot.revision,
        action: action.clone(),
    })?;
    let reference = format!("refs/heads/{name}");
    if git(&snapshot.workspace, &["symbolic-ref", "-q", &reference])
        .await
        .is_ok()
    {
        return Err("Review branch name is a symbolic ref; it was not changed".into());
    }
    match git(
        &snapshot.workspace,
        &["show-ref", "--verify", "--hash", &reference],
    )
    .await
    {
        Ok(current) if current.trim() != commit => {
            return Err("Review branch already points elsewhere; it was not changed".into())
        }
        Ok(_) => {}
        Err(_) => {
            // Compare-and-set from an absent ref; never overwrite another branch.
            if let Err(error) = git(
                &snapshot.workspace,
                &[
                    "update-ref",
                    "--no-deref",
                    &reference,
                    &commit,
                    "0000000000000000000000000000000000000000",
                ],
            )
            .await
            {
                let current = git(
                    &snapshot.workspace,
                    &["show-ref", "--verify", "--hash", &reference],
                )
                .await?;
                if current.trim() != commit {
                    return Err(format!(
                        "Branch creation not confirmed; /branch {task} can recheck: {error}"
                    ));
                }
            }
        }
    }
    let actual = git(
        &snapshot.workspace,
        &["show-ref", "--verify", "--hash", &reference],
    )
    .await?;
    if actual.trim() != commit {
        return Err("Review branch changed before confirmation".into());
    }
    // Only receipt storage is retried. Git ref creation is not repeated in this loop.
    for _ in 0..32 {
        if let Ok(current) = store.snapshot() {
            if current
                .receipts
                .iter()
                .any(|receipt| receipt.request.action == action)
            {
                return Ok((
                    current,
                    format!("Review branch ready: {name} · git switch {name}"),
                ));
            }
            if let Some(receipt) = current
                .receipts
                .iter()
                .find(|receipt| receipt.request.correlation == correlation)
            {
                if receipt.request.action != action {
                    return Err("Branch exists, but receipt correlation conflicts".into());
                }
            }
            let request = Request {
                correlation: correlation.clone(),
                expected_revision: current.revision,
                action: action.clone(),
            };
            if let Ok((saved, _)) = store.transact(request) {
                return Ok((
                    saved,
                    format!("Review branch ready: {name} · git switch {name}"),
                ));
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(format!(
        "Branch {name} exists; receipt not confirmed. Repeat /branch {task} to reconcile."
    ))
}
