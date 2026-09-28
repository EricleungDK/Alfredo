//! Assignment admission queries the current model catalog; receipts remain task authority.
use crate::{
    provider::Ollama,
    tasks::{Action, Request, Snapshot, TaskStore},
};

pub async fn assign(
    store: TaskStore,
    request: Request,
    provider: Ollama,
) -> Result<(Snapshot, String), String> {
    let Action::Assign { task, model } = &request.action else {
        return Err("Assignment request required".into());
    };
    let task = *task;
    let model = model.clone();
    let snapshot = store.snapshot()?;
    // Already acknowledged assignments replay even if the server is now unavailable.
    let replay = snapshot
        .receipts
        .iter()
        .any(|receipt| receipt.request.correlation == request.correlation);
    if !replay {
        if snapshot.revision != request.expected_revision {
            return Err("Task state changed; refresh before assigning".into());
        }
        snapshot
            .tasks
            .iter()
            .find(|item| item.id == task)
            .ok_or("Unknown task")?
            .validate_assignment(&model)?;
        let models = provider.models().await?;
        if !models.contains(&model) {
            return Err(format!(
                "Worker model {model} is not installed; /models lists available choices"
            ));
        }
    }
    // The catalog query holds no store lock. Recheck revision/state during publication.
    let (snapshot, receipt) = tokio::task::spawn_blocking(move || store.transact(request))
        .await
        .map_err(|_| {
            "Assignment storage stopped unexpectedly; retry the exact request".to_string()
        })??;
    let current = snapshot
        .tasks
        .iter()
        .find(|item| item.id == task)
        .ok_or("Missing assigned task")?;
    let notice = format!(
        "Assignment recorded: task #{task} → {model} · revision {} · current model {} / {:?}{}",
        receipt.revision,
        current.model,
        current.status,
        if current.status == crate::tasks::TaskStatus::Proposed {
            " · fresh /approve required"
        } else {
            ""
        }
    );
    Ok((snapshot, notice))
}
