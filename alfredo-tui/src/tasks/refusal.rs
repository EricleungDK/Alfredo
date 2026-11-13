use super::{Snapshot, Task, TaskStatus, MAX_TASKS};

/// A failure after a boundary record was written leaves the outcome unknown.
pub(super) fn after_write(wrote: bool, reason: String) -> Refusal {
    if wrote {
        Refusal::Uncertain(reason)
    } else {
        Refusal::Denied(reason)
    }
}

/// The way out of a full task list: the limit is per mission, so a new mission starts empty.
pub(super) const NEW_MISSION: &str =
    "start a new mission (alfredo-tui --new-mission NAME, or Start New Mission in the mission picker)";

/// The same way out for `--headless`, which has no mission picker but takes `--new-mission`.
const HEADLESS_NEW_MISSION: &str =
    "start a new mission (add --new-mission NAME to this --headless command)";

/// A capacity refusal worded for `--headless`; any other text is returned unchanged.
pub fn headless_capacity_wording(text: &str) -> String {
    text.replace(NEW_MISSION, HEADLESS_NEW_MISSION)
}

/// The one refusal for a task list that cannot take `needed` more tasks (used for every
/// capacity check, before and after any planner request).
#[must_use]
pub fn capacity_refusal(used: usize, needed: usize) -> String {
    let free = MAX_TASKS.saturating_sub(used);
    if needed <= 1 {
        format!(
            "Task capacity reached: this mission has {used} of {MAX_TASKS} tasks and finished tasks are never reclaimed; {NEW_MISSION} to keep planning"
        )
    } else {
        format!(
            "Task capacity reached: this plan needs {needed} task slots but only {free} free ({used} of {MAX_TASKS} used); {NEW_MISSION} to keep planning"
        )
    }
}

/// Whether `text` is a task-limit refusal (a stop that retrying can never fix).
#[must_use]
pub fn is_capacity_refusal(text: &str) -> bool {
    text.contains("Task capacity reached: ")
}

pub fn unknown_task(task: u64) -> String {
    format!("Unknown task {task}; the task panel lists ids")
}

/// Lowercase status word shown in refusals.
pub fn status_label(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Proposed => "proposed",
        TaskStatus::Approved => "approved",
        TaskStatus::Cancelled => "cancelled",
        TaskStatus::Running => "running",
        TaskStatus::ReviewReady => "review-ready",
        TaskStatus::NeedsHumanReview => "needs-human-review",
        TaskStatus::Accepted => "accepted",
        TaskStatus::Failed => "failed",
        TaskStatus::Rejected => "rejected",
    }
}

/// A running task: wait, `/stop` a worker of this process, or record a dead worker with
/// `/recover`. Shown wherever the store holds Running but this process has no worker.
pub(super) fn running_hint(task: u64) -> String {
    format!(
        "Task #{task} is running; wait for it to finish, /stop it, or /recover {task} if its worker died"
    )
}

/// A task cancelled before any run has nothing to requeue, review or evidence; new
/// work is the way forward.
fn cancelled_before_run(task: u64) -> String {
    format!("Task #{task} was cancelled before it ran; /task describes new work to do instead")
}

/// Refusal for `/accept`/`/reject`/`/review` on a task that is not review-ready.
pub(super) fn review_refusal(task: u64, status: &TaskStatus) -> String {
    match status {
        TaskStatus::Accepted | TaskStatus::Rejected => {
            format!("Task #{task} is already {}", status_label(status))
        }
        TaskStatus::Failed => format!("Task #{task} failed; /repair {task} <hint>"),
        TaskStatus::Running => running_hint(task),
        TaskStatus::Proposed => {
            format!("Task #{task} is proposed; /approve {task}, then /run {task}")
        }
        TaskStatus::Approved => {
            format!("Task #{task} has not run; /run {task} first (it is approved)")
        }
        TaskStatus::Cancelled => {
            format!("Task #{task} is cancelled; /tasks shows what can still be reviewed")
        }
        TaskStatus::NeedsHumanReview => {
            format!("Task #{task} needs human review; /tasks shows its state")
        }
        TaskStatus::ReviewReady => format!("Task #{task} is review-ready"),
    }
}

/// Refusal for `/run` on a task that still needs approval.
pub fn proposed_run(task: u64) -> String {
    format!("Task #{task} is proposed; /approve {task} first")
}

/// Refusal for `/run` on a task that cannot start a worker, naming the next command
/// the store accepts. `None` only for approved. A running task is reported here only
/// when this process has no worker for it (the caller checks its own workers first),
/// so the way out is `/recover`, not the store's internal owner rule.
pub fn run_refusal(item: &Task) -> Option<String> {
    let task = item.id;
    let label = status_label(&item.status);
    let hint = match &item.status {
        TaskStatus::Proposed => return Some(proposed_run(task)),
        TaskStatus::Approved => return None,
        TaskStatus::Running => return Some(running_hint(task)),
        TaskStatus::Accepted => "nothing is left to run".to_owned(),
        TaskStatus::Failed | TaskStatus::Rejected => {
            format!("/repair {task} <hint> to try again")
        }
        TaskStatus::ReviewReady => format!("/accept {task} or /review {task} JSON"),
        TaskStatus::Cancelled if item.can_requeue() => {
            format!("/requeue {task} to approve it again")
        }
        TaskStatus::Cancelled if item.run.is_none() => {
            return Some(cancelled_before_run(task));
        }
        TaskStatus::Cancelled => "/tasks shows what can still be reviewed".to_owned(),
        TaskStatus::NeedsHumanReview => format!("/review {task} JSON"),
    };
    Some(format!("Task #{task} is {label}; {hint}"))
}

/// Why a task has no run to read, naming a next step the store accepts for it.
pub fn not_run(item: &Task) -> String {
    let task = item.id;
    match item.status {
        TaskStatus::Cancelled => cancelled_before_run(task),
        _ => format!("Task #{task} has not run; /run {task} first"),
    }
}

/// Why a transaction wrote nothing. Transient refusals leave no receipt, so the
/// caller may prepare the decision again on current state; others are final.
#[derive(Debug)]
pub enum Refusal {
    /// Another receipt landed after the request captured its revision. Carries
    /// the current snapshot when the refusing side read it.
    Stale(Option<Box<Snapshot>>),
    /// The store lock stayed contended past its bounded wait.
    Busy,
    /// Policy, scope, approval, evidence, capacity or any other refusal.
    Denied(String),
    /// A write or worker failed after the request may have taken effect, so the
    /// outcome is unknown and an exact retry is the honest next step.
    Uncertain(String),
}
impl Refusal {
    pub fn transient(&self) -> bool {
        matches!(self, Self::Stale(_) | Self::Busy)
    }
    /// Decided before any write: the same request can only be refused again.
    pub fn final_refusal(&self) -> bool {
        matches!(self, Self::Denied(_))
    }
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stale(_) => "Task state changed; refresh before submitting again",
            Self::Busy => "Task store is busy; refresh and retry",
            Self::Denied(reason) | Self::Uncertain(reason) => reason,
        })
    }
}
impl From<String> for Refusal {
    fn from(reason: String) -> Self {
        Self::Denied(reason)
    }
}
impl From<crate::understanding::LockError> for Refusal {
    fn from(error: crate::understanding::LockError) -> Self {
        match error {
            crate::understanding::LockError::Busy => Self::Busy,
            crate::understanding::LockError::Failed(reason) => Self::Denied(reason),
        }
    }
}
impl From<&str> for Refusal {
    fn from(reason: &str) -> Self {
        Self::Denied(reason.into())
    }
}
impl From<Refusal> for String {
    fn from(refusal: Refusal) -> Self {
        refusal.to_string()
    }
}
