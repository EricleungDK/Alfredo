//! Process-local dispatch intent; approvals and run receipts remain durable authority.
use crate::tasks::{Action, Snapshot, Task, TaskStatus};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One proposed automatic launch, linked to its exact process-local enabling command.
/// This becomes dispatchable only after the console has durably saved the intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub correlation: String,
    pub expected_revision: u64,
    pub task: u64,
    pub approval_revision: u64,
    pub source: crate::control_command::Request,
}
impl RunRequest {
    pub fn validate(&self) -> Result<(), String> {
        self.source.validate()?;
        if self.correlation.trim().is_empty()
            || self.correlation.len() > 160
            || self.correlation.chars().any(char::is_control)
            || self.correlation == self.source.correlation
            || !(1..=256).contains(&self.task)
            || self.expected_revision >= 4096
            || self.approval_revision == 0
            || self.approval_revision > self.expected_revision
            || !matches!(
                self.source.operation,
                crate::control_command::Operation::Dispatch { enabled: true, .. }
            )
        {
            return Err("Invalid automatic launch identity, approval or source".into());
        }
        Ok(())
    }
}

/// Consecutive transient refusals (stale revision, busy store) tolerated for one
/// decision before the controller stops and asks for attention.
pub const TRANSIENT_LIMIT: u32 = 5;

#[derive(Default)]
pub struct Dispatch {
    pub enabled: bool,
    pub attempts: BTreeMap<u64, u64>,
    pub failures: BTreeMap<u64, String>,
    /// Consecutive launch refusals that wrote nothing, per task.
    pub transient: BTreeMap<u64, u32>,
    /// Set when a launch hit `TRANSIENT_LIMIT`; dispatch was turned off.
    pub contended: Option<String>,
}
pub fn approval(snapshot: &Snapshot, task: u64) -> Option<u64> {
    snapshot.receipts.iter().rev().find_map(|receipt| {
        matches!(receipt.request.action, Action::Approve {task:id} | Action::Requeue {task:id} if id == task)
            .then_some(receipt.revision)
    })
}
impl Dispatch {
    pub fn next(&self, snapshot: &Snapshot, active: &BTreeSet<u64>) -> Option<u64> {
        self.next_matching(snapshot, active, |_| true)
    }
    pub fn next_matching(
        &self,
        snapshot: &Snapshot,
        active: &BTreeSet<u64>,
        eligible: impl Fn(&Task) -> bool,
    ) -> Option<u64> {
        if !self.enabled {
            return None;
        }
        snapshot.tasks.iter().find_map(|task| {
            if task.status != TaskStatus::Approved
                || task.policy.is_none()
                || active.contains(&task.id)
                || snapshot.architecture_blocker(task.id).is_some()
                || snapshot.architecture_obsolete(task.id)
                || !eligible(task)
                || (task.repair_of.is_some() && snapshot.resolution_for_family(task.id).is_some())
                || task
                    .dependencies
                    .iter()
                    .any(|id| snapshot.dependency_source(*id).is_err())
            {
                return None;
            }
            let approval = approval(snapshot, task.id)?;
            (self.attempts.get(&task.id) != Some(&approval)).then_some(task.id)
        })
    }
}
