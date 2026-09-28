//! Read-only projection of acknowledged task receipts; no inferred actor or timestamp.
use crate::tasks::{Action, Snapshot};

#[derive(Debug)]
pub struct Entry {
    pub revision: u64,
    pub task: u64,
    pub correlation: String,
    pub summary: String,
    pub detail: String,
}

pub fn entries(snapshot: &Snapshot, query: &str) -> Vec<Entry> {
    iter_entries(snapshot, query).collect()
}

/// Projects newest matching receipts on demand. Exact task filters reject
/// unrelated receipts before formatting; text search still inspects their text.
pub fn iter_entries<'a>(snapshot: &'a Snapshot, query: &str) -> impl Iterator<Item = Entry> + 'a {
    let exact_task = query
        .strip_prefix('#')
        .and_then(|id| id.parse::<u64>().ok());
    let query = query.to_lowercase();
    snapshot
        .receipts
        .iter()
        .rev()
        .filter_map(move |receipt| {
            if exact_task.is_some_and(|id| id != receipt.task && !matches!(receipt.request.action, Action::ResolveRepair { task } if snapshot.resolved_by(id) == Some(task)) && !matches!(&receipt.request.action, Action::ReviewAndRepair { task, .. } | Action::ReviewArchitecture { task, .. } if *task == id) && !matches!(&receipt.request.action, Action::Plan { plan } if id >= receipt.task && id < receipt.task + plan.tasks.len() as u64)) {
                return None;
            }
            let (summary, detail) = match &receipt.request.action {
                Action::ReviewArchitecture { task, decision } => (if receipt.task == *task { format!("Architect revision required · #{task}") } else { format!("Architecture review · repair #{} proposed", receipt.task) }, decision.summary()),
                Action::ResolveRepair { task } => ("Repair resolution recorded".into(), format!("Accepted repair #{task} supplies its unsuccessful ancestors for future dependencies")),
                Action::Assign { model, .. } => ("Worker assigned".into(), format!("Local Agent model {model} · fresh approval required")),
                Action::Plan { plan } => (format!("Plan proposed · {} tasks{}", plan.tasks.len(), plan.scope.as_ref().map(|scope| format!(" · scope revision {}", scope.revision)).unwrap_or_default()), format!("Frontier Architect {} · {} · tasks require approval{}", plan.planner, plan.prompt, plan.context.as_ref().map(|context| format!(" · committed context {} · {} sources", context.baseline, context.sources.len())).unwrap_or_default())),
                Action::Branch { name, commit, .. } => (
                    "Review branch recorded".into(),
                    format!("{name} · commit {commit}"),
                ),
                Action::Propose {
                    title,
                    model,
                    dependencies,
                } => (
                    "Task proposed".into(),
                    format!("{title} · model {model} · depends on {dependencies:?}"),
                ),
                Action::ReviewAndRepair { task, decision } => (
                    format!("Review #{task}: {} · repair #{} proposed", decision.outcome.label(), receipt.task),
                    format!("{}\nRepair #{} inherits policy; fresh approval required", decision.summary(), receipt.task),
                ),
                Action::Repair { task, reason } => (
                    "Repair proposed · approval required".into(),
                    format!("Parent #{task} · {reason}"),
                ),
                Action::Permit { policy, .. } => (
                    "Policy set · approval required".into(),
                    format!("Files {:?} · check {:?}", policy.files, policy.check),
                ),
                Action::Approve { .. } => (
                    "Task approved".into(),
                    "Approval does not mean execution has started".into(),
                ),
                Action::Cancel { .. } => (
                    "Task cancelled".into(),
                    "Unstarted task cancellation acknowledged".into(),
                ),
                Action::Start {
                    baseline, inputs, ..
                } => (
                    "Worker run claimed".into(),
                    format!(
                        "Committed baseline {baseline} · {} accepted dependency inputs",
                        inputs.len()
                    ),
                ),
                Action::Finish {
                    run,
                    status,
                    evidence_sha256,
                    detail,
                    ..
                } => (
                    format!("Worker result: {status:?}"),
                    format!("{run} · {detail} · evidence SHA-256 {evidence_sha256}"),
                ),
                Action::Decide { decision, .. } => (
                    decision.risk.map_or_else(|| format!("Review: {}", decision.outcome.label()), |risk| format!("Human review required: {}", risk.label())),
                    decision.summary(),
                ),
                Action::Assess { assessment, .. } => (
                    if assessment.accept { "Review accepted with criterion assessment".into() } else { "Review rejected with criterion assessment".into() },
                    assessment.summary(),
                ),
                Action::Review { accept, .. } => (
                    if *accept {
                        "Review accepted".into()
                    } else {
                        "Review rejected".into()
                    },
                    "Decision recorded; changes remain in isolated worktree".into(),
                ),
            };
            let title = snapshot
                .tasks
                .iter()
                .find(|task| task.id == receipt.task)
                .map(|task| task.title.as_str())
                .unwrap_or("");
            if exact_task.is_none()
                && !format!(
                    "{} {} {} {} {}",
                    receipt.task, title, summary, detail, receipt.request.correlation
                )
                .to_lowercase()
                .contains(&query)
            {
                return None;
            }
            Some(Entry {
                revision: receipt.revision,
                task: receipt.task,
                correlation: receipt.request.correlation.clone(),
                summary,
                detail,
            })
        })
}
