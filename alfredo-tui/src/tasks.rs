//! Rust task authority. All callers use the same locked revision/receipt transaction.
pub const SCHEMA_VERSION: u32 = 16;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_STORE: usize = 4 * 1024 * 1024;
const MAX_TASKS: usize = 256;
const MAX_RECEIPTS: usize = 4096;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);
pub type Result<T> = std::result::Result<T, String>;

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
}
impl Refusal {
    pub fn transient(&self) -> bool {
        matches!(self, Self::Stale(_) | Self::Busy)
    }
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stale(_) => "Task state changed; refresh before submitting again",
            Self::Busy => "Task store is busy; refresh and retry",
            Self::Denied(reason) => reason,
        })
    }
}
impl From<String> for Refusal {
    fn from(reason: String) -> Self {
        Self::Denied(reason)
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Proposed,
    Approved,
    Cancelled,
    Running,
    ReviewReady,
    NeedsHumanReview,
    Accepted,
    Failed,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkPolicy {
    pub files: Vec<String>,
    pub check: Vec<String>,
}

impl WorkPolicy {
    pub fn validate(&self) -> Result<()> {
        use std::path::Component;
        if self.files.is_empty()
            || self.files.len() > 32
            || self.check.is_empty()
            || self.check.len() > 32
            || self.check.iter().any(|arg| !text_valid(arg, 2048))
        {
            return Err("Policy needs 1–32 exact files and a bounded check argv".into());
        }
        let mut unique = BTreeSet::new();
        for path in &self.files {
            if !text_valid(path, 512)
                || !unique.insert(path)
                || path.contains('\\')
                || path.contains(':')
                || path.starts_with('-')
                || path.split('/').any(|part| {
                    part.is_empty() || part.starts_with(".git") || part == "." || part == ".."
                })
                || Path::new(path)
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err("Policy files must be distinct repository-relative paths without Git metadata or traversal".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyInput {
    pub task: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_task: Option<u64>,
    pub run: String,
    pub evidence_sha256: String,
    pub candidate: String,
}

impl DependencyInput {
    pub fn source_id(&self) -> u64 {
        self.source_task.unwrap_or(self.task)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRun {
    pub id: String,
    pub baseline: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<DependencyInput>,
    pub evidence_sha256: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub model: String,
    pub dependencies: Vec<u64>,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<WorkPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<TaskRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_of: Option<u64>,
}

impl Task {
    pub fn validate_assignment(&self, model: &str) -> Result<()> {
        if !text_valid(model, 200) {
            return Err("Invalid worker model identity".into());
        }
        if self.run.is_some() || !matches!(self.status, TaskStatus::Proposed | TaskStatus::Approved)
        {
            return Err(
                "Assignment can only change before execution; propose a repair task".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    ReviewArchitecture {
        task: u64,
        decision: crate::assessment::Decision,
    },
    ResolveRepair {
        task: u64,
    },
    Assign {
        task: u64,
        model: String,
    },
    Plan {
        plan: crate::planner::Plan,
    },
    Branch {
        task: u64,
        name: String,
        commit: String,
    },
    Repair {
        task: u64,
        reason: String,
    },
    Propose {
        title: String,
        model: String,
        dependencies: Vec<u64>,
    },
    Approve {
        task: u64,
    },
    Cancel {
        task: u64,
    },
    Permit {
        task: u64,
        policy: WorkPolicy,
    },
    Start {
        task: u64,
        baseline: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inputs: Vec<DependencyInput>,
    },
    Finish {
        task: u64,
        run: String,
        status: TaskStatus,
        evidence_sha256: String,
        detail: String,
    },
    ReviewAndRepair {
        task: u64,
        decision: crate::assessment::Decision,
    },
    Decide {
        task: u64,
        decision: crate::assessment::Decision,
    },
    Assess {
        task: u64,
        assessment: crate::assessment::Assessment,
    },
    Review {
        task: u64,
        accept: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub correlation: String,
    pub expected_revision: u64,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub request: Request,
    pub revision: u64,
    pub task: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub workspace: PathBuf,
    pub mission: String,
    pub revision: u64,
    pub tasks: Vec<Task>,
    pub receipts: Vec<Receipt>,
}

impl Snapshot {
    pub fn architecture_failures(&self, mut id: u64) -> usize {
        let mut runs = BTreeSet::new();
        for _ in 0..self.tasks.len() {
            let Some(task) = self.tasks.iter().find(|t| t.id == id) else {
                break;
            };
            if self.decision_for_task(id).is_some_and(|d| {
                d.failure == Some(crate::assessment::FailureKind::Architecture)
                    && d.proposes_repair()
            }) {
                if let Some(run) = &task.run {
                    runs.insert(run.id.as_str());
                }
            }
            if self
                .plan_for_task(id)
                .is_some_and(|p| p.architecture.is_some())
            {
                break;
            }
            match task.repair_of {
                Some(parent) => id = parent,
                None => break,
            }
        }
        runs.len()
    }
    pub fn architecture_required(&self, task: u64) -> bool {
        // Ordinary rejected repair chains have no escalation receipt. Check the
        // receipt first instead of rescanning every ancestor on every UI redraw.
        if !self.tasks.iter().any(|t| t.id == task && t.status == TaskStatus::Rejected)
            || !self.receipts.iter().any(|r| matches!(r.request.action, Action::ReviewArchitecture { task: id, .. } if id == task && r.task == task))
        {
            return false;
        }
        let pending = match self.receipts.iter().rev().find(|r| matches!(&r.request.action, Action::Plan { plan } if plan.architecture.as_ref().is_some_and(|o| o.task == task))) {
            None => true,
            Some(adoption) => self.tasks.iter().any(|t| t.id == adoption.task && t.status == TaskStatus::Cancelled && t.run.is_none()),
        };
        pending && self.architecture_failures(task) >= 2
    }
    pub fn architecture_obsolete(&self, mut task: u64) -> bool {
        let Some(root) = self.repair_root(task) else {
            return false;
        };
        let Some(boundary) = self
            .receipts
            .iter()
            .rev()
            .find_map(|r| match &r.request.action {
                Action::Plan { plan }
                    if plan
                        .architecture
                        .as_ref()
                        .is_some_and(|o| self.repair_root(o.task) == Some(root)) =>
                {
                    Some(r.task)
                }
                _ => None,
            })
        else {
            return false;
        };
        for _ in 0..self.tasks.len() {
            if task == boundary {
                return false;
            }
            match self
                .tasks
                .iter()
                .find(|t| t.id == task)
                .and_then(|t| t.repair_of)
            {
                Some(parent) => task = parent,
                None => break,
            }
        }
        true
    }
    pub fn architecture_blocker(&self, task: u64) -> Option<u64> {
        let root = self.repair_root(task)?;
        self.tasks
            .iter()
            .find(|t| self.repair_root(t.id) == Some(root) && self.architecture_required(t.id))
            .map(|t| t.id)
    }
    pub fn architecture_origin(&self, task: u64) -> Result<crate::architecture::Origin> {
        if !self.architecture_required(task) || self.resolution_for_family(task).is_some() {
            return Err("Task has no pending Architect revision".into());
        }
        let root = self.repair_root(task).ok_or("Unknown repair root")?;
        if self.tasks.iter().any(|t| {
            self.repair_root(t.id) == Some(root)
                && matches!(
                    t.status,
                    TaskStatus::Proposed
                        | TaskStatus::Approved
                        | TaskStatus::Running
                        | TaskStatus::ReviewReady
                        | TaskStatus::NeedsHumanReview
                )
        }) {
            return Err(
                "Resolve outstanding repair work or human holds before Architect revision".into(),
            );
        }
        let item = self
            .tasks
            .iter()
            .find(|t| t.id == task)
            .ok_or("Unknown task")?;
        let run = item.run.as_ref().ok_or("Missing Architect source run")?;
        let review_revision = self.receipts.iter().rev().find(|r| matches!(r.request.action, Action::ReviewArchitecture { task: id, .. } if id == task && r.task == task)).ok_or("Missing Architect route receipt")?.revision;
        Ok(crate::architecture::Origin {
            task,
            review_revision,
            run: run.id.clone(),
            evidence_sha256: run
                .evidence_sha256
                .clone()
                .ok_or("Missing Architect source evidence")?,
        })
    }

    pub fn repair_root(&self, mut id: u64) -> Option<u64> {
        for _ in 0..=self.tasks.len() {
            let task = self.tasks.iter().find(|t| t.id == id)?;
            match task.repair_of {
                Some(parent) if parent < id => id = parent,
                None => return Some(id),
                _ => return None,
            }
        }
        None
    }
    pub fn resolved_by(&self, id: u64) -> Option<u64> {
        self.receipts.iter().find_map(|receipt| {
            let Action::ResolveRepair { task } = &receipt.request.action else {
                return None;
            };
            let mut current = *task;
            for _ in 0..self.tasks.len() {
                current = self.tasks.iter().find(|t| t.id == current)?.repair_of?;
                if current == id {
                    return Some(*task);
                }
            }
            None
        })
    }
    pub fn resolution_for_family(&self, id: u64) -> Option<u64> {
        let root = self.repair_root(id)?;
        self.receipts
            .iter()
            .find_map(|receipt| match receipt.request.action {
                Action::ResolveRepair { task } if self.repair_root(task) == Some(root) => {
                    Some(task)
                }
                _ => None,
            })
    }
    pub fn dependency_source(&self, id: u64) -> Result<&Task> {
        let declared = self
            .tasks
            .iter()
            .find(|t| t.id == id)
            .ok_or("Unknown dependency")?;
        let source = if declared.status == TaskStatus::Accepted {
            Some(declared)
        } else {
            self.resolved_by(id)
                .and_then(|source| self.tasks.iter().find(|t| t.id == source))
        };
        source
            .filter(|t| t.status == TaskStatus::Accepted)
            .ok_or_else(|| format!("Dependency #{id} is not accepted"))
    }

    pub fn decision_for_task(&self, task: u64) -> Option<&crate::assessment::Decision> {
        self.receipts
            .iter()
            .rev()
            .find_map(|receipt| match &receipt.request.action {
                Action::Decide { task: id, decision }
                | Action::ReviewAndRepair { task: id, decision }
                | Action::ReviewArchitecture { task: id, decision }
                    if *id == task =>
                {
                    Some(decision)
                }
                _ => None,
            })
    }
    pub fn review_summary_for_task(&self, task: u64) -> Option<String> {
        self.receipts
            .iter()
            .rev()
            .find_map(|receipt| match &receipt.request.action {
                Action::Decide { task: id, decision }
                | Action::ReviewAndRepair { task: id, decision }
                | Action::ReviewArchitecture { task: id, decision }
                    if *id == task =>
                {
                    Some(decision.summary())
                }
                Action::Assess {
                    task: id,
                    assessment,
                } if *id == task => Some(assessment.summary()),
                Action::Review { task: id, accept } if *id == task => Some(format!(
                    "Legacy reviewer {} · no criterion notes recorded",
                    if *accept { "accepted" } else { "rejected" }
                )),
                _ => None,
            })
    }
    pub fn task_status_label(&self, task: &Task) -> String {
        if task.status == TaskStatus::NeedsHumanReview {
            return "Needs human review".into();
        }
        self.decision_for_task(task.id)
            .map(|decision| decision.outcome.task_label().into())
            .unwrap_or_else(|| format!("{:?}", task.status))
    }

    pub fn assessment_for_task(&self, task: u64) -> Option<&crate::assessment::Assessment> {
        self.receipts
            .iter()
            .rev()
            .find_map(|receipt| match &receipt.request.action {
                Action::Assess {
                    task: id,
                    assessment,
                } if *id == task => Some(assessment),
                _ => None,
            })
    }

    /// Criteria are immutable proposal data; repairs inherit the original contract.
    /// Missing legacy criteria remain unrecorded, never inferred from a passed check.
    pub fn acceptance_for_task(&self, mut task: u64) -> &[String] {
        for _ in 0..=self.tasks.len() {
            for receipt in &self.receipts {
                if let Action::Plan { plan } = &receipt.request.action {
                    if let Some(index) = task
                        .checked_sub(receipt.task)
                        .and_then(|n| usize::try_from(n).ok())
                    {
                        if let Some(step) = plan.tasks.get(index) {
                            return &step.acceptance;
                        }
                    }
                }
            }
            match self
                .tasks
                .iter()
                .find(|item| item.id == task)
                .and_then(|item| item.repair_of)
            {
                Some(parent) if parent < task => task = parent,
                _ => break,
            }
        }
        &[]
    }

    pub fn plan_for_task(&self, task: u64) -> Option<&crate::planner::Plan> {
        self.receipts
            .iter()
            .find_map(|receipt| match &receipt.request.action {
                Action::Plan { plan }
                    if task >= receipt.task && task < receipt.task + plan.tasks.len() as u64 =>
                {
                    Some(plan)
                }
                _ => None,
            })
    }
}

#[derive(Clone)]
pub struct TaskStore {
    root: PathBuf,
    workspace: PathBuf,
    mission: String,
}

fn text_valid(text: &str, limit: usize) -> bool {
    !text.trim().is_empty() && text.len() <= limit && !text.chars().any(|c| c.is_control())
}

pub(crate) fn regular_file(path: &Path, create: bool, write: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(write).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Task store entry must be a regular non-symlink file".into());
        }
    }
    let file = options
        .open(path)
        .map_err(|e| format!("Cannot open task store: {e}"))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Task store entry is not a regular file".into());
    }
    Ok(file)
}

// Unlock explicitly before close: a concurrently forked child can briefly inherit
// the open file description before exec closes it. Closing only the parent's fd
// would otherwise leave the transaction lock held by an unrelated child.
pub(crate) struct StoreLock(pub(crate) File);
impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// A scoped worker claim. Inherited descriptors cannot prolong an orderly release.
/// Dropping this guard grants no recovery authority; saved evidence is still required.
pub struct WorkerOwner {
    _lock: StoreLock,
}

impl TaskStore {
    pub fn new(root: &Path, workspace: &Path, mission: &str) -> Result<Self> {
        let workspace = workspace
            .canonicalize()
            .map_err(|e| format!("Workspace unavailable: {e}"))?;
        if !workspace.is_dir() || workspace.to_str().is_none() {
            return Err("Workspace must be a UTF-8 directory".into());
        }
        if !text_valid(mission, 120) {
            return Err("Mission name must contain 1–120 bytes without controls".into());
        }
        let root = if root.is_absolute() {
            root.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(root)
        };
        // State is separate from the coding workspace and all legacy mission files.
        for ancestor in root.ancestors() {
            if ancestor.exists()
                && ancestor
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(&workspace)
            {
                return Err("Task state must live outside the coding workspace".into());
            }
        }
        let identity = serde_json::to_vec(&(&workspace, mission)).map_err(|e| e.to_string())?;
        let namespace = format!("{:x}", Sha256::digest(identity));
        Ok(Self {
            root: root.join("rust-tasks-v1").join(namespace),
            workspace,
            mission: mission.into(),
        })
    }

    pub(crate) fn identity(&self) -> (&Path, &str) {
        (&self.workspace, &self.mission)
    }

    fn lock(&self) -> Result<StoreLock> {
        self.lock_checked().map_err(String::from)
    }

    fn lock_checked(&self) -> std::result::Result<StoreLock, Refusal> {
        for ancestor in self.root.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err("Task state ancestors must not be symlinks".into())
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string().into()),
            }
        }
        fs::create_dir_all(&self.root).map_err(|e| format!("Cannot create task state: {e}"))?;
        for ancestor in self.root.ancestors() {
            if fs::symlink_metadata(ancestor)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err("Task state ancestors must not be symlinks".into());
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let lock = regular_file(&self.root.join("tasks.lock"), true, true)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
        loop {
            match lock.try_lock() {
                Ok(()) => return Ok(StoreLock(lock)),
                Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(std::fs::TryLockError::WouldBlock) => return Err(Refusal::Busy),
                Err(error) => return Err(format!("Task store lock unavailable: {error}").into()),
            }
        }
    }

    fn empty(&self) -> Snapshot {
        Snapshot {
            schema_version: SCHEMA_VERSION,
            workspace: self.workspace.clone(),
            mission: self.mission.clone(),
            revision: 0,
            tasks: vec![],
            receipts: vec![],
        }
    }

    fn read_locked(&self) -> Result<Snapshot> {
        let path = self.root.join("tasks.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(self.empty()),
            Err(e) => return Err(e.to_string()),
            Ok(_) => {}
        }
        let file = regular_file(&path, false, false)?;
        let mut bytes = Vec::new();
        file.take((MAX_STORE + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_STORE {
            return Err("Task state exceeds 4 MiB".into());
        }
        let snapshot: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|_| "Malformed task state; original bytes preserved".to_string())?;
        self.validate(&snapshot)?;
        Ok(snapshot)
    }

    fn validate(&self, snapshot: &Snapshot) -> Result<()> {
        if !matches!(snapshot.schema_version, 1..=SCHEMA_VERSION)
            || snapshot.workspace != self.workspace
            || snapshot.mission != self.mission
            || snapshot.tasks.len() > MAX_TASKS
            || snapshot.receipts.len() > MAX_RECEIPTS
            || snapshot.revision != snapshot.receipts.len() as u64
        {
            return Err("Task state identity, version or bounds are invalid".into());
        }
        // Replaying every bounded receipt proves the snapshot's task states and
        // dependency graph; inconsistent snapshots cannot fabricate an approval.
        let mut proof = self.empty();
        let mut correlations = BTreeSet::new();
        for receipt in &snapshot.receipts {
            if snapshot.schema_version < 9
                && matches!(&receipt.request.action, Action::Plan { plan } if plan.scope.is_some())
            {
                return Err("Plan scope binding requires schema v9".into());
            }
            if snapshot.schema_version < 8
                && matches!(&receipt.request.action, Action::Plan { plan } if plan.context.is_some())
            {
                return Err("Repository plan context requires schema v8".into());
            }
            if snapshot.schema_version < 7
                && matches!(receipt.request.action, Action::Assign { .. })
            {
                return Err("Assignment receipts require schema v7".into());
            }
            if snapshot.schema_version < 6 && matches!(receipt.request.action, Action::Plan { .. })
            {
                return Err("Plan receipts require schema v6".into());
            }
            if snapshot.schema_version < 5
                && matches!(receipt.request.action, Action::Branch { .. })
            {
                return Err("Branch receipts require schema v5".into());
            }
            if snapshot.schema_version < 4
                && matches!(&receipt.request.action, Action::Start { inputs, .. } if !inputs.is_empty())
            {
                return Err("Dependency inputs require task schema v4".into());
            }
            if snapshot.schema_version < 3
                && matches!(receipt.request.action, Action::Repair { .. })
            {
                return Err("Repair receipts require task schema v3".into());
            }
            if snapshot.schema_version == 1
                && !matches!(
                    receipt.request.action,
                    Action::Propose { .. } | Action::Approve { .. } | Action::Cancel { .. }
                )
            {
                return Err("Legacy task state cannot grant execution permissions".into());
            }
            if matches!(&receipt.request.action, Action::Decide { decision, .. } | Action::ReviewAndRepair { decision, .. } if decision.failure.is_some() && decision.proposes_repair())
            {
                return Err("Architecture failure requires its governed review route".into());
            }
            if snapshot.schema_version < 16
                && (matches!(receipt.request.action, Action::ReviewArchitecture { .. })
                    || matches!(&receipt.request.action, Action::Decide { decision, .. } | Action::ReviewAndRepair { decision, .. } if decision.failure.is_some())
                    || matches!(&receipt.request.action, Action::Plan { plan } if plan.architecture.is_some()))
            {
                return Err("Architect routing requires task schema v16".into());
            }
            if snapshot.schema_version < 15
                && (matches!(receipt.request.action, Action::ResolveRepair { .. })
                    || matches!(&receipt.request.action, Action::Start { inputs, .. } if inputs.iter().any(|i| i.source_task.is_some())))
            {
                return Err("Repair resolution requires task schema v15".into());
            }
            if snapshot.schema_version < 14
                && matches!(receipt.request.action, Action::ReviewAndRepair { .. })
            {
                return Err("Atomic review and repair requires task schema v14".into());
            }
            if snapshot.schema_version < 13
                && matches!(&receipt.request.action, Action::Decide { decision, .. } if decision.risk.is_some())
            {
                return Err("Review risk classification requires task schema v13".into());
            }
            if snapshot.schema_version < 12
                && matches!(receipt.request.action, Action::Decide { .. })
            {
                return Err("Review outcomes require task schema v12".into());
            }
            if snapshot.schema_version < 11
                && matches!(receipt.request.action, Action::Assess { .. })
            {
                return Err("Criterion review requires task schema v11".into());
            }
            if snapshot.schema_version < 10
                && matches!(&receipt.request.action, Action::Plan { plan } if plan.tasks.iter().any(|step| !step.acceptance.is_empty()))
            {
                return Err("Plan acceptance criteria require schema v10".into());
            }
            if !text_valid(&receipt.request.correlation, 160)
                || !correlations.insert(&receipt.request.correlation)
                || receipt.request.expected_revision != proof.revision
                || receipt.revision != proof.revision + 1
            {
                return Err("Task receipt sequence is invalid".into());
            }
            let id = Self::apply(&mut proof, &receipt.request.action)?;
            if id != receipt.task {
                return Err("Task receipt target is invalid".into());
            }
            proof.revision += 1;
            // Later assessments may reference only this already validated prefix.
            proof.receipts.push(receipt.clone());
        }
        if proof.tasks != snapshot.tasks {
            return Err("Task state does not match its receipts".into());
        }
        Ok(())
    }

    /// Read-only picker preflight. The admitted selection repeats full validation
    /// under the namespace lock before any mission creation.
    pub fn check_mission_selection(&self, start_new: bool) -> Result<()> {
        for path in self.root.ancestors() {
            match fs::symlink_metadata(path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err("Mission state ancestors must not be symlinks".into())
                }
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.to_string())
                }
                _ => {}
            }
        }
        let manifest = self.root.join("mission.json");
        let mut exists = fs::symlink_metadata(&manifest).is_ok()
            || fs::symlink_metadata(self.root.join("tasks.json")).is_ok();
        if !exists && self.root.try_exists().map_err(|error| error.to_string())? {
            for (index, entry) in fs::read_dir(&self.root)
                .map_err(|error| error.to_string())?
                .enumerate()
            {
                if index >= 1024 {
                    return Err("Mission namespace exceeds selection scan limit".into());
                }
                let entry = entry.map_err(|error| error.to_string())?;
                if entry.file_name().to_str().is_some_and(|name| {
                    name.starts_with("conversations-") && name.ends_with(".json")
                }) {
                    exists = true;
                    break;
                }
            }
        }
        match (start_new, exists) {
            (true, true) => Err("Mission already exists; choose Resume or another name".into()),
            (false, false) => Err("Mission does not exist; choose Start New Mission".into()),
            (false, true) if fs::symlink_metadata(&manifest).is_ok() => {
                crate::missions::validate_manifest(&manifest, &self.workspace, &self.mission)
            }
            _ => Ok(()),
        }
    }

    /// Select identity under the namespace lock; this grants no task approval.
    pub fn select_mission(&self, start_new: bool) -> Result<()> {
        if !start_new && !self.root.exists() {
            return Err("Mission does not exist; choose Start New Mission".into());
        }
        let _lock = self.lock()?;
        let manifest = self.root.join("mission.json");
        if std::fs::symlink_metadata(&manifest).is_ok() {
            if start_new {
                return Err("Mission already exists; choose Resume or another name".into());
            }
            return crate::missions::validate_manifest(&manifest, &self.workspace, &self.mission);
        }
        let has_tasks = std::fs::symlink_metadata(self.root.join("tasks.json")).is_ok();
        let mut conversations = Vec::new();
        for (index, entry) in fs::read_dir(&self.root)
            .map_err(|e| e.to_string())?
            .enumerate()
        {
            if index >= 1024 {
                return Err("Mission namespace exceeds selection scan limit".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|s| s.starts_with("conversations-") && s.ends_with(".json"))
            {
                conversations.push(entry.path());
            }
        }
        if has_tasks || !conversations.is_empty() {
            if start_new {
                return Err("Mission already exists; choose Resume or another name".into());
            }
            if has_tasks {
                self.read_locked()?;
                return Ok(());
            }
            // Legacy conversation-only mission: validate a saved namespace without rewriting it.
            conversations.sort();
            let path = &conversations[0];
            let mut bytes = Vec::new();
            regular_file(path, false, false)?
                .take(12 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 12 * 1024 * 1024 {
                return Err("Legacy conversation exceeds bounds".into());
            }
            let saved: crate::conversations::Snapshot =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid legacy conversation state")?;
            if !text_valid(&saved.namespace, 120) {
                return Err("Invalid legacy conversation name".into());
            }
            saved.validate(&saved.namespace)?;
            let expected = format!(
                "conversations-{:x}.json",
                Sha256::digest(saved.namespace.as_bytes())
            );
            if path.file_name().and_then(|s| s.to_str()) != Some(&expected) {
                return Err("Legacy conversation identity mismatch".into());
            }
            return Ok(());
        }
        if !start_new {
            return Err("Mission does not exist; choose Start New Mission".into());
        }
        crate::missions::create_manifest(&manifest, &self.workspace, &self.mission)
    }

    pub fn understanding(&self) -> crate::understanding::Store {
        crate::understanding::Store::new(
            self.root.parent().unwrap().parent().unwrap(),
            &self.workspace,
        )
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let _lock = self.lock()?;
        self.read_locked()
    }

    /// Read-only storage admission before a branch effect. This neither reserves
    /// capacity nor proves Git state; the final transaction must still revalidate.
    pub(crate) fn preflight_branch(&self, request: Request) -> Result<()> {
        if !matches!(request.action, Action::Branch { .. }) {
            return Err("Branch preflight requires a branch request".into());
        }
        if !text_valid(&request.correlation, 160) {
            return Err("Invalid task correlation identity".into());
        }
        let _lock = self.lock()?;
        let mut snapshot = self.read_locked()?;
        if let Some(receipt) = snapshot
            .receipts
            .iter()
            .find(|receipt| receipt.request.correlation == request.correlation)
        {
            if receipt.request.action != request.action {
                return Err("Branch correlation belongs to another request".into());
            }
        }
        // Exact handoff reconciliation needs no additional receipt, even at capacity.
        if snapshot
            .receipts
            .iter()
            .any(|receipt| receipt.request.action == request.action)
        {
            return Ok(());
        }
        if snapshot.revision != request.expected_revision {
            return Err("Task state changed; refresh before creating a review branch".into());
        }
        if snapshot.receipts.len() == MAX_RECEIPTS {
            return Err("Task receipt capacity reached".into());
        }
        snapshot.schema_version = SCHEMA_VERSION;
        let task = Self::apply(&mut snapshot, &request.action)?;
        snapshot.revision += 1;
        snapshot.receipts.push(Receipt {
            request,
            revision: snapshot.revision,
            task,
        });
        Self::require_completion_capacity(&snapshot)?;
        if serde_json::to_vec(&snapshot)
            .map_err(|e| e.to_string())?
            .len()
            > MAX_STORE
        {
            return Err("Task storage capacity reached (4 MiB)".into());
        }
        Ok(())
    }

    pub fn transact(&self, request: Request) -> Result<(Snapshot, Receipt)> {
        self.transact_checked(request).map_err(String::from)
    }

    /// `transact` with a typed refusal, so callers can tell a stale revision or a
    /// busy lock (nothing written; re-prepare on current state) from a final one.
    pub fn transact_checked(
        &self,
        request: Request,
    ) -> std::result::Result<(Snapshot, Receipt), Refusal> {
        if !text_valid(&request.correlation, 160) {
            return Err("Invalid task correlation identity".into());
        }
        let scope = self.understanding();
        let scope_guard = scope.lock()?;
        let _lock = self.lock_checked()?;
        let mut snapshot = self.read_locked()?;
        if let Some(receipt) = snapshot
            .receipts
            .iter()
            .find(|r| r.request.correlation == request.correlation)
        {
            if receipt.request != request {
                return Err("Correlation already belongs to a different task request".into());
            }
            return Ok((snapshot.clone(), receipt.clone()));
        }
        if !matches!(
            request.action,
            Action::Finish { .. }
                | Action::Cancel { .. }
                | Action::Review { .. }
                | Action::Assess { .. }
                | Action::Decide { .. }
                | Action::Branch { .. }
        ) {
            scope_guard.ensure_open()?;
        }
        match &request.action {
            Action::Plan { plan } => scope_guard.ensure_binding(plan.scope.as_deref())?,
            Action::Start { task, .. } => {
                if let Some(plan) = snapshot.plan_for_task(*task) {
                    scope_guard.ensure_binding(plan.scope.as_deref())?;
                }
            }
            _ => {}
        }
        if snapshot.revision != request.expected_revision {
            return Err(Refusal::Stale(Some(Box::new(snapshot))));
        }
        if let Action::Repair { task, .. } | Action::ReviewAndRepair { task, .. } = &request.action
        {
            if snapshot.resolution_for_family(*task).is_some() {
                return Err("Repair family is already resolved".into());
            }
        }
        if let Action::Decide { decision, .. } | Action::ReviewAndRepair { decision, .. } =
            &request.action
        {
            if decision.failure.is_some() && decision.proposes_repair() {
                return Err("Architecture failure requires its governed review route".into());
            }
        }
        // Earlier v12 builds allowed a sibling while a repair was held. Preserve
        // those receipts during replay, but never authorize another such proposal.
        if let Action::Repair { task, .. } = &request.action {
            if snapshot.tasks.iter().any(|child| {
                child.repair_of == Some(*task) && child.status == TaskStatus::NeedsHumanReview
            }) {
                return Err(
                    "An unresolved repair is held for human review; resolve its review first"
                        .into(),
                );
            }
        }
        if let Action::Review { task, accept: true } = &request.action {
            if !snapshot.acceptance_for_task(*task).is_empty() {
                return Err("This task has acceptance criteria; use /review ID JSON to record a reason and evidence notes for every criterion".into());
            }
        }
        if snapshot.receipts.len() == MAX_RECEIPTS {
            return Err("Task receipt capacity reached".into());
        }
        if snapshot.schema_version < SCHEMA_VERSION {
            // Migration never supplies permissions absent from the old store.
            let backup = self
                .root
                .join(format!("tasks-v{}-backup.json", snapshot.schema_version));
            let original = fs::read(self.root.join("tasks.json")).map_err(|e| e.to_string())?;
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
            {
                Ok(mut file) => {
                    file.write_all(&original)
                        .and_then(|_| file.sync_all())
                        .map_err(|e| e.to_string())?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let mut bytes = Vec::new();
                    regular_file(&backup, false, false)?
                        .take((MAX_STORE + 1) as u64)
                        .read_to_end(&mut bytes)
                        .map_err(|e| e.to_string())?;
                    if bytes != original {
                        return Err("Legacy task backup differs; original state preserved".into());
                    }
                }
                Err(error) => return Err(error.to_string().into()),
            }
            #[cfg(unix)]
            File::open(&self.root)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| e.to_string())?;
            snapshot.schema_version = SCHEMA_VERSION;
        }
        match &request.action {
            Action::Plan { plan } if plan.architecture.is_some() => {
                let origin = plan.architecture.as_ref().unwrap();
                if snapshot.architecture_origin(origin.task)? != *origin {
                    return Err("Stale Architect origin".into());
                }
                self.architecture_context_locked(&snapshot, origin.task)?;
            }
            Action::ResolveRepair { task } => {
                let mut id = Some(*task);
                while let Some(current) = id {
                    let item = snapshot
                        .tasks
                        .iter()
                        .find(|t| t.id == current)
                        .ok_or("Unknown repair")?;
                    let run = item
                        .run
                        .as_ref()
                        .ok_or("Repair lineage lacks run evidence")?;
                    let raw = self.read_evidence(
                        &run.id,
                        run.evidence_sha256
                            .as_deref()
                            .ok_or("Missing evidence digest")?,
                        current,
                        &snapshot,
                    )?;
                    if current == *task {
                        let evidence: crate::worker::Evidence =
                            serde_json::from_str(&raw).map_err(|_| "Malformed repair evidence")?;
                        if evidence.status != TaskStatus::ReviewReady
                            || evidence.candidate_commit.is_none()
                        {
                            return Err("Resolution requires successful candidate evidence".into());
                        }
                    }
                    id = item.repair_of;
                }
            }
            Action::Branch { task, commit, .. } => {
                let item = snapshot
                    .tasks
                    .iter()
                    .find(|item| item.id == *task)
                    .ok_or("Unknown task")?;
                let run = item.run.as_ref().ok_or("Task has no run")?;
                let raw = self.read_evidence(
                    &run.id,
                    run.evidence_sha256
                        .as_deref()
                        .ok_or("Missing evidence digest")?,
                    *task,
                    &snapshot,
                )?;
                let evidence: crate::worker::Evidence =
                    serde_json::from_str(&raw).map_err(|_| "Malformed branch evidence")?;
                if evidence.candidate_commit.as_ref() != Some(commit) {
                    return Err("Branch differs from the accepted candidate".into());
                }
            }
            Action::Start { inputs, .. } => {
                for input in inputs {
                    let raw = self.read_evidence(
                        &input.run,
                        &input.evidence_sha256,
                        input.source_id(),
                        &snapshot,
                    )?;
                    let evidence: crate::worker::Evidence =
                        serde_json::from_str(&raw).map_err(|_| "Malformed dependency evidence")?;
                    if evidence.candidate_commit.as_deref() != Some(input.candidate.as_str()) {
                        return Err("Dependency candidate differs from saved evidence".into());
                    }
                }
            }
            Action::Finish {
                task,
                run,
                evidence_sha256,
                status,
                detail,
            } => {
                let raw = self.read_evidence(run, evidence_sha256, *task, &snapshot)?;
                let evidence: crate::worker::Evidence =
                    serde_json::from_str(&raw).map_err(|_| "Malformed worker evidence")?;
                if evidence.status != *status || evidence.detail != *detail {
                    return Err("Worker result differs from its evidence".into());
                }
            }
            Action::Review { task, .. }
            | Action::Assess { task, .. }
            | Action::Decide { task, .. }
            | Action::ReviewAndRepair { task, .. }
            | Action::ReviewArchitecture { task, .. }
            | Action::Repair { task, .. } => {
                let item = snapshot
                    .tasks
                    .iter()
                    .find(|t| t.id == *task)
                    .ok_or("Unknown task")?;
                let run = item.run.as_ref().ok_or("Task has no run evidence")?;
                self.read_evidence(
                    &run.id,
                    run.evidence_sha256
                        .as_deref()
                        .ok_or("Task has no evidence receipt")?,
                    *task,
                    &snapshot,
                )?;
            }
            _ => {}
        }
        let task = Self::apply(&mut snapshot, &request.action)?;
        snapshot.revision += 1;
        let receipt = Receipt {
            request,
            revision: snapshot.revision,
            task,
        };
        snapshot.receipts.push(receipt.clone());
        if !matches!(receipt.request.action, Action::Finish { .. }) {
            Self::require_completion_capacity(&snapshot)?;
        }
        self.save(&snapshot)?;
        Ok((snapshot, receipt))
    }

    // Reserve both journal slots and serialized bytes while holding the transaction lock.
    // Finish may consume its reservation; legacy overcommitted journals may still finish
    // whenever their actual result fits the existing hard bounds.
    fn require_completion_capacity(snapshot: &Snapshot) -> Result<()> {
        let running: Vec<_> = snapshot
            .tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Running)
            .map(|task| {
                (
                    task.id,
                    task.run
                        .as_ref()
                        .expect("validated running task")
                        .id
                        .clone(),
                )
            })
            .collect();
        if snapshot.receipts.len() + running.len() > MAX_RECEIPTS {
            return Err("Task receipt capacity is reserved for running worker results".into());
        }
        if running.is_empty() {
            return Ok(());
        }
        let mut projected = snapshot.clone();
        for (task, run) in running {
            // Controls are forbidden. Quotes/backslashes have the largest JSON expansion
            // per allowed input byte. ReviewReady is the longest valid terminal status.
            let request = Request {
                correlation: "\"".repeat(160),
                expected_revision: projected.revision,
                action: Action::Finish {
                    task,
                    run,
                    status: TaskStatus::ReviewReady,
                    evidence_sha256: "0".repeat(64),
                    detail: "\"".repeat(1024),
                },
            };
            Self::apply(&mut projected, &request.action)?;
            projected.revision += 1;
            projected.receipts.push(Receipt {
                request,
                revision: projected.revision,
                task,
            });
        }
        if serde_json::to_vec(&projected)
            .map_err(|e| e.to_string())?
            .len()
            > MAX_STORE
        {
            return Err("Task storage capacity is reserved for running worker results".into());
        }
        Ok(())
    }

    fn apply(snapshot: &mut Snapshot, action: &Action) -> Result<u64> {
        if let Action::Repair { task, .. }
        | Action::ReviewAndRepair { task, .. }
        | Action::ReviewArchitecture { task, .. }
        | Action::Start { task, .. }
        | Action::Decide { task, .. }
        | Action::Assess { task, .. }
        | Action::Review { task, .. }
        | Action::ResolveRepair { task } = action
        {
            if snapshot.architecture_obsolete(*task) {
                return Err("An adopted Architect revision supersedes this repair branch".into());
            }
        }
        if let Action::Repair { task, .. }
        | Action::ReviewAndRepair { task, .. }
        | Action::ReviewArchitecture { task, .. }
        | Action::Start { task, .. }
        | Action::Decide { task, .. }
        | Action::Assess { task, .. }
        | Action::Review { task, .. } = action
        {
            if snapshot.resolution_for_family(*task).is_some() {
                return Err("Repair family is already resolved".into());
            }
        }
        if let Action::Repair { task, .. }
        | Action::ReviewAndRepair { task, .. }
        | Action::ReviewArchitecture { task, .. }
        | Action::Start { task, .. } = action
        {
            if let Some(blocker) = snapshot.architecture_blocker(*task) {
                return Err(format!("Architect revision required for task #{blocker}; use /architect-revise {blocker}"));
            }
        }
        match action {
            Action::ReviewArchitecture { task, decision } => {
                if decision.failure != Some(crate::assessment::FailureKind::Architecture)
                    || !decision.proposes_repair()
                {
                    return Err(
                        "Architect routing requires a no-risk architecture failure review".into(),
                    );
                }
                if snapshot.tasks.iter().any(|t| {
                    t.repair_of == Some(*task)
                        && matches!(
                            t.status,
                            TaskStatus::Proposed
                                | TaskStatus::Approved
                                | TaskStatus::Running
                                | TaskStatus::ReviewReady
                                | TaskStatus::NeedsHumanReview
                        )
                }) {
                    return Err("An unresolved repair already exists".into());
                }
                let prior = snapshot
                    .tasks
                    .iter()
                    .find(|t| t.id == *task)
                    .and_then(|t| t.repair_of)
                    .map_or(0, |parent| snapshot.architecture_failures(parent));
                Self::apply(
                    snapshot,
                    &Action::Decide {
                        task: *task,
                        decision: decision.clone(),
                    },
                )?;
                let prior = if snapshot
                    .plan_for_task(*task)
                    .is_some_and(|p| p.architecture.is_some())
                {
                    0
                } else {
                    prior
                };
                if prior >= 1 {
                    Ok(*task)
                } else {
                    Self::apply(
                        snapshot,
                        &Action::Repair {
                            task: *task,
                            reason: decision.reason.clone(),
                        },
                    )
                }
            }
            Action::ResolveRepair { task } => {
                if snapshot.architecture_blocker(*task).is_some() {
                    return Err("Architect revision must be addressed before resolution".into());
                }
                let item = snapshot
                    .tasks
                    .iter()
                    .find(|t| t.id == *task)
                    .ok_or("Unknown repair")?;
                if item.status != TaskStatus::Accepted || item.repair_of.is_none() {
                    return Err("Resolution requires an accepted repair".into());
                }
                if snapshot.resolution_for_family(*task).is_some() {
                    return Err("Repair family is already resolved".into());
                }
                let root = snapshot
                    .repair_root(*task)
                    .ok_or("Invalid repair lineage")?;
                if snapshot.tasks.iter().any(|t| {
                    snapshot.repair_root(t.id) == Some(root)
                        && matches!(
                            t.status,
                            TaskStatus::Proposed
                                | TaskStatus::Approved
                                | TaskStatus::Running
                                | TaskStatus::ReviewReady
                                | TaskStatus::NeedsHumanReview
                        )
                }) {
                    return Err(
                        "Repair family has an unresolved branch or human review hold".into(),
                    );
                }
                let contract = snapshot.acceptance_for_task(*task);
                if !contract.is_empty() {
                    if let Some(decision) = snapshot.decision_for_task(*task) {
                        decision.validate_contract(contract)?;
                        if !decision.outcome.approves() {
                            return Err("Repair lacks passing criterion review".into());
                        }
                    } else if let Some(assessment) = snapshot.assessment_for_task(*task) {
                        assessment.validate_contract(contract)?;
                        if !assessment.accept {
                            return Err("Repair lacks passing criterion review".into());
                        }
                    } else {
                        return Err("Repair requires recorded acceptance criterion review".into());
                    }
                }
                let mut parent = item.repair_of;
                while let Some(id) = parent {
                    let ancestor = snapshot
                        .tasks
                        .iter()
                        .find(|t| t.id == id)
                        .ok_or("Unknown repair ancestor")?;
                    if !matches!(
                        ancestor.status,
                        TaskStatus::Failed | TaskStatus::Rejected | TaskStatus::Cancelled
                    ) || ancestor
                        .run
                        .as_ref()
                        .and_then(|r| r.evidence_sha256.as_ref())
                        .is_none()
                    {
                        return Err(
                            "Resolution ancestors must have terminal unsuccessful run evidence"
                                .into(),
                        );
                    }
                    parent = ancestor.repair_of;
                }
                Ok(*task)
            }

            Action::Assign { task, model } => {
                let item = snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == *task)
                    .ok_or("Unknown task")?;
                item.validate_assignment(model)?;
                item.model = model.clone();
                item.status = TaskStatus::Proposed;
                Ok(*task)
            }

            Action::Plan { plan } => {
                plan.validate()?;
                if snapshot.tasks.len() + plan.tasks.len() > MAX_TASKS {
                    return Err("Task capacity reached".into());
                }
                let first = snapshot.tasks.len() as u64 + 1;
                if let Some(origin) = &plan.architecture {
                    if snapshot.architecture_origin(origin.task)? != *origin {
                        return Err("Stale Architect source binding".into());
                    }
                    let parent = snapshot
                        .tasks
                        .iter()
                        .find(|t| t.id == origin.task)
                        .ok_or("Unknown Architect source")?;
                    if plan.context.as_ref().is_some_and(|context| {
                        parent
                            .run
                            .as_ref()
                            .is_none_or(|run| context.baseline != run.baseline)
                    }) {
                        return Err("Architect context must match the source baseline".into());
                    }
                    let step = &plan.tasks[0];
                    snapshot.tasks.push(Task {
                        id: first,
                        title: step.title.clone(),
                        model: step.model.clone(),
                        dependencies: parent.dependencies.clone(),
                        status: TaskStatus::Proposed,
                        policy: Some(step.policy.clone()),
                        run: None,
                        repair_of: Some(origin.task),
                    });
                    return Ok(first);
                }
                for step in &plan.tasks {
                    let task = Self::apply(
                        snapshot,
                        &Action::Propose {
                            title: step.title.clone(),
                            model: step.model.clone(),
                            dependencies: step
                                .dependencies
                                .iter()
                                .map(|id| first + id - 1)
                                .collect(),
                        },
                    )?;
                    Self::apply(
                        snapshot,
                        &Action::Permit {
                            task,
                            policy: step.policy.clone(),
                        },
                    )?;
                }
                Ok(first)
            }

            Action::Branch { task, name, commit } => {
                let item = snapshot
                    .tasks
                    .iter()
                    .find(|item| item.id == *task)
                    .ok_or("Unknown task")?;
                if item.status != TaskStatus::Accepted
                    || commit.len() != 40
                    || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
                    || *name != crate::branch::name(*task, commit)?
                {
                    return Err(
                        "Only an accepted candidate can be recorded as its review branch".into(),
                    );
                }
                Ok(*task)
            }
            Action::Propose {
                title,
                model,
                dependencies,
            } => {
                if !text_valid(title, 8192) || !text_valid(model, 200) {
                    return Err(
                        "Task title/model is empty, contains controls or exceeds its limit".into(),
                    );
                }
                if snapshot.tasks.len() == MAX_TASKS {
                    return Err("Task capacity reached".into());
                }
                let id = snapshot.tasks.len() as u64 + 1;
                let unique: BTreeSet<_> = dependencies.iter().collect();
                if unique.len() != dependencies.len()
                    || dependencies.iter().any(|d| *d == 0 || *d >= id)
                {
                    return Err("Dependencies must be distinct existing earlier tasks".into());
                }
                snapshot.tasks.push(Task {
                    id,
                    title: title.clone(),
                    model: model.clone(),
                    dependencies: dependencies.clone(),
                    status: TaskStatus::Proposed,
                    policy: None,
                    run: None,
                    repair_of: None,
                });
                Ok(id)
            }
            Action::Repair { task, reason } => {
                if !text_valid(reason, 2048) {
                    return Err("Repair reason must be 1–2048 bytes without controls".into());
                }
                let parent = snapshot
                    .tasks
                    .iter()
                    .find(|t| t.id == *task)
                    .ok_or("Unknown repair parent")?
                    .clone();
                if !matches!(
                    parent.status,
                    TaskStatus::Failed | TaskStatus::Rejected | TaskStatus::Cancelled
                ) || parent.policy.is_none()
                    || parent
                        .run
                        .as_ref()
                        .and_then(|r| r.evidence_sha256.as_ref())
                        .is_none()
                {
                    return Err("Repair requires a failed, rejected or cancelled run with retained evidence".into());
                }
                if snapshot.tasks.iter().any(|t| {
                    t.repair_of == Some(*task)
                        && matches!(
                            t.status,
                            TaskStatus::Proposed
                                | TaskStatus::Approved
                                | TaskStatus::Running
                                | TaskStatus::ReviewReady
                        )
                }) {
                    return Err("An unresolved repair already exists for this task".into());
                }
                if snapshot.tasks.len() == MAX_TASKS {
                    return Err("Task capacity reached".into());
                }
                let id = snapshot.tasks.len() as u64 + 1;
                snapshot.tasks.push(Task {
                    id,
                    title: format!("Repair #{task}: {reason}"),
                    model: parent.model,
                    dependencies: parent.dependencies,
                    status: TaskStatus::Proposed,
                    policy: parent.policy,
                    run: None,
                    repair_of: Some(*task),
                });
                Ok(id)
            }
            Action::Approve { task } | Action::Cancel { task } => {
                let item = snapshot
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == *task)
                    .ok_or("Unknown task")?;
                match action {
                    Action::Approve { .. } if item.status == TaskStatus::Proposed => {
                        item.status = TaskStatus::Approved
                    }
                    Action::Cancel { .. }
                        if matches!(item.status, TaskStatus::Proposed | TaskStatus::Approved) =>
                    {
                        item.status = TaskStatus::Cancelled
                    }
                    _ => return Err("Task transition is not allowed from its current state".into()),
                }
                Ok(*task)
            }
            Action::Permit { task, policy } => {
                policy.validate()?;
                let item = snapshot
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == *task)
                    .ok_or("Unknown task")?;
                if !matches!(item.status, TaskStatus::Proposed | TaskStatus::Approved) {
                    return Err(
                        "Policy can only change before execution; propose a new repair task".into(),
                    );
                }
                item.policy = Some(policy.clone());
                item.status = TaskStatus::Proposed;
                Ok(*task)
            }
            Action::Start {
                task,
                baseline,
                inputs,
            } => {
                if baseline.len() != 40 || !baseline.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("Invalid baseline commit".into());
                }
                let item = snapshot
                    .tasks
                    .iter()
                    .find(|t| t.id == *task)
                    .ok_or("Unknown task")?;
                if item.status != TaskStatus::Approved || item.policy.is_none() {
                    return Err("Task needs explicit /permit policy followed by /approve".into());
                }
                if inputs.iter().map(|input| input.task).collect::<Vec<_>>() != item.dependencies {
                    return Err("Run inputs must match the exact task dependencies".into());
                }
                for input in inputs {
                    if item.repair_of.is_none() {
                        let source = snapshot.dependency_source(input.task)?;
                        let expected = (source.id != input.task).then_some(source.id);
                        if input.source_task != expected {
                            return Err("Dependency input differs from selected resolution".into());
                        }
                    }
                    let parent = snapshot
                        .tasks
                        .iter()
                        .find(|task| task.id == input.source_id())
                        .ok_or("Unknown dependency")?;
                    let run = parent
                        .run
                        .as_ref()
                        .ok_or("Dependency has no completed run")?;
                    if parent.status != TaskStatus::Accepted
                        || run.id != input.run
                        || run.evidence_sha256.as_ref() != Some(&input.evidence_sha256)
                        || input.candidate.len() != 40
                        || !input.candidate.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        return Err("Dependency input is not the exact accepted result".into());
                    }
                }
                if let Some(parent) = item.repair_of {
                    let original = snapshot
                        .tasks
                        .iter()
                        .find(|t| t.id == parent)
                        .and_then(|t| t.run.as_ref())
                        .ok_or("Missing repair parent run")?;
                    if original.baseline != *baseline || original.inputs != *inputs {
                        return Err("Repair must use its parent's committed baseline".into());
                    }
                }
                let item = snapshot.tasks.iter_mut().find(|t| t.id == *task).unwrap();
                item.status = TaskStatus::Running;
                item.run = Some(TaskRun {
                    id: format!("task-{task}-run-{}", snapshot.revision + 1),
                    baseline: baseline.clone(),
                    inputs: inputs.clone(),
                    evidence_sha256: None,
                    detail: "Worker claimed; effects may be in progress".into(),
                });
                Ok(*task)
            }
            Action::Finish {
                task,
                run,
                status,
                evidence_sha256,
                detail,
            } => {
                if !matches!(
                    status,
                    TaskStatus::ReviewReady | TaskStatus::Failed | TaskStatus::Cancelled
                ) || evidence_sha256.len() != 64
                    || !evidence_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                    || !text_valid(detail, 1024)
                {
                    return Err("Invalid worker result".into());
                }
                let item = snapshot
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == *task)
                    .ok_or("Unknown task")?;
                let bound = item.run.as_mut().ok_or("Task has no run")?;
                if item.status != TaskStatus::Running || bound.id != *run {
                    return Err("Stale or mismatched worker result".into());
                }
                bound.evidence_sha256 = Some(evidence_sha256.clone());
                bound.detail = detail.clone();
                item.status = status.clone();
                Ok(*task)
            }
            Action::ReviewAndRepair { task, decision } => {
                if !decision.proposes_repair() {
                    return Err("Automatic repair requires Needs repair or Rejected without unresolved risk".into());
                }
                if snapshot.tasks.iter().any(|child| {
                    child.repair_of == Some(*task) && child.status == TaskStatus::NeedsHumanReview
                }) {
                    return Err(
                        "An unresolved repair is held for human review; resolve its review first"
                            .into(),
                    );
                }
                Self::apply(
                    snapshot,
                    &Action::Decide {
                        task: *task,
                        decision: decision.clone(),
                    },
                )?;
                Self::apply(
                    snapshot,
                    &Action::Repair {
                        task: *task,
                        reason: decision.reason.clone(),
                    },
                )
            }
            Action::Decide { task, decision } => {
                use crate::assessment::Outcome;
                decision.validate_contract(snapshot.acceptance_for_task(*task))?;
                let item = snapshot
                    .tasks
                    .iter()
                    .find(|item| item.id == *task)
                    .ok_or("Unknown task")?;
                if !matches!(
                    item.status,
                    TaskStatus::ReviewReady | TaskStatus::Failed | TaskStatus::NeedsHumanReview
                ) {
                    return Err("Review outcomes require retained review-ready, failed or human-review work".into());
                }
                let run = item.run.as_ref().ok_or("Task has no run evidence")?;
                let passed = snapshot.receipts.iter().any(|receipt| matches!(&receipt.request.action,
                    Action::Finish { task: id, run: run_id, status: TaskStatus::ReviewReady, .. } if *id == *task && run_id == &run.id));
                if decision.outcome.approves() && !passed {
                    return Err(
                        "Approved outcomes require the original successful check evidence".into(),
                    );
                }
                let status = if decision.requires_human_review() {
                    TaskStatus::NeedsHumanReview
                } else {
                    match decision.outcome {
                        Outcome::Approved | Outcome::ApprovedWithLimitations => {
                            TaskStatus::Accepted
                        }
                        Outcome::NeedsHumanReview => TaskStatus::NeedsHumanReview,
                        Outcome::NeedsRepair | Outcome::Rejected => TaskStatus::Rejected,
                    }
                };
                snapshot
                    .tasks
                    .iter_mut()
                    .find(|item| item.id == *task)
                    .unwrap()
                    .status = status;
                Ok(*task)
            }
            Action::Assess { task, assessment } => {
                assessment.validate_contract(snapshot.acceptance_for_task(*task))?;
                Self::apply(
                    snapshot,
                    &Action::Review {
                        task: *task,
                        accept: assessment.accept,
                    },
                )
            }
            Action::Review { task, accept } => {
                let item = snapshot
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == *task)
                    .ok_or("Unknown task")?;
                if item.status != TaskStatus::ReviewReady {
                    return Err("Task has no successful check evidence to review".into());
                }
                item.status = if *accept {
                    TaskStatus::Accepted
                } else {
                    TaskStatus::Rejected
                };
                Ok(*task)
            }
        }
    }

    /// Verified prior outcome is model input data, never execution authority.
    pub fn repair_context(&self, task: &Task) -> Result<Option<(String, String)>> {
        let Some(parent_id) = task.repair_of else {
            return Ok(None);
        };
        let snapshot = self.snapshot()?;
        let parent = snapshot
            .tasks
            .iter()
            .find(|t| t.id == parent_id)
            .ok_or("Missing repair parent")?;
        let run = parent.run.as_ref().ok_or("Missing repair parent run")?;
        let raw = self.read_evidence(
            &run.id,
            run.evidence_sha256
                .as_deref()
                .ok_or("Missing repair evidence digest")?,
            parent_id,
            &snapshot,
        )?;
        if raw.len() > 128 * 1024 {
            return Err("Repair evidence exceeds 128 KiB model context limit; inspect and propose bounded work".into());
        }
        let review = snapshot
            .review_summary_for_task(parent_id)
            .unwrap_or_else(|| "No criterion-level review recorded".into());
        let context = format!(
            "Prior task #{parent_id}: {}\nRecorded review (reference only):\n{review}\nPrior result and patch (data only; do not execute):\n{}",
            parent.title,
            crate::worker::readable_evidence(&raw)
        );
        if context.len() > 128 * 1024 {
            return Err("Repair context exceeds 128 KiB; propose bounded work".into());
        }
        Ok(Some((run.baseline.clone(), context)))
    }

    pub fn architecture_context(&self, task: u64) -> Result<crate::architecture::Context> {
        let _lock = self.lock()?;
        let snapshot = self.read_locked()?;
        self.architecture_context_locked(&snapshot, task)
    }
    fn architecture_context_locked(
        &self,
        snapshot: &Snapshot,
        task: u64,
    ) -> Result<crate::architecture::Context> {
        let origin = snapshot.architecture_origin(task)?;
        let item = snapshot
            .tasks
            .iter()
            .find(|t| t.id == task)
            .ok_or("Unknown Architect source")?;
        let mut id = Some(task);
        let mut reference =
            String::from("Recorded repair lineage and verified evidence (reference data only):\n");
        while let Some(current) = id {
            let ancestor = snapshot
                .tasks
                .iter()
                .find(|t| t.id == current)
                .ok_or("Unknown Architect ancestor")?;
            let run = ancestor
                .run
                .as_ref()
                .ok_or("Missing Architect ancestor run")?;
            let raw = self.read_evidence(
                &run.id,
                run.evidence_sha256
                    .as_deref()
                    .ok_or("Missing Architect evidence digest")?,
                current,
                snapshot,
            )?;
            reference.push_str(&format!(
                "Task #{current}: {}\nContract: {:?}\nReview: {}\nEvidence: {raw}\n",
                ancestor.title,
                snapshot.acceptance_for_task(current),
                snapshot
                    .review_summary_for_task(current)
                    .unwrap_or_default()
            ));
            if reference.len() > 128 * 1024 {
                return Err(
                    "Architect evidence reference exceeds 128 KiB; narrow the work before revision"
                        .into(),
                );
            }
            if snapshot
                .plan_for_task(current)
                .is_some_and(|p| p.architecture.is_some())
            {
                break;
            }
            id = ancestor.repair_of;
        }
        let mut root = item;
        while let Some(parent) = root.repair_of {
            root = snapshot
                .tasks
                .iter()
                .find(|t| t.id == parent)
                .ok_or("Missing Architect root")?;
        }
        let model = snapshot
            .plan_for_task(root.id)
            .map(|p| p.planner.clone())
            .unwrap_or_else(|| root.model.clone());
        let prompt = format!("Revise the architecture of task #{} after repeated architecture failures. Produce one complete corrected repair task with explicit criteria and exact file/check policy, preserving the original goal: {}", root.id, root.title);
        if prompt.len() > 8192 {
            return Err("Architect task goal exceeds prompt limit".into());
        }
        Ok(crate::architecture::Context {
            baseline: item
                .run
                .as_ref()
                .ok_or("Missing Architect baseline")?
                .baseline
                .clone(),
            origin,
            prompt,
            model,
            reference,
            revision: snapshot.revision,
        })
    }

    pub fn conversation_directory(&self) -> Result<PathBuf> {
        let _lock = self.lock()?;
        Ok(self.root.clone())
    }

    pub fn run_directory(&self, id: &str) -> Result<PathBuf> {
        if id.len() > 100
            || !id.starts_with("task-")
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("Invalid run identity".into());
        }
        Ok(self.root.join("runs").join(id))
    }

    /// Held from before the durable claim until the worker has stopped publishing.
    /// OS ownership is released on process death; it is never inferred from time.
    pub fn claim_worker(&self, task: u64) -> Result<WorkerOwner> {
        let scope = self.understanding();
        let scope_guard = scope.lock()?;
        scope_guard.ensure_open()?;
        let _store_lock = self.lock()?;
        let snapshot = self.read_locked()?;
        if snapshot.architecture_obsolete(task) {
            return Err("An adopted Architect revision supersedes this worker branch".into());
        }
        if snapshot.architecture_blocker(task).is_some() {
            return Err("Architect revision required before more worker execution".into());
        }
        if let Some(plan) = snapshot.plan_for_task(task) {
            scope_guard.ensure_binding(plan.scope.as_deref())?;
        }
        if !snapshot
            .tasks
            .iter()
            .any(|item| item.id == task && item.status == TaskStatus::Approved)
        {
            return Err("Only an approved task can acquire a new worker owner".into());
        }
        let file = regular_file(&self.root.join(format!("worker-{task}.lock")), true, true)?;
        file.try_lock()
            .map_err(|_| "Worker owner is still active")?;
        Ok(WorkerOwner {
            _lock: StoreLock(file),
        })
    }

    fn recovery_owner(&self, task: u64) -> Result<Option<WorkerOwner>> {
        // Never create an ownership marker while inspecting a legacy claim.
        let file = regular_file(&self.root.join(format!("worker-{task}.lock")), false, true)
            .map_err(|_| "No worker ownership marker; legacy run requires inspection")?;
        match file.try_lock() {
            Ok(()) => Ok(Some(WorkerOwner {
                _lock: StoreLock(file),
            })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(error) => Err(format!("Cannot verify worker ownership: {error}")),
        }
    }

    fn recovery_request(&self, task: u64, snapshot: &Snapshot) -> Result<Request> {
        let item = snapshot
            .tasks
            .iter()
            .find(|t| t.id == task)
            .ok_or("Unknown task")?;
        if item.status != TaskStatus::Running {
            return Err("Task does not need recovery".into());
        }
        let run = item.run.as_ref().ok_or("Missing run claim")?;
        let mut bytes = Vec::new();
        regular_file(
            &self.run_directory(&run.id)?.join("evidence.json"),
            false,
            false,
        )?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("Evidence exceeds recovery bound".into());
        }
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let raw = self.read_evidence(&run.id, &digest, task, snapshot)?;
        let evidence: crate::worker::Evidence =
            serde_json::from_str(&raw).map_err(|_| "Invalid evidence")?;
        if !matches!(
            evidence.status,
            TaskStatus::ReviewReady | TaskStatus::Failed | TaskStatus::Cancelled
        ) {
            return Err("Evidence has no terminal worker outcome".into());
        }
        Ok(Request {
            correlation: format!("finish:{}", run.id),
            expected_revision: snapshot.revision,
            action: Action::Finish {
                task,
                run: run.id.clone(),
                status: evidence.status,
                evidence_sha256: digest,
                detail: evidence.detail,
            },
        })
    }

    pub fn run_observations(&self, snapshot: &Snapshot) -> std::collections::BTreeMap<u64, String> {
        snapshot.tasks.iter().filter(|t| t.status == TaskStatus::Running).map(|task| {
            let observation = match self.recovery_owner(task.id) {
                Ok(None) => "Worker owner active in another process".into(),
                Ok(Some(_owner)) => match self.recovery_request(task.id, snapshot) {
                    Ok(_) => "Worker stopped · saved result available · /recover ID".into(),
                    Err(_) if task.run.as_ref().and_then(|run| self.run_directory(&run.id).ok()).is_some_and(|path| crate::run_boundary::interrupted_before_check(&path, task).is_ok()) => "Worker stopped before check launch · /recover ID records interruption without replay".into(),
                    Err(_) if task.run.as_ref().and_then(|run| self.run_directory(&run.id).ok()).is_some_and(|path| crate::run_boundary::interrupted_after_check(&path, task, &snapshot.mission).is_ok()) => "Worker stopped after check · saved terminal check result · /recover ID records failure without replay".into(),
                    Err(_) => "Worker stopped · outcome unknown · inspect retained run; do not rerun effects".into(),
                },
                Err(_) => "Worker ownership unverified · inspect retained run; do not rerun effects".into(),
            };
            (task.id, observation)
        }).collect()
    }

    /// Reconcile saved evidence or a proven check-boundary interruption. Never replay effects.
    pub fn recover(&self, task: u64) -> Result<(Snapshot, String)> {
        let _owner = self
            .recovery_owner(task)?
            .ok_or("Worker is still active; recovery refused")?;
        let snapshot = self.snapshot()?;
        let item = snapshot
            .tasks
            .iter()
            .find(|t| t.id == task)
            .ok_or("Unknown task")?;
        if item.status != TaskStatus::Running {
            self.evidence(task)?;
            return Ok((
                snapshot,
                "Task result already acknowledged; no effects replayed".into(),
            ));
        }
        let run_directory =
            self.run_directory(&item.run.as_ref().ok_or("Missing run claim")?.id)?;
        let (request, detail) = match self.recovery_request(task, &snapshot) {
            Ok(request) => (
                request,
                "Saved worker result recovered; no effects replayed",
            ),
            Err(error) => {
                // Both interruption proofs require absent final evidence. An existing
                // malformed file stays intact and cannot be replaced by a checkpoint.
                let detail = if crate::run_boundary::interrupted_before_check(&run_directory, item)
                    .is_ok()
                {
                    crate::run_boundary::record_interrupted(&run_directory, item)?;
                    "Interrupted worker recorded as failed before check launch; no effects replayed"
                } else if crate::run_boundary::interrupted_after_check(
                    &run_directory,
                    item,
                    &snapshot.mission,
                )
                .is_ok()
                {
                    crate::run_boundary::record_interrupted_after_check(
                        &run_directory,
                        item,
                        &snapshot.mission,
                    )?;
                    "Interrupted worker recorded as failed after check; candidate not finalized; no effects replayed"
                } else {
                    return Err(format!(
                        "Outcome unknown; retained evidence cannot be recovered: {error}. No effects replayed"
                    ));
                };
                (self.recovery_request(task, &snapshot)?, detail)
            }
        };
        let run = item.run.as_ref().ok_or("Missing run claim")?;
        let directory = self.run_directory(&run.id)?;
        regular_file(&directory.join("evidence.json"), false, false)?
            .sync_all()
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        File::open(&directory)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())?;
        let (snapshot, _) = self.transact(request)?;
        Ok((snapshot, detail.into()))
    }

    fn read_evidence(
        &self,
        run: &str,
        digest: &str,
        task: u64,
        snapshot: &Snapshot,
    ) -> Result<String> {
        let item = snapshot
            .tasks
            .iter()
            .find(|t| t.id == task)
            .ok_or("Unknown evidence task")?;
        if item.run.as_ref().is_none_or(|bound| bound.id != run) {
            return Err("Evidence is not bound to this task".into());
        }
        let mut bytes = Vec::new();
        regular_file(
            &self.run_directory(run)?.join("evidence.json"),
            false,
            false,
        )?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 || format!("{:x}", Sha256::digest(&bytes)) != digest {
            return Err("Evidence size or digest mismatch; review blocked".into());
        }
        let evidence: crate::worker::Evidence =
            serde_json::from_slice(&bytes).map_err(|_| "Malformed worker evidence")?;
        if let Some(agent) = &evidence.agent {
            agent.validate(run, &item.model)?;
        }
        if evidence.candidate_commit.as_ref().is_some_and(|commit| {
            commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err("Invalid evidence candidate commit identity".into());
        }
        if evidence.run != run || evidence.baseline != item.run.as_ref().unwrap().baseline {
            return Err("Evidence run/baseline mismatch".into());
        }
        if evidence.status == TaskStatus::ReviewReady
            && evidence.check.as_ref().is_none_or(|check| {
                check.status != "completed"
                    || check.exit_code != Some(0)
                    || check.reconciliation_required
            })
        {
            return Err("Review evidence lacks a successful bounded check receipt".into());
        }
        String::from_utf8(bytes).map_err(|_| "Evidence is not UTF-8".into())
    }

    pub fn evidence(&self, task: u64) -> Result<String> {
        let _lock = self.lock()?;
        let snapshot = self.read_locked()?;
        let item = snapshot
            .tasks
            .iter()
            .find(|t| t.id == task)
            .ok_or("Unknown task")?;
        let run = item.run.as_ref().ok_or("Task has not run")?;
        self.read_evidence(
            &run.id,
            run.evidence_sha256
                .as_deref()
                .ok_or("Evidence has not been acknowledged")?,
            task,
            &snapshot,
        )
    }

    fn save(&self, snapshot: &Snapshot) -> Result<()> {
        let bytes = serde_json::to_vec(snapshot).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_STORE {
            return Err("Task state exceeds 4 MiB".into());
        }
        let temporary = self.root.join(format!(
            ".tasks-{}-{}.tmp",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&temporary, self.root.join("tasks.json")).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            File::open(&self.root)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| format!("Task save outcome uncertain; retry the same request: {e}"))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}
