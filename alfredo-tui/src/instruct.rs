//! Owner instructions to an agent watched in the agent view. Like autopilot,
//! each instruction only chooses the next command the owner could have typed
//! (`/cancel-task`, `/repair`, `/review`, `/after`, `/permit`, `/approve`, `/run`,
//! `/plan-revise`); the terminal saves and dispatches it through the normal
//! console intent path, so task policy, evidence verification, locks and receipts
//! stay authoritative. A note approves the inherited file/check policy only: it
//! never widens files or changes the check. Held (risk or human-review) work is
//! refused. While an instruction is applied its task family is held from
//! autopilot, so no decision is made twice.
use crate::{
    agent_view::{Note, Target},
    autopilot::{Autopilot, Submission},
    command_intent::Intent,
    task_control::TaskControl,
    tasks::{Action, Snapshot, Task, TaskStatus},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// Repair reason prefix naming an owner instruction.
pub const OWNER: &str = "Owner: ";
/// Separates a queued note from the check failure it was applied to.
pub const AFTER_CHECK: &str = " · after check: ";
/// Leaves room for the prefix and a failure tail in a 2048-byte repair reason.
const MAX_NOTE: usize = 1024;
const VERSION: u32 = 1;
const MAX_STATE: usize = 256 * 1024;
/// Finished instructions kept for the agent view.
const HISTORY: usize = 64;
/// Refusals of one step without effect before the instruction stops.
const ATTEMPTS: u32 = 3;
/// Header of the owner's instruction at the top of a worker request.
pub const HEADER: &str =
    "OWNER INSTRUCTION (from the repository owner; follow it within the approved files and check below)";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    /// Cancel the generation, then rerun with the note (a repair outside the budget).
    Steer,
    /// Applied after the running check: repair on failure, dropped on pass.
    AfterCheck,
    Repair,
    /// Record the review as needs-repair, then repair with the note.
    ReviewRepair,
    FollowUp,
    Revise,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instruction {
    pub id: u64,
    /// Task family repair root; 0 for the architect.
    pub root: u64,
    /// The attempt the note was given to (0 for the architect).
    pub task: u64,
    pub note: String,
    pub effect: Effect,
    /// Correlation of the command creating the child or revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<u64>,
    #[serde(default)]
    pub cancel_requested: bool,
    /// Current effect for the agent view, or the final outcome.
    pub status: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    next: u64,
    instructions: Vec<Instruction>,
}

enum Step {
    Wait(String),
    Command(String, String),
    Done(String),
    Fail(String),
}

#[derive(Default)]
pub struct Instructions {
    path: Option<PathBuf>,
    next: u64,
    items: Vec<Instruction>,
    /// The last submitted step: instruction id, attempt key, intent.
    last: Option<(u64, String, Intent)>,
    attempts: BTreeMap<String, u32>,
    transient: u32,
    notice: Option<String>,
}

/// The owner instruction file of one conversation set in a mission directory.
pub fn state_path(directory: &Path, conversation: &str) -> PathBuf {
    directory.join(format!(
        "owner-{:x}.json",
        Sha256::digest(conversation.as_bytes())
    ))
}

/// The owner's note of a repair task, from the receipt that created it.
pub fn owner_note(snapshot: &Snapshot, task: u64) -> Option<String> {
    let reason = snapshot
        .receipts
        .iter()
        .find_map(|receipt| match &receipt.request.action {
            Action::Repair { reason, .. } if receipt.task == task => Some(reason),
            Action::ReviewAndRepair { decision, .. } if receipt.task == task => {
                Some(&decision.reason)
            }
            _ => None,
        })?;
    let note = reason.strip_prefix(OWNER)?;
    let note = note.split(AFTER_CHECK).next().unwrap_or(note).trim();
    (!note.is_empty()).then(|| note.to_owned())
}

/// Latest attempt of the family rooted at `root`.
pub fn head(snapshot: &Snapshot, root: u64) -> Option<&Task> {
    snapshot
        .tasks
        .iter()
        .filter(|task| snapshot.repair_root(task.id) == Some(root))
        .max_by_key(|task| task.id)
}

fn clean(note: &str) -> Result<String, String> {
    let note: String = note
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let note = note.split_whitespace().collect::<Vec<_>>().join(" ");
    if note.is_empty() {
        return Err("Type an instruction for the agent".into());
    }
    if note.len() > MAX_NOTE {
        return Err(format!("Instructions are limited to {MAX_NOTE} bytes"));
    }
    Ok(note)
}

fn find(snapshot: &Snapshot, id: u64) -> Option<&Task> {
    snapshot.tasks.iter().find(|task| task.id == id)
}

fn checking(tasks: &TaskControl, task: u64) -> bool {
    tasks
        .worker_stage(task)
        .is_some_and(|(stage, _, _)| stage == "Running approved check")
}

impl Instructions {
    /// Load this conversation set's instructions. Active ones continue: each
    /// step is re-derived from current task state; nothing is replayed.
    pub fn open(directory: &Path, conversation: &str) -> Result<Self, String> {
        let path = state_path(directory, conversation);
        let mut this = Self {
            path: Some(path.clone()),
            ..Default::default()
        };
        match fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > MAX_STATE {
                    return Err("Owner instruction state exceeds its size bound".into());
                }
                let saved: Saved = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Owner instruction state is invalid ({error})"))?;
                if saved.version != VERSION {
                    return Err("Unsupported owner instruction state".into());
                }
                this.next = saved.next;
                this.items = saved.instructions;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Cannot read owner instructions: {error}")),
        }
        Ok(this)
    }

    fn persist(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        let done = self.items.iter().filter(|item| item.done).count();
        if done > HISTORY {
            let mut drop = done - HISTORY;
            self.items.retain(|item| {
                let remove = item.done && drop > 0;
                drop -= usize::from(remove);
                !remove
            });
        }
        let saved = Saved {
            version: VERSION,
            next: self.next,
            instructions: self.items.clone(),
        };
        let result = (|| -> Result<(), String> {
            let bytes = serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())?;
            let directory = path.parent().ok_or("Missing state directory")?;
            let temporary = directory.join(format!(".owner-{}.tmp", std::process::id()));
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            fs::rename(&temporary, path).map_err(|e| e.to_string())?;
            File::open(directory)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| e.to_string())
        })();
        if let Err(error) = result {
            self.notice = Some(format!("Owner instruction save failed: {error}"));
        }
    }

    /// Task families (repair roots) an active instruction holds from autopilot.
    pub fn held(&self) -> BTreeSet<u64> {
        self.items
            .iter()
            .filter(|item| !item.done && item.root != 0)
            .map(|item| item.root)
            .chain(
                self.items
                    .iter()
                    .filter(|item| !item.done && item.effect == Effect::FollowUp)
                    .filter_map(|item| item.child),
            )
            .collect()
    }

    /// An architect instruction is waiting to revise the draft.
    pub fn planning_held(&self) -> bool {
        self.items
            .iter()
            .any(|item| !item.done && item.effect == Effect::Revise)
    }

    pub fn active(&self) -> bool {
        self.items.iter().any(|item| !item.done)
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.items
    }

    /// Owner notes of a family (or the architect) for the agent view.
    pub fn notes(&self, target: Target) -> Vec<Note> {
        let root = match target {
            Target::Task(root) => root,
            Target::Architect => 0,
        };
        self.items
            .iter()
            .filter(|item| item.root == root)
            .map(|item| Note {
                task: item.task,
                text: item.note.clone(),
                status: item.status.clone(),
            })
            .collect()
    }

    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// Whether a not yet dispatched autopilot command would decide for a family
    /// (or a plan) the owner is instructing. The terminal withdraws it.
    pub fn supersedes(&self, intent: &Intent, snapshot: Option<&Snapshot>) -> bool {
        let held = self.held();
        match intent {
            Intent::Task { request } => match &request.action {
                Action::Plan { .. } => self.planning_held(),
                Action::Approve { task }
                | Action::Review { task, .. }
                | Action::Assess { task, .. }
                | Action::Decide { task, .. }
                | Action::ReviewAndRepair { task, .. }
                | Action::Repair { task, .. }
                | Action::ResolveRepair { task }
                | Action::Cancel { task } => snapshot
                    .and_then(|snapshot| snapshot.repair_root(*task))
                    .is_some_and(|root| held.contains(&root)),
                _ => false,
            },
            Intent::Planner { .. } => self.planning_held(),
            _ => false,
        }
    }

    /// Classify and record an instruction for `target`. Nothing is written to the
    /// task store here; the steps follow through `tick`.
    pub fn give(
        &mut self,
        tasks: &TaskControl,
        target: Target,
        note: &str,
    ) -> Result<String, String> {
        let note = clean(note)?;
        let (root, task, effect, status, notice) = match target {
            Target::Architect => {
                if !tasks.planner.active() && tasks.planner.checkpoint().is_none() {
                    return Err("No plan is being drafted; /plan REQUEST starts one".into());
                }
                if self.planning_held() {
                    return Err("A plan revision is already waiting; let it finish first".into());
                }
                (
                    0,
                    0,
                    Effect::Revise,
                    if tasks.planner.active() {
                        "queued · revises the draft when it is complete"
                    } else {
                        "revising the draft"
                    },
                    "Plan revision requested".to_string(),
                )
            }
            Target::Task(root) => {
                let snapshot = tasks.snapshot.as_ref().ok_or("Task state is loading")?;
                let head = head(snapshot, root).ok_or(format!("Task #{root} not found"))?;
                if self.held().contains(&root) {
                    return Err(format!(
                        "An instruction for #{} is still being applied; wait for it",
                        head.id
                    ));
                }
                let id = head.id;
                let (effect, status, notice) = match head.status {
                    TaskStatus::Proposed | TaskStatus::Approved
                        if tasks.workers.contains_key(&id) =>
                    {
                        return Err(format!("#{id} is starting; send the note once it runs"))
                    }
                    TaskStatus::Running if !tasks.workers.contains_key(&id) => {
                        return Err(format!(
                            "#{id} has no live worker in this terminal; /recover {id} first"
                        ))
                    }
                    TaskStatus::Running if checking(tasks, id) => (
                        Effect::AfterCheck,
                        "queued · applies if the check fails",
                        format!("Note queued for #{id} · applies if the check fails"),
                    ),
                    TaskStatus::Running => (
                        Effect::Steer,
                        "steering · cancelling the generation",
                        format!("Steering #{id} · cancelling the generation"),
                    ),
                    TaskStatus::Failed | TaskStatus::Rejected => (
                        Effect::Repair,
                        "repair with this note",
                        format!("Repairing #{id} with your note"),
                    ),
                    TaskStatus::Cancelled if head.run.is_some() => (
                        Effect::Repair,
                        "repair with this note",
                        format!("Repairing #{id} with your note"),
                    ),
                    TaskStatus::ReviewReady => (
                        Effect::ReviewRepair,
                        "needs repair · repair with this note",
                        format!(
                            "Review of #{id} recorded as needs repair; repairing with your note"
                        ),
                    ),
                    TaskStatus::Accepted => (
                        Effect::FollowUp,
                        "follow-up task",
                        format!("Follow-up of #{id} with the same files and check"),
                    ),
                    TaskStatus::NeedsHumanReview => return Err(format!(
                        "#{id} is held for human review; resolve it with /review {id} JSON first"
                    )),
                    // A repair proposed but not started: the note replaces it.
                    TaskStatus::Proposed | TaskStatus::Approved if head.repair_of.is_some() => {
                        let parent = head.repair_of.unwrap();
                        let parent_task = find(snapshot, parent)
                            .ok_or(format!("Repair parent #{parent} not found"))?;
                        if !matches!(
                            parent_task.status,
                            TaskStatus::Failed | TaskStatus::Rejected | TaskStatus::Cancelled
                        ) {
                            return Err(format!("#{id} has not started yet"));
                        }
                        self.push(
                            root,
                            parent,
                            note.clone(),
                            Effect::Repair,
                            "repair with this note",
                        );
                        return Ok(format!(
                            "Repairing #{parent} with your note instead of #{id}"
                        ));
                    }
                    TaskStatus::Proposed | TaskStatus::Approved | TaskStatus::Cancelled => {
                        return Err(format!(
                            "#{id} has not started yet; approve and run it first"
                        ))
                    }
                };
                (root, id, effect, status, notice)
            }
        };
        self.push(root, task, note, effect, status);
        Ok(notice)
    }

    fn push(&mut self, root: u64, task: u64, note: String, effect: Effect, status: &str) {
        self.next += 1;
        self.items.push(Instruction {
            id: self.next,
            root,
            task,
            note,
            effect,
            request: None,
            child: None,
            cancel_requested: false,
            status: status.into(),
            done: false,
        });
        self.persist();
    }

    fn finish(&mut self, id: u64, status: String, notice: bool) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.done = true;
            item.status = status.clone();
        }
        if notice {
            self.notice = Some(status);
        }
        self.last = None;
        self.persist();
    }

    /// Choose at most one next command for the oldest active instruction.
    pub fn tick(tasks: &mut TaskControl, autopilot: &mut Autopilot) -> Option<Submission> {
        let mut owner = std::mem::take(&mut tasks.owner);
        let result = owner.step(tasks, autopilot);
        tasks.owner = owner;
        result
    }

    /// `give` on the registry held by `tasks`.
    pub fn give_to(tasks: &mut TaskControl, target: Target, note: &str) -> Result<String, String> {
        let mut owner = std::mem::take(&mut tasks.owner);
        let result = owner.give(tasks, target, note);
        tasks.owner = owner;
        result
    }

    fn step(&mut self, tasks: &mut TaskControl, autopilot: &mut Autopilot) -> Option<Submission> {
        if tasks.pending || tasks.writing {
            return None;
        }
        if let Some((id, key, intent)) = self.last.clone() {
            let launched =
                matches!(&intent, Intent::Run { task, .. } if tasks.workers.contains_key(task));
            if !launched && tasks.intent_pending(&intent) {
                return None;
            }
            if let Some(error) = tasks.intent_error(&intent).filter(|_| !launched) {
                if tasks.intent_transient(&intent) {
                    // Nothing was written; prepare again on the current state.
                    self.transient += 1;
                    if let Some(count) = self.attempts.get_mut(&key) {
                        *count = count.saturating_sub(1);
                    }
                    if self.transient >= crate::dispatch::TRANSIENT_LIMIT {
                        self.transient = 0;
                        self.finish(
                            id,
                            format!("stopped: task state kept changing: {error}"),
                            true,
                        );
                        return None;
                    }
                } else {
                    self.finish(id, format!("refused: {error}"), true);
                    return None;
                }
            } else {
                self.transient = 0;
            }
            self.last = None;
        }
        let snapshot = tasks.snapshot.clone()?;
        let index = self.items.iter().position(|item| !item.done)?;
        let mut item = self.items[index].clone();
        let step = self.decide(&mut item, &snapshot, tasks, autopilot);
        let changed = self.items[index] != item;
        self.items[index] = item.clone();
        match step {
            Step::Wait(status) => {
                if self.items[index].status != status {
                    self.items[index].status = status;
                    self.persist();
                } else if changed {
                    self.persist();
                }
                None
            }
            Step::Done(status) => {
                let notice = status.starts_with("Note not needed");
                self.finish(item.id, status, notice);
                None
            }
            Step::Fail(reason) => {
                self.finish(item.id, format!("refused: {reason}"), true);
                None
            }
            Step::Command(key, text) => {
                let attempts = self.attempts.entry(key.clone()).or_default();
                *attempts += 1;
                if *attempts > ATTEMPTS {
                    self.finish(item.id, format!("stopped: “{text}” had no effect"), true);
                    return None;
                }
                let model = snapshot
                    .tasks
                    .iter()
                    .find(|task| task.id == item.task)
                    .map_or_else(|| autopilot_model(autopilot), |task| task.model.clone());
                let intent = match tasks.prepare_command(&text, &model) {
                    Ok(Some(intent)) => intent,
                    Ok(None) => {
                        self.finish(item.id, format!("refused: “{text}” is not a command"), true);
                        return None;
                    }
                    Err(error) => {
                        self.finish(item.id, format!("refused: {error}"), true);
                        return None;
                    }
                };
                let correlation = intent.correlation().to_string();
                let current = &mut self.items[index];
                if key.starts_with("create-") || key.starts_with("revise") {
                    current.request = Some(correlation.clone());
                }
                if key.starts_with("cancel-worker") {
                    current.cancel_requested = true;
                }
                if key.starts_with("revise") {
                    autopilot.adopt_revision(&correlation);
                }
                self.persist();
                self.last = Some((item.id, key, intent.clone()));
                Some(Submission {
                    text: format!("You · {}", first_line(&text)),
                    intent,
                })
            }
        }
    }

    fn decide(
        &mut self,
        item: &mut Instruction,
        snapshot: &Snapshot,
        tasks: &TaskControl,
        autopilot: &mut Autopilot,
    ) -> Step {
        if item.effect == Effect::Revise {
            if item.request.is_some() {
                return Step::Done("plan revision requested".into());
            }
            if tasks.planner.active() {
                return Step::Wait("queued · revises the draft when it is complete".into());
            }
            if tasks.planner.checkpoint().is_none() {
                return Step::Fail("no plan draft to revise".into());
            }
            return Step::Command("revise".into(), format!("/plan-revise {}", item.note));
        }
        let Some(task) = find(snapshot, item.task).cloned() else {
            return Step::Fail(format!("task #{} not found", item.task));
        };
        // A follow-up or repair child already exists: approve and start it.
        if let Some(child) = item.child.or_else(|| {
            let request = item.request.as_ref()?;
            snapshot
                .receipts
                .iter()
                .find(|receipt| &receipt.request.correlation == request)
                .map(|receipt| receipt.task)
        }) {
            if item.child != Some(child) {
                item.child = Some(child);
                if item.effect == Effect::FollowUp && autopilot.roots(tasks).contains(&item.root) {
                    autopilot.adopt(child);
                }
            }
            return self.launch(item, snapshot, tasks, &task, child);
        }
        if item.request.is_some() {
            // The creating command left no receipt and no refusal: retry it.
            item.request = None;
        }
        match item.effect {
            Effect::Steer => match task.status {
                TaskStatus::Running if !item.cancel_requested => {
                    if !tasks.workers.contains_key(&task.id) {
                        return Step::Fail(format!(
                            "#{} has no live worker; /recover {} first",
                            task.id, task.id
                        ));
                    }
                    Step::Command(
                        format!("cancel-worker-{}", task.id),
                        format!("/cancel-task {}", task.id),
                    )
                }
                TaskStatus::Running => Step::Wait("steering · cancelling the generation".into()),
                _ => {
                    // The run ended (cancelled, or finished first): continue as a repair.
                    item.effect = match task.status {
                        TaskStatus::ReviewReady => Effect::ReviewRepair,
                        TaskStatus::Accepted => Effect::FollowUp,
                        _ => Effect::Repair,
                    };
                    self.decide(item, snapshot, tasks, autopilot)
                }
            },
            Effect::AfterCheck => match task.status {
                TaskStatus::Running => Step::Wait("queued · applies if the check fails".into()),
                TaskStatus::ReviewReady | TaskStatus::Accepted => {
                    Step::Done("Note not needed: check passed".into())
                }
                TaskStatus::Failed | TaskStatus::Rejected | TaskStatus::Cancelled => {
                    self.create_repair(item, snapshot, tasks, &task)
                }
                _ => Step::Fail(format!(
                    "#{} is {}",
                    task.id,
                    crate::dashboard::state_word(&task)
                )),
            },
            Effect::Repair => self.create_repair(item, snapshot, tasks, &task),
            Effect::ReviewRepair => match task.status {
                TaskStatus::ReviewReady => {
                    let criteria: Vec<_> = snapshot
                        .acceptance_for_task(task.id)
                        .iter()
                        .enumerate()
                        .map(|(index, _)| crate::assessment::Criterion {
                            criterion: index as u64 + 1,
                            met: false,
                            note: "Owner asked for a change before acceptance".into(),
                        })
                        .collect();
                    let decision = crate::assessment::Decision {
                        failure: None,
                        risk: None,
                        outcome: crate::assessment::Outcome::NeedsRepair,
                        reason: format!("{OWNER}{}", item.note),
                        criteria,
                        limitations: vec![],
                    };
                    let json = serde_json::to_string(&decision).unwrap_or_default();
                    Step::Command(
                        format!("create-review-{}", task.id),
                        format!("/review {} {json}", task.id),
                    )
                }
                TaskStatus::Accepted => {
                    item.effect = Effect::FollowUp;
                    self.decide(item, snapshot, tasks, autopilot)
                }
                TaskStatus::Failed | TaskStatus::Rejected | TaskStatus::Cancelled => {
                    item.effect = Effect::Repair;
                    self.decide(item, snapshot, tasks, autopilot)
                }
                TaskStatus::NeedsHumanReview => Step::Fail(format!(
                    "#{} is held for human review; resolve it with /review {} JSON first",
                    task.id, task.id
                )),
                _ => Step::Fail(format!(
                    "#{} is {}",
                    task.id,
                    crate::dashboard::state_word(&task)
                )),
            },
            Effect::FollowUp => {
                let title = item.note.clone();
                Step::Command(
                    format!("create-follow-up-{}", task.id),
                    format!("/after {} {title}", item.root),
                )
            }
            Effect::Revise => unreachable!("handled above"),
        }
    }

    fn create_repair(
        &mut self,
        item: &mut Instruction,
        snapshot: &Snapshot,
        tasks: &TaskControl,
        task: &Task,
    ) -> Step {
        if task.status == TaskStatus::NeedsHumanReview {
            return Step::Fail(format!(
                "#{} is held for human review; resolve it with /review {} JSON first",
                task.id, task.id
            ));
        }
        // A repair proposed by autopilot for the same failure and not started
        // yet is cancelled first: the owner's note takes its place.
        if let Some(sibling) = snapshot.tasks.iter().find(|child| {
            child.repair_of == Some(task.id)
                && matches!(child.status, TaskStatus::Proposed | TaskStatus::Approved)
                && child.run.is_none()
                && !tasks.workers.contains_key(&child.id)
        }) {
            return Step::Command(
                format!("cancel-sibling-{}", sibling.id),
                format!("/cancel-task {}", sibling.id),
            );
        }
        let mut reason = format!("{OWNER}{}", item.note);
        if item.effect == Effect::AfterCheck {
            let failure = crate::autopilot::failure_detail(tasks, task);
            let room = 2000usize.saturating_sub(reason.len() + AFTER_CHECK.len());
            reason.push_str(AFTER_CHECK);
            reason.push_str(&crate::autopilot::clean(&failure, room));
        }
        Step::Command(
            format!("create-repair-{}", task.id),
            format!("/repair {} {reason}", task.id),
        )
    }

    fn launch(
        &mut self,
        item: &Instruction,
        snapshot: &Snapshot,
        tasks: &TaskControl,
        origin: &Task,
        child: u64,
    ) -> Step {
        let Some(task) = find(snapshot, child) else {
            return Step::Wait("waiting for the new task".into());
        };
        let kind = if item.effect == Effect::FollowUp {
            "follow-up"
        } else {
            "repair"
        };
        if tasks.workers.contains_key(&child) || task.run.is_some() {
            return Step::Done(format!("{kind} #{child} started"));
        }
        match task.status {
            TaskStatus::Proposed if task.policy.is_none() => {
                // A follow-up inherits exactly the origin's files and check.
                let Some(policy) = &origin.policy else {
                    return Step::Fail(format!("#{} has no file/check policy", origin.id));
                };
                let json = serde_json::to_string(policy).unwrap_or_default();
                Step::Command(format!("permit-{child}"), format!("/permit {child} {json}"))
            }
            TaskStatus::Proposed => {
                Step::Command(format!("approve-{child}"), format!("/approve {child}"))
            }
            TaskStatus::Approved => Step::Command(format!("run-{child}"), format!("/run {child}")),
            _ => Step::Fail(format!(
                "{kind} #{child} is {}",
                crate::dashboard::state_word(task)
            )),
        }
    }
}

fn autopilot_model(_autopilot: &Autopilot) -> String {
    "pending".into()
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    // Review JSON stays in F4 activity; the transcript keeps the verb.
    match line.find(" {") {
        Some(index) if line.starts_with("/review ") || line.starts_with("/permit ") => {
            line[..index].to_owned()
        }
        _ => line.to_owned(),
    }
}
