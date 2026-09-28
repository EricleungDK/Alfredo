//! Read-only task search and explanations; these never authorize a run.
use crate::tasks::{Snapshot, Task, TaskStatus};
pub fn readiness(snapshot: &Snapshot, task: &Task) -> String {
    if snapshot.architecture_required(task.id) {
        return format!("Repeated architecture failures · Frontier Architect revision required · /architect-revise {}", task.id);
    }
    if let Some(source) = snapshot.resolved_by(task.id) {
        return format!("Resolved by accepted repair #{source} · original result {} · /tasks #{source} to inspect", snapshot.task_status_label(task));
    }
    if snapshot.architecture_obsolete(task.id) {
        return "Superseded by adopted Architect revision · inspect the revised task".into();
    }
    if task.repair_of.is_some() {
        if let Some(source) = snapshot.resolution_for_family(task.id) {
            if source != task.id {
                return format!("Repair family resolved by #{source} · no further repair work");
            }
        } else if task.status == TaskStatus::Accepted {
            return format!(
                "{} repair · /resolve-repair {} to use it for original dependencies",
                snapshot.task_status_label(task),
                task.id
            );
        }
    }
    if matches!(
        task.status,
        TaskStatus::Rejected | TaskStatus::Failed | TaskStatus::Cancelled
    ) {
        if let Some(child) = snapshot
            .tasks
            .iter()
            .rev()
            .find(|child| child.repair_of == Some(task.id))
        {
            return format!(
                "Repair #{}: {} · /tasks #{} to inspect",
                child.id,
                snapshot.task_status_label(child),
                child.id
            );
        }
    }
    if task.status == TaskStatus::NeedsHumanReview {
        if let Some(risk) = snapshot.decision_for_task(task.id).and_then(|d| d.risk) {
            return format!("{} risk escalated to human review; resolve with /review before repair or dependent work", risk.label());
        }
    }
    let gate = match task.status {
        TaskStatus::Proposed if task.policy.is_none() => {
            "Needs exact file/check policy, then approval"
        }
        TaskStatus::Proposed => "Needs approval",
        TaskStatus::Approved if task.policy.is_none() => {
            "Needs exact file/check policy and fresh approval"
        }
        TaskStatus::Approved => "Approved for run validation",
        TaskStatus::Running => "Run claimed; inspect progress or recovery evidence",
        TaskStatus::NeedsHumanReview => {
            "Held for human review; /review with an explicit outcome to resolve"
        }
        TaskStatus::ReviewReady => "Check passed; review evidence before accepting",
        TaskStatus::Accepted
            if snapshot.decision_for_task(task.id).is_some_and(|d| {
                d.outcome == crate::assessment::Outcome::ApprovedWithLimitations
            }) =>
        {
            "Accepted with limitations; inspect recorded review before using result"
        }
        TaskStatus::Rejected
            if snapshot
                .decision_for_task(task.id)
                .is_some_and(|d| d.outcome == crate::assessment::Outcome::NeedsRepair) =>
        {
            "Repair requested; /repair proposes a child requiring approval"
        }
        TaskStatus::Accepted => "Accepted result available to dependent tasks",
        TaskStatus::Failed => "Failed; inspect evidence before repair",
        TaskStatus::Rejected => "Rejected; propose a repair task",
        TaskStatus::Cancelled => "Cancelled; no execution will be dispatched",
    };
    if !matches!(task.status, TaskStatus::Proposed | TaskStatus::Approved) {
        return gate.into();
    }
    let mut blockers = Vec::new();
    let mut resolved = Vec::new();
    for id in &task.dependencies {
        match snapshot.dependency_source(*id) {
            Ok(source) if source.id != *id => {
                resolved.push(format!("#{id} via repair #{}", source.id))
            }
            Ok(_) => {}
            Err(_) => {
                let state = snapshot
                    .tasks
                    .iter()
                    .find(|parent| parent.id == *id)
                    .map(|parent| snapshot.task_status_label(parent))
                    .unwrap_or_else(|| "missing".into());
                blockers.push(format!("#{id} ({state})"));
            }
        }
    }
    let mut detail = gate.to_string();
    if !blockers.is_empty() {
        detail.push_str(&format!(" · blocked by {}", blockers.join(", ")));
    }
    if !resolved.is_empty() {
        detail.push_str(&format!(" · dependency inputs {}", resolved.join(", ")));
    }
    detail
}
pub fn matches(snapshot: &Snapshot, task: &Task, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    if let Some(id) = query.strip_prefix('#') {
        return id.parse::<u64>().ok() == Some(task.id);
    }
    let query = query.to_lowercase();
    format!("{} {} {:?}", task.title, task.model, task.status)
        .to_lowercase()
        .contains(&query)
        || readiness(snapshot, task).to_lowercase().contains(&query)
}

/// Observed project gate; task transactions still revalidate authoritative state.
#[derive(Clone, Debug)]
pub struct ScopeStatus {
    pub revision: Option<u64>,
    pub blocked: bool,
    pub label: String,
}
impl ScopeStatus {
    pub fn loading() -> Self {
        Self {
            revision: None,
            blocked: true,
            label: "Scope unobserved · /refresh before starting work".into(),
        }
    }
    pub fn observe(result: Result<crate::understanding::Snapshot, String>) -> Self {
        match result {
            Ok(state) => Self {
                revision: Some(state.revision),
                blocked: state.brief.is_some() && !state.confirmed,
                label: if let Some(flow) = &state.flow {
                    format!(
                        "Wayfinder / {} · Shared Understanding {} · revision {}",
                        flow.mode.label(),
                        if state.confirmed {
                            "confirmed"
                        } else {
                            "pending"
                        },
                        state.revision
                    )
                } else if state.brief.is_none() {
                    "Scope outside explicit flow · /scope to begin".into()
                } else if state.confirmed {
                    format!(
                        "Scope confirmed · revision {} · task approval still required",
                        state.revision
                    )
                } else {
                    format!(
                        "Scope pending · /scope then /scope-confirm {} · dispatch off",
                        state.draft_revision
                    )
                },
            },
            Err(_) => Self {
                revision: None,
                blocked: true,
                label: "Scope unavailable · /scope to inspect · dispatch off".into(),
            },
        }
    }
    pub fn run_blocker(&self, snapshot: &Snapshot, task: &Task) -> Option<String> {
        if !matches!(task.status, TaskStatus::Proposed | TaskStatus::Approved) {
            return None;
        }
        if self.blocked {
            return Some(self.label.clone());
        }
        let plan = snapshot.plan_for_task(task.id)?;
        let planned_revision = plan.scope.as_ref().map_or(0, |scope| scope.revision);
        (self.revision != Some(planned_revision))
            .then(|| "Plan scope changed · generate and review a fresh /plan".into())
    }
    pub fn readiness(&self, snapshot: &Snapshot, task: &Task) -> String {
        let detail = readiness(snapshot, task);
        match self.run_blocker(snapshot, task) {
            Some(blocker) => format!("{blocker} · {detail}"),
            None => detail,
        }
    }
}

/// Canonical work needing inspection, independent of the selected conversation/view.
/// Recorded runs without a local worker are not evidence of a live or dead process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkStatus {
    pub workers: usize,
    pub recorded: usize,
    pub review: usize,
    pub held: usize,
    pub repair: usize,
    pub architect: usize,
    pub resolve: usize,
    pub loaded: bool,
}
impl WorkStatus {
    pub fn attention(&self) -> usize {
        self.recorded + self.review + self.held + self.repair + self.architect + self.resolve
    }
    pub fn concise(&self, width: u16) -> String {
        if !self.loaded {
            return format!("Work {} local · state unavailable", self.workers);
        }
        if width < 40 {
            return format!("{} work · {} alerts", self.workers, self.attention());
        }
        let mut parts = vec![format!("Work {} local", self.workers)];
        // Nouns pluralize; state words (held, review, resolve) stay as labels.
        for (count, label, plural) in [
            (self.held, "held", false),
            (self.review, "review", false),
            (self.architect, "Architect", false),
            (self.repair, "repair", true),
            (self.resolve, "resolve", false),
            (self.recorded, "recorded run", true),
        ] {
            if count != 0 {
                let suffix = if plural && count != 1 { "s" } else { "" };
                parts.push(format!("{count} {label}{suffix}"));
            }
        }
        if self.attention() == 0 {
            parts.push("no pending review".into());
        }
        parts.join(" · ")
    }
}
