//! Persistable operation identity. Intent is not approval or proof of execution.
use crate::{tasks, understanding};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Intent {
    Selection {
        request: crate::selection_command::Request,
    },
    SelectionArrival {
        request: crate::selection_command::Request,
    },
    Wayfinder {
        request: understanding::Request,
        user_message: usize,
    },
    ArchitectDraft {
        request: crate::planner_command::ArchitectRequest,
    },
    DispatchRun {
        request: crate::dispatch::RunRequest,
    },
    Control {
        request: crate::control_command::Request,
    },
    Planner {
        request: crate::planner_command::Request,
    },
    Task {
        request: tasks::Request,
    },
    Scope {
        request: understanding::Request,
    },
    Run {
        correlation: String,
        expected_revision: u64,
        task: u64,
    },
    Branch {
        correlation: String,
        expected_revision: u64,
        task: u64,
        run: String,
    },
    Recover {
        correlation: String,
        task: u64,
        run: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Acknowledgment {
    Task {
        revision: u64,
        task: u64,
        correlation: String,
    },
    Scope {
        revision: u64,
        correlation: String,
    },
}
impl Intent {
    pub fn selection_request(&self) -> Option<&crate::selection_command::Request> {
        match self {
            Self::Selection { request } | Self::SelectionArrival { request } => Some(request),
            _ => None,
        }
    }
    pub fn scope_request(&self) -> Option<&understanding::Request> {
        match self {
            Self::Scope { request } | Self::Wayfinder { request, .. } => Some(request),
            _ => None,
        }
    }
    pub fn planner_request(&self) -> Option<&crate::planner_command::Request> {
        match self {
            Self::Planner { request } => Some(request),
            Self::ArchitectDraft { request } => Some(&request.request),
            _ => None,
        }
    }
    fn dispatch_run_binding(
        request: &crate::dispatch::RunRequest,
        snapshot: Option<&tasks::Snapshot>,
    ) -> Option<Self> {
        request.validate().ok()?;
        let snapshot = snapshot?;
        let approval = snapshot.receipts.iter().take(usize::try_from(request.expected_revision).ok()?).rev().find(|receipt| matches!(receipt.request.action, tasks::Action::Approve { task } if task == request.task))?;
        if approval.revision != request.approval_revision {
            return None;
        }
        Some(Self::Run {
            correlation: request.correlation.clone(),
            expected_revision: request.expected_revision,
            task: request.task,
        })
    }
    pub fn correlation(&self) -> &str {
        match self {
            Self::Selection { request } | Self::SelectionArrival { request } => {
                &request.correlation
            }
            Self::Task { request } => &request.correlation,
            Self::Planner { request } => &request.correlation,
            Self::ArchitectDraft { request } => &request.request.correlation,
            Self::Control { request } => &request.correlation,
            Self::DispatchRun { request } => &request.correlation,
            Self::Scope { request } | Self::Wayfinder { request, .. } => &request.correlation,
            Self::Run { correlation, .. }
            | Self::Branch { correlation, .. }
            | Self::Recover { correlation, .. } => correlation,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let correlation = self.correlation();
        if correlation.trim().is_empty()
            || correlation.len() > 160
            || correlation.chars().any(char::is_control)
            || serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 160 * 1024
        {
            return Err("Command intent identity or size is invalid".into());
        }
        match self {
            Self::Selection { request } => {
                request.validate()?;
                if !matches!(
                    request.origin,
                    crate::selection_command::Origin::Conversation { .. }
                ) {
                    return Err("Selection source command requires its conversation origin".into());
                }
            }
            Self::SelectionArrival { request } => request.validate()?,
            Self::Wayfinder {
                request,
                user_message,
            } => {
                understanding::validate_request(request)?;
                if *user_message >= crate::model::MAX_MESSAGES || !user_message.is_multiple_of(2) {
                    return Err("Wayfinder intent requires a bounded original user turn".into());
                }
            }
            Self::Planner { request } => request.validate()?,
            Self::ArchitectDraft { request } => request.validate()?,
            Self::Control { request } => request.validate()?,
            Self::DispatchRun { request } => request.validate()?,
            Self::Task { request } => {
                if request.expected_revision > 4096
                    || matches!(
                        request.action,
                        tasks::Action::Start { .. }
                            | tasks::Action::Finish { .. }
                            | tasks::Action::Branch { .. }
                    )
                {
                    return Err("Task intent needs a supported exact user request".into());
                }
                match &request.action {
                    tasks::Action::Plan { plan } => plan.validate()?,
                    tasks::Action::Permit { policy, .. } => policy.validate()?,
                    tasks::Action::Decide { decision, .. }
                    | tasks::Action::ReviewAndRepair { decision, .. }
                    | tasks::Action::ReviewArchitecture { decision, .. } => decision.validate()?,
                    tasks::Action::Assess { assessment, .. } => assessment.validate()?,
                    _ => {}
                }
            }
            Self::Scope { request } if request.expected_revision > 256 => {
                return Err("Scope intent revision exceeds bounds".into())
            }
            Self::Run {
                expected_revision,
                task,
                ..
            }
            | Self::Branch {
                expected_revision,
                task,
                ..
            } => {
                if *expected_revision > 4096 || !(1..=256).contains(task) {
                    return Err("Command task or revision exceeds bounds".into());
                }
            }
            Self::Recover { task, .. } if !(1..=256).contains(task) => {
                return Err("Recovery task exceeds bounds".into())
            }
            _ => {}
        }
        if let Self::Branch { run, .. } | Self::Recover { run, .. } = self {
            if run.is_empty()
                || run.len() > 100
                || !run.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err("Command run binding is invalid".into());
            }
        }
        Ok(())
    }
    pub fn reconcile(
        &self,
        snapshot: Option<&tasks::Snapshot>,
        scope: Option<&understanding::Snapshot>,
    ) -> Option<Acknowledgment> {
        if let Self::DispatchRun { request } = self {
            return Self::dispatch_run_binding(request, snapshot)?.reconcile(snapshot, scope);
        }
        // Rendering resolves against an already validated canonical snapshot.
        // Do not serialize a potentially 128-KiB Plan on every redraw: exact
        // receipt equality proves its payload, while persistence validates intent.
        let correlation = self.correlation();
        if correlation.trim().is_empty()
            || correlation.len() > 160
            || correlation.chars().any(char::is_control)
            || matches!(self, Self::Task { request } if matches!(request.action, tasks::Action::Start { .. } | tasks::Action::Finish { .. } | tasks::Action::Branch { .. }))
        {
            return None;
        }
        if let Some(request) = self.scope_request() {
            return scope?
                .receipts
                .get(usize::try_from(request.expected_revision).ok()?)
                .filter(|r| r.request == *request)
                .map(|r| Acknowledgment::Scope {
                    revision: r.request.expected_revision + 1,
                    correlation: r.request.correlation.clone(),
                });
        }
        let snapshot = snapshot?;
        let matches = |receipt: &&tasks::Receipt| match self {
            Self::Task { request } => receipt.request == *request,
            Self::Run {
                correlation,
                expected_revision,
                task,
            } => {
                receipt.request.correlation == *correlation
                    && receipt.request.expected_revision == *expected_revision
                    && matches!(receipt.request.action, tasks::Action::Start { task: id, .. } if id == *task)
            }
            Self::Branch {
                correlation,
                task,
                run,
                ..
            } => {
                receipt.request.correlation == *correlation
                    && snapshot
                        .tasks
                        .iter()
                        .any(|t| t.id == *task && t.run.as_ref().is_some_and(|r| r.id == *run))
                    && matches!(receipt.request.action, tasks::Action::Branch { task: id, .. } if id == *task)
            }
            Self::Recover { task, run, .. } => {
                receipt.request.correlation == format!("finish:{run}")
                    && matches!(&receipt.request.action, tasks::Action::Finish { task: id, run: finished, .. } if id == task && finished == run)
            }
            Self::Selection { .. }
            | Self::SelectionArrival { .. }
            | Self::Scope { .. }
            | Self::Wayfinder { .. }
            | Self::ArchitectDraft { .. }
            | Self::Planner { .. }
            | Self::Control { .. }
            | Self::DispatchRun { .. } => false,
        };
        let expected = match self {
            Self::Task { request } => Some(request.expected_revision),
            Self::Run {
                expected_revision, ..
            }
            | Self::Branch {
                expected_revision, ..
            } => Some(*expected_revision),
            _ => None,
        };
        let indexed = expected
            .and_then(|revision| usize::try_from(revision).ok())
            .and_then(|index| snapshot.receipts.get(index))
            .filter(matches);
        let receipt = indexed.or_else(|| {
            // Branch publication may retry only receipt storage at a newer revision;
            // recovery binds an existing run's deterministic Finish receipt.
            if matches!(self, Self::Branch { .. } | Self::Recover { .. }) {
                snapshot.receipts.iter().find(matches)
            } else {
                None
            }
        })?;
        Some(Acknowledgment::Task {
            revision: receipt.revision,
            task: receipt.task,
            correlation: receipt.request.correlation.clone(),
        })
    }
    /// Resolve the canonical task lifecycle anchored by this saved command.
    /// A worker claim acknowledges Start only; a result requires its own exact
    /// Finish receipt. Local worker ownership or task status cannot supply one.
    pub fn task_receipts(&self, snapshot: Option<&tasks::Snapshot>) -> Vec<Acknowledgment> {
        if let Self::DispatchRun { request } = self {
            return Self::dispatch_run_binding(request, snapshot)
                .map(|intent| intent.task_receipts(snapshot))
                .unwrap_or_default();
        }
        if let Self::Control { request } = self {
            return match &request.operation {
                crate::control_command::Operation::CancelWorker {
                    task,
                    start_correlation,
                    expected_start_revision,
                } => Self::Run {
                    correlation: start_correlation.clone(),
                    expected_revision: match expected_start_revision.checked_sub(1) {
                        Some(revision) => revision,
                        None => return vec![],
                    },
                    task: *task,
                }
                .task_receipts(snapshot),
                crate::control_command::Operation::Dispatch { .. } => vec![],
            };
        }
        let Some(primary @ Acknowledgment::Task { .. }) = self.reconcile(snapshot, None) else {
            return vec![];
        };
        let Self::Run {
            task,
            expected_revision,
            ..
        } = self
        else {
            return vec![primary];
        };
        let snapshot = snapshot.unwrap(); // The primary task receipt requires it.
        let start = &snapshot.receipts[*expected_revision as usize];
        let tasks::Action::Start {
            baseline, inputs, ..
        } = &start.request.action
        else {
            return vec![primary];
        };
        let run_id = format!("task-{task}-run-{}", start.revision);
        let Some(run) = snapshot
            .tasks
            .iter()
            .find(|item| item.id == *task)
            .and_then(|item| item.run.as_ref())
            .filter(|run| run.id == run_id && run.baseline == *baseline && run.inputs == *inputs)
        else {
            return vec![primary];
        };
        // A canonical unfinished run cannot have a Finish receipt. Avoid
        // rescanning the ledger on each redraw while its worker is active.
        if run.evidence_sha256.is_none() {
            return vec![primary];
        }
        let finish_correlation = format!("finish:{run_id}");
        let finish = snapshot.receipts.iter().find(|receipt| {
            receipt.revision > start.revision
                && receipt.task == *task
                && receipt.request.correlation == finish_correlation
                && matches!(&receipt.request.action,
                    tasks::Action::Finish { task: finished_task, run: finished_run, status, evidence_sha256, .. }
                    if finished_task == task && finished_run == &run_id
                        && matches!(status, tasks::TaskStatus::ReviewReady | tasks::TaskStatus::Failed | tasks::TaskStatus::Cancelled)
                        && evidence_sha256.len() == 64
                        && evidence_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                        && run.evidence_sha256.as_ref() == Some(evidence_sha256))
        });
        let mut receipts = vec![primary];
        if let Some(finish) = finish {
            receipts.push(Acknowledgment::Task {
                revision: finish.revision,
                task: finish.task,
                correlation: finish.request.correlation.clone(),
            });
        }
        receipts
    }

    /// Task receipts actually shown by this command. Cancellation uses Start as
    /// a result binding, but its own primary line acknowledges only the request.
    pub fn displayed_task_receipts(
        &self,
        snapshot: Option<&tasks::Snapshot>,
    ) -> Vec<Acknowledgment> {
        let receipts = self.task_receipts(snapshot);
        if matches!(self, Self::Control { request } if matches!(request.operation, crate::control_command::Operation::CancelWorker { .. }))
        {
            receipts.into_iter().skip(1).collect()
        } else {
            receipts
        }
    }
    pub fn acknowledgment(
        &self,
        snapshot: Option<&tasks::Snapshot>,
        scope: Option<&understanding::Snapshot>,
    ) -> Option<String> {
        self.reconcile(snapshot, scope).map(|ack| match ack {
            Acknowledgment::Task {
                revision,
                task,
                correlation,
            } => format!("Task receipt r{revision} · task #{task} · {correlation}"),
            Acknowledgment::Scope {
                revision,
                correlation,
            } => format!("Scope receipt r{revision} · {correlation}"),
        })
    }
}
