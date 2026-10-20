//! Autopilot: a thin controller that chooses the next command a user could have
//! typed (`/plan`, `/plan-save`, `/approve`, `/dispatch on`, `/review`, `/repair`,
//! `/resolve-repair`). The terminal saves and dispatches each choice through the
//! normal console intent path, so task policy, evidence verification, locks and
//! receipts stay authoritative. Its own small state file lives beside the task
//! store; it never replays inference or effects on restart and restores paused.
use crate::{
    command_intent::Intent,
    dispatch::TRANSIENT_LIMIT,
    task_control::TaskControl,
    tasks::{Action, Snapshot, Task, TaskStatus, TaskStore},
    understanding,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{runtime::Runtime, sync::oneshot};

const VERSION: u32 = 1;
/// Label column of the finished report.
const LABEL: usize = 10;
/// Fields of one-line results are separated by three spaces.
const GAP: &str = "   ";
/// Repeated submissions of one decision without effect pause the loop.
const ATTEMPTS: u32 = 3;
const MAX_STATE: usize = 256 * 1024;
pub const DEFAULT_MAX_REPAIRS: u32 = 3;
pub const MAX_REPAIRS: u32 = 16;
/// Planning attempts per goal: the first plan plus two validation re-plans.
const PLAN_ATTEMPTS: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Phase {
    Scoping,
    Planning,
    Saving,
    Running,
    Finishing,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    id: String,
    goal: String,
    model: String,
    max_repairs: u32,
    phase: Phase,
    paused: bool,
    started: u64,
    #[serde(default)]
    finished: Option<u64>,
    #[serde(default)]
    plan_attempts: u32,
    /// Single re-plan error written by older builds; read-only.
    #[serde(default)]
    plan_error: Option<String>,
    /// Validation errors of every rejected planning attempt, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    plan_errors: Vec<String>,
    #[serde(default)]
    plan_request: Option<String>,
    #[serde(default)]
    save_request: Option<String>,
    #[serde(default)]
    first: Option<u64>,
    #[serde(default)]
    count: u64,
    /// Heads cancelled before a resume; only these may be repaired automatically.
    #[serde(default)]
    retry_cancelled: BTreeSet<u64>,
    #[serde(default)]
    notice: String,
    #[serde(default)]
    report: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    /// Owner follow-up tasks adopted as further families of this run.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    adopted: BTreeSet<u64>,
    /// Integrations already made; a reopened run integrates on `-N` branches.
    #[serde(default, skip_serializing_if = "is_zero")]
    round: u32,
    /// Follow-up work arrived while the integration branch was being built; the
    /// finishing job's result is discarded and the run integrates again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    stale: bool,
    /// Families settled as accepted and families in the run, kept current for
    /// read-only peeks from other missions. Absent in older state files.
    #[serde(default, skip_serializing_if = "is_zero")]
    done: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    total: u32,
    /// Uncommitted changes in the workspace when the run started, and the short
    /// HEAD workers were based on. Absent for clean trees and in older state files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dirty: Option<(u32, String)>,
    /// The `--on-finish` hook was started for this finish (at most once per
    /// finish; cleared when follow-up work reopens the run).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hook_fired: bool,
}
fn is_zero(value: &u32) -> bool {
    *value == 0
}
impl Saved {
    /// The start warning, also carried by the summary of the run.
    fn dirty_warning(&self) -> Option<String> {
        let (count, head) = self.dirty.as_ref().filter(|(count, _)| *count > 0)?;
        if *count == crate::worker::UNCHECKED_CHANGES {
            return Some(format!(
                "uncommitted changes not checked (repository filters or includes need qualification) · workers see HEAD {head}"
            ));
        }
        let (noun, verb) = if *count == 1 {
            ("change", "is")
        } else {
            ("changes", "are")
        };
        Some(format!(
            "{count} uncommitted {noun} {verb} not visible to workers (based on HEAD {head})"
        ))
    }
    fn roots(&self) -> Vec<u64> {
        let first = self.first.unwrap_or(0);
        let mut roots: Vec<u64> = (first..first + self.count).collect();
        roots.extend(self.adopted.iter().filter(|id| **id >= first + self.count));
        roots
    }
    fn active(&self) -> bool {
        !matches!(self.phase, Phase::Done | Phase::Failed)
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != VERSION
            || !text(&self.goal, 4096)
            || !text(&self.model, 200)
            || self.id.len() != 16
            || !self.id.bytes().all(|b| b.is_ascii_hexdigit())
            || self.max_repairs > MAX_REPAIRS
        {
            return Err("Unsupported or invalid autopilot state".into());
        }
        Ok(())
    }
}

fn text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
/// Remove controls and bound bytes on a character boundary.
pub(crate) fn clean(value: &str, limit: usize) -> String {
    let mut result: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if result.len() > limit {
        let mut end = limit;
        while !result.is_char_boundary(end) {
            end -= 1;
        }
        result.truncate(end);
    }
    result.trim().to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Planning,
    Running,
    Paused,
    Finishing,
    /// Finished with every planned task accepted.
    Done,
    /// Finished with some planned tasks accepted and others failed or held.
    Partial,
    /// Finished with no planned task accepted (or stopped before any ran).
    Failed,
}
impl RunState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Finishing => "integrating",
            Self::Done => "done",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
    pub fn marker(self) -> &'static str {
        match self {
            Self::Paused => "‖",
            Self::Done => "✓",
            Self::Partial => "◐",
            Self::Failed => "✗",
            _ => "▶",
        }
    }
    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Partial | Self::Failed)
    }
}

/// Finished state from original planned-task outcomes; repairs never count as tasks.
fn outcome(accepted: usize, total: usize) -> RunState {
    if total > 0 && accepted == total {
        RunState::Done
    } else if accepted > 0 {
        RunState::Partial
    } else {
        RunState::Failed
    }
}

fn plural(count: u32, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

/// Read-only projection for the UI. Counts derive from durable task receipts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: RunState,
    pub goal: String,
    pub done: usize,
    pub total: usize,
    pub failed: usize,
    pub repairs: u32,
    /// Repair budget per task (`--max-repairs`).
    pub max_repairs: u32,
    pub elapsed: Duration,
    pub branch: Option<String>,
}
impl Status {
    pub fn line(&self) -> String {
        let seconds = self.elapsed.as_secs();
        let clock = if seconds >= 3600 {
            format!(
                "{}:{:02}:{:02}",
                seconds / 3600,
                seconds / 60 % 60,
                seconds % 60
            )
        } else {
            format!("{:02}:{:02}", seconds / 60, seconds % 60)
        };
        let marker = self.state.marker();
        let mut line = format!(
            "Autopilot {marker} {} · {}/{} done · {} failed · repairs {} · {clock}",
            self.state.label(),
            self.done,
            self.total,
            self.failed,
            self.repairs
        );
        if let Some(branch) = &self.branch {
            line.push_str(&format!(" · {}", clean(branch, 80)));
        }
        line.push_str(&format!(" · {}", clean(&self.goal, 60)));
        line
    }
}

/// One planned task's outcome: `None` while still open (or cancelled by /stop).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskOutcome {
    pub id: u64,
    pub title: String,
    /// Latest task of the family: the original or its newest repair.
    pub head: u64,
    /// Status of the original planned task.
    pub status: TaskStatus,
    pub repairs: u32,
    pub result: Option<Result<(), String>>,
}

/// One command chosen by autopilot. The caller admits it exactly like typed input.
#[derive(Clone, Debug)]
pub struct Submission {
    pub text: String,
    pub intent: Intent,
}

pub fn is_command(text: &str) -> bool {
    let text = text.trim();
    let verb = text.split_whitespace().next().unwrap_or("");
    verb == "/go"
        || verb == "/clean"
        || matches!(text, "/pause" | "/resume" | "/stop" | "/autopilot")
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Settled {
    Active,
    Success(u64),
    Stuck(String),
    /// Cancelled by /stop or quit before /resume; neither a failure nor a repair.
    Cancelled,
}

struct Family {
    root: u64,
    head: u64,
    settled: Settled,
    repairs: u32,
}

/// The autopilot state file of one conversation set in a mission directory.
pub fn state_path(directory: &Path, conversation: &str) -> PathBuf {
    directory.join(format!(
        "autopilot-{:x}.json",
        Sha256::digest(conversation.as_bytes())
    ))
}

/// Another mission's saved loop state as read by `peek`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peek {
    /// The saved phase as written.
    pub word: &'static str,
    /// Accepted and total task families; absent before a plan or in state
    /// files written before counts were saved.
    pub counts: Option<(u32, u32)>,
}

/// Read another mission's saved loop state without locks or writes.
/// `Ok(None)`: no loop was ever started. A loop left active by another
/// process is reported as saved.
pub fn peek(path: &Path) -> Result<Option<Peek>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if bytes.len() > MAX_STATE {
        return Err("Autopilot state exceeds its size bound".into());
    }
    let saved: Saved = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    saved.validate()?;
    let counts = (saved.total > 0).then_some((saved.done, saved.total));
    let word = match saved.phase {
        // A finished loop's saved report names its outcome.
        Phase::Done => {
            // Older builds wrote `Autopilot partial: …`; newer ones `◐ Autopilot partial`.
            let first = saved
                .report
                .as_deref()
                .and_then(|report| report.lines().next())
                .unwrap_or_default()
                .trim_start_matches(['✓', '◐', '✗', ' ']);
            if first.starts_with("Autopilot partial") {
                "partial"
            } else if first.starts_with("Autopilot failed") {
                "failed"
            } else {
                "done"
            }
        }
        Phase::Failed => "failed",
        _ if saved.paused => "paused",
        Phase::Scoping | Phase::Planning | Phase::Saving => "planning",
        Phase::Running | Phase::Finishing => "running",
    };
    Ok(Some(Peek { word, counts }))
}

pub struct Autopilot {
    path: PathBuf,
    saved: Option<Saved>,
    last: Option<Intent>,
    /// Attempt key of `last`.
    last_key: Option<String>,
    attempts: BTreeMap<String, u32>,
    /// Consecutive submissions refused without effect because task state moved.
    transient: u32,
    job: Option<oneshot::Receiver<Result<String, String>>>,
    /// Monotonic process clock plus seconds already elapsed when it was taken.
    clock: (std::time::Instant, u64),
    notice: String,
    /// One-line result (or the restore notice), handed once to the terminal footer.
    finished_notice: Option<String>,
    /// The restore notice naming runs that Alfredo's exit interrupted was shown.
    announced_interrupted: bool,
    /// Heads sent to `/recover N requeue` whose outcome is not yet known (not persisted).
    recovering: BTreeSet<u64>,
    /// The user's `--on-finish` program, if any.
    on_finish: Option<PathBuf>,
    /// Failure notices of running hooks, polled by `take_finished_notice`.
    hook_results: Vec<std::sync::mpsc::Receiver<String>>,
}

impl Autopilot {
    /// Load this conversation set's autopilot. A saved active loop is always
    /// restored paused; nothing is replayed until an explicit resume.
    pub fn open(directory: &Path, conversation: &str) -> Result<Self, String> {
        let path = state_path(directory, conversation);
        let saved = match fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > MAX_STATE {
                    return Err("Autopilot state exceeds its size bound; file preserved".into());
                }
                let mut saved: Saved = serde_json::from_slice(&bytes).map_err(|error| {
                    format!("Autopilot state is invalid ({error}); file preserved")
                })?;
                saved.validate()?;
                if saved.active() {
                    saved.paused = true;
                    if saved.phase == Phase::Finishing {
                        saved.phase = Phase::Running;
                        saved.stale = false;
                    }
                }
                Some(saved)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("Cannot read autopilot state: {error}")),
        };
        let saved_elapsed = saved
            .as_ref()
            .map(|saved: &Saved| now().saturating_sub(saved.started));
        let notice = saved
            .as_ref()
            .filter(|saved| saved.active())
            .map(|_| "Autopilot restored paused · /resume or F5 continues".to_string())
            .unwrap_or_default();
        Ok(Self {
            path,
            saved,
            last: None,
            last_key: None,
            attempts: BTreeMap::new(),
            transient: 0,
            job: None,
            clock: (std::time::Instant::now(), saved_elapsed.unwrap_or_default()),
            notice,
            finished_notice: None,
            announced_interrupted: false,
            recovering: BTreeSet::new(),
            on_finish: crate::on_finish::configured(),
            hook_results: Vec::new(),
        })
    }

    /// Override the startup `--on-finish` program (tests).
    pub fn set_on_finish(&mut self, program: Option<PathBuf>) {
        self.on_finish = program;
    }

    /// Start the `--on-finish` hook on its own thread, once per finish. Called
    /// where the run first becomes done, partial or failed, with the summary
    /// already stored; a crash between that save and the spawn loses the call
    /// rather than repeating it. Never changes autopilot state.
    fn fire_hook(&mut self, state: RunState, accepted: usize, repairs: u32) {
        let (Some(program), Some(saved)) = (self.on_finish.clone(), self.saved.as_mut()) else {
            return;
        };
        if saved.hook_fired {
            return;
        }
        saved.hook_fired = true;
        let value = crate::on_finish::env_value;
        let env = vec![
            ("ALFREDO_STATE", state.label().to_string()),
            (
                "ALFREDO_BRANCH",
                value(saved.branch.as_deref().unwrap_or("")),
            ),
            ("ALFREDO_GOAL", value(&saved.goal)),
            ("ALFREDO_ACCEPTED", accepted.to_string()),
            ("ALFREDO_TOTAL", saved.roots().len().to_string()),
            ("ALFREDO_REPAIRS", repairs.to_string()),
        ];
        self.hook_results
            .push(crate::on_finish::spawn(program, env));
    }

    /// The finished loop's one-line result, once; it replaces the stale start
    /// notice. A failed `--on-finish` hook reports through the same footer slot.
    pub fn take_finished_notice(&mut self) -> Option<String> {
        if let Some(notice) = self.finished_notice.take() {
            return Some(notice);
        }
        let mut found = None;
        self.hook_results
            .retain(|receiver| match receiver.try_recv() {
                Ok(notice) => {
                    found.get_or_insert(notice);
                    false
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
            });
        found
    }

    /// Scripted runs: wait for every started `--on-finish` hook (each thread
    /// already enforces the 10 s limit, killing and reaping its child) and
    /// return one failure line per hook that failed; `grace` bounds the wait.
    pub fn finish_hooks(&mut self, grace: std::time::Duration) -> Vec<String> {
        let deadline = std::time::Instant::now() + grace;
        let mut failures = Vec::new();
        for receiver in self.hook_results.drain(..) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match receiver.recv_timeout(left) {
                Ok(notice) => failures.push(notice),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    failures.push("--on-finish hook did not finish in time".into())
                }
            }
        }
        failures
    }

    fn elapsed(&self) -> u64 {
        match self.saved.as_ref().and_then(|saved| {
            saved
                .finished
                .map(|finished| finished.saturating_sub(saved.started))
        }) {
            Some(fixed) => fixed,
            None => self.clock.1 + self.clock.0.elapsed().as_secs(),
        }
    }

    pub fn notice(&self) -> &str {
        &self.notice
    }
    pub fn report(&self) -> Option<&str> {
        self.saved.as_ref()?.report.as_deref()
    }
    /// True while the loop may still act; switching work must wait for pause.
    pub fn running(&self) -> bool {
        self.saved
            .as_ref()
            .is_some_and(|saved| saved.active() && !saved.paused)
    }

    fn persist(&mut self) {
        let Some(saved) = &self.saved else {
            return;
        };
        let result = (|| -> Result<(), String> {
            let bytes = serde_json::to_vec_pretty(saved).map_err(|e| e.to_string())?;
            let directory = self.path.parent().ok_or("Missing state directory")?;
            let temporary = directory.join(format!(".autopilot-{}.tmp", std::process::id()));
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
            fs::rename(&temporary, &self.path).map_err(|e| e.to_string())?;
            File::open(directory)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| e.to_string())
        })();
        if let Err(error) = result {
            if let Some(saved) = self.saved.as_mut() {
                saved.paused = saved.active();
            }
            self.notice = format!("Autopilot paused: state save failed: {error}");
        }
    }

    fn set_notice(&mut self, tasks: &mut TaskControl, notice: String) {
        tasks.notice = notice.clone();
        self.notice = notice.clone();
        if let Some(saved) = self.saved.as_mut() {
            saved.notice = notice;
        }
    }

    pub fn start(
        &mut self,
        goal: &str,
        model: &str,
        max_repairs: u32,
        tasks: &TaskControl,
    ) -> Result<String, String> {
        let goal = goal.trim();
        if !text(goal, 4096) {
            return Err("Usage: /go GOAL, or /go to run the open plan (GOAL is 1–4096 bytes without control characters)".into());
        }
        self.admit(model, max_repairs, tasks)?;
        if tasks.planner.active() || tasks.planner.checkpoint().is_some() {
            if self.stopped_draft(tasks) {
                return Err("The stopped autopilot's plan draft is open; /resume to continue it or /plan-cancel to discard".into());
            }
            return Err("A plan draft is open; /go with no goal runs it, or /plan-save or /plan-cancel it before /go GOAL".into());
        }
        if let Some(scope) = &tasks.canonical_scope {
            scope_blocker(scope, goal)?;
        }
        self.begin(goal, model, max_repairs, tasks, Phase::Scoping, None)
    }

    /// Bare `/go`: run a plan the user already reviewed instead of planning.
    /// An open draft is saved unchanged; otherwise the mission's saved, still
    /// proposed tasks run. Either way no planner request is made, and nothing
    /// changes when the plan is refused.
    pub fn start_plan(
        &mut self,
        model: &str,
        max_repairs: u32,
        tasks: &TaskControl,
    ) -> Result<String, String> {
        self.admit(model, max_repairs, tasks)?;
        if self.stopped_draft(tasks) {
            return Err("The stopped autopilot's plan draft is open; /resume to continue it or /plan-cancel to discard".into());
        }
        if tasks.planner.active() {
            return Err("A plan is still being generated; wait for the draft, then /go".into());
        }
        if let Some(draft) = tasks.planner.checkpoint() {
            if draft.plan.architecture.is_some() {
                return Err(
                    "The open draft is an Architect repair revision; /plan-save it instead of /go"
                        .into(),
                );
            }
            draft
                .validate()
                .map_err(|error| format!("The open plan draft is not valid: {error}"))?;
            lint_refusal(&draft.plan)?;
            let goal = plan_goal(&draft.plan)?;
            return self.begin(
                &goal,
                model,
                max_repairs,
                tasks,
                Phase::Saving,
                Some(vec![]),
            );
        }
        let snapshot = tasks
            .snapshot
            .as_ref()
            .ok_or("Tasks are not loaded yet; try /go again")?;
        let claimed: BTreeSet<u64> = self
            .saved
            .as_ref()
            .map(|saved| saved.roots().into_iter().collect())
            .unwrap_or_default();
        let mut ids: Vec<u64> = Vec::new();
        let mut goal = None;
        for receipt in &snapshot.receipts {
            let Action::Plan { plan } = &receipt.request.action else {
                continue;
            };
            if plan.architecture.is_some() {
                continue;
            }
            let proposed: Vec<u64> = (receipt.task..receipt.task + plan.tasks.len() as u64)
                .filter(|id| !claimed.contains(id) && !ids.contains(id))
                .filter(|id| {
                    snapshot.tasks.iter().any(|task| {
                        task.id == *id
                            && task.status == TaskStatus::Proposed
                            && task.policy.is_some()
                            && task.repair_of.is_none()
                    })
                })
                .collect();
            if proposed.is_empty() {
                continue;
            }
            lint_refusal(plan)?;
            goal = Some(plan_goal(plan)?);
            ids.extend(proposed);
        }
        let Some(goal) = goal else {
            return Err("Usage: /go GOAL, or /go to run the open plan; there is no open plan draft or saved proposed task to run".into());
        };
        ids.sort_unstable();
        for id in &ids {
            let Some(task) = snapshot.tasks.iter().find(|task| task.id == *id) else {
                continue;
            };
            if let Some(dep) = task
                .dependencies
                .iter()
                .find(|dep| !ids.contains(dep) && snapshot.dependency_source(**dep).is_err())
            {
                return Err(format!(
                    "Task #{id} depends on #{dep}, which is not accepted and not part of this run; nothing changed. Finish or accept #{dep} first, or /cancel-task #{id}"
                ));
            }
        }
        self.begin(&goal, model, max_repairs, tasks, Phase::Running, Some(ids))
    }

    /// Checks shared by every way of starting a run.
    fn admit(&self, model: &str, max_repairs: u32, tasks: &TaskControl) -> Result<(), String> {
        if !text(model, 200) {
            return Err("Autopilot needs a selected model".into());
        }
        if max_repairs > MAX_REPAIRS {
            return Err(format!("--max-repairs accepts 0 to {MAX_REPAIRS}"));
        }
        if let Some(saved) = &self.saved {
            if saved.active() && (!saved.paused || !tasks.workers.is_empty()) {
                return Err(format!(
                    "Autopilot is active for “{}”; /pause or /stop and let workers finish before a new /go",
                    clean(&saved.goal, 80)
                ));
            }
        }
        if let Some(blocker) = crate::doctor::sandbox_blocker() {
            return Err(blocker);
        }
        Ok(())
    }

    /// The open draft belongs to a stopped autopilot run.
    fn stopped_draft(&self, tasks: &TaskControl) -> bool {
        self.saved
            .as_ref()
            .filter(|saved| saved.active() && saved.paused)
            .is_some_and(|saved| {
                matches!(&saved.plan_request, Some(request) if tasks.planner.active() || draft_matches(tasks, request))
            })
    }

    /// Record a new run. `plan` is `None` for a goal still to be planned,
    /// `Some(empty)` for an open draft to save, and the saved task ids to run.
    fn begin(
        &mut self,
        goal: &str,
        model: &str,
        max_repairs: u32,
        tasks: &TaskControl,
        phase: Phase,
        plan: Option<Vec<u64>>,
    ) -> Result<String, String> {
        let id = format!(
            "{:x}",
            Sha256::digest(format!(
                "{goal}\0{}\0{:?}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
            ))
        )[..16]
            .to_string();
        let ids = plan.clone().unwrap_or_default();
        self.saved = Some(Saved {
            version: VERSION,
            id,
            goal: goal.into(),
            model: model.into(),
            max_repairs,
            phase,
            paused: false,
            started: now(),
            finished: None,
            plan_attempts: 0,
            plan_error: None,
            plan_errors: vec![],
            plan_request: None,
            save_request: None,
            first: ids.first().copied(),
            count: 0,
            retry_cancelled: BTreeSet::new(),
            notice: String::new(),
            report: None,
            branch: None,
            adopted: ids.iter().copied().collect(),
            round: 0,
            stale: false,
            done: 0,
            total: 0,
            dirty: crate::worker::uncommitted_changes(tasks.workspace()),
            hook_fired: false,
        });
        self.last = None;
        self.last_key = None;
        self.attempts.clear();
        self.transient = 0;
        self.job = None;
        self.finished_notice = None;
        self.clock = (std::time::Instant::now(), 0);
        let stages = match &plan {
            None => "plan → approve → dispatch → review/repair",
            Some(_) => "reviewed plan → approve → dispatch → review/repair",
        };
        self.notice = format!(
            "Autopilot started · {stages} (max {max_repairs} repairs per task) · F5 pauses"
        );
        if let Some(warning) = self.saved.as_ref().and_then(Saved::dirty_warning) {
            self.notice.push_str(&format!(" · {warning}"));
        }
        self.persist();
        Ok(self.notice.clone())
    }

    /// No new starts or automatic decisions; running workers finish normally.
    pub fn pause(&mut self, tasks: &mut TaskControl) -> String {
        let Some(saved) = self.saved.as_mut().filter(|saved| saved.active()) else {
            return "No active autopilot".into();
        };
        saved.paused = true;
        tasks.disable_dispatch();
        self.persist();
        self.notice = "Autopilot paused · running workers finish · /resume or F5 continues".into();
        self.notice.clone()
    }

    /// A typed `/dispatch off` is the user taking control: pause rather than
    /// re-enable dispatch behind their back. Other manual commands pass through.
    pub fn observe_manual(&mut self, text: &str, tasks: &mut TaskControl) {
        if text.trim() == "/dispatch off" && self.running() {
            self.pause(tasks);
            self.notice = "Autopilot paused by /dispatch off · /resume continues".into();
        }
    }

    pub fn resume(&mut self, tasks: &TaskControl) -> Result<String, String> {
        let saved = self
            .saved
            .as_mut()
            .ok_or("No autopilot goal; use /go GOAL")?;
        if !saved.active() {
            return Err("Autopilot already finished; /go GOAL starts a new loop".into());
        }
        saved.paused = false;
        if let Some(snapshot) = &tasks.snapshot {
            // Runs cancelled by /stop or quit may be repaired after this resume.
            for family in families(snapshot, saved, tasks) {
                if let Some(task) = snapshot.tasks.iter().find(|t| t.id == family.head) {
                    if task.status == TaskStatus::Cancelled && task.run.is_some() {
                        saved.retry_cancelled.insert(task.id);
                    }
                }
            }
        }
        if saved.phase == Phase::Planning
            && !tasks.planner.active()
            && !matches!(&saved.plan_request, Some(request) if draft_matches(tasks, request))
        {
            // Interrupted generation restarts once explicitly resumed.
            saved.plan_request = None;
        }
        self.recovering.clear();
        self.attempts.clear();
        self.last = None;
        self.last_key = None;
        self.transient = 0;
        self.persist();
        self.notice = "Autopilot resumed".into();
        Ok(self.notice.clone())
    }

    pub fn toggle(&mut self, tasks: &mut TaskControl) -> Result<String, String> {
        if self.running() {
            Ok(self.pause(tasks))
        } else {
            self.resume(tasks)
        }
    }

    /// Pause, then cancel running workers through the existing cancel command.
    pub fn stop(&mut self, runtime: &Runtime, tasks: &mut TaskControl) -> String {
        let model = self
            .saved
            .as_ref()
            .map(|saved| saved.model.clone())
            .unwrap_or_else(|| "pending".into());
        let was_active = self.saved.as_ref().is_some_and(|saved| saved.active());
        self.pause(tasks);
        let planning_cancelled = self.cancel_planning(tasks);
        let workers: Vec<u64> = tasks.workers.keys().copied().collect();
        let mut errors = Vec::new();
        for task in &workers {
            if let Err(error) = tasks.command(runtime, &format!("/cancel-task {task}"), &model) {
                errors.push(format!("#{task}: {error}"));
            }
        }
        // Only a live loop can be "stopped" or "continued" by /resume.
        let head = if was_active {
            "Autopilot stopped"
        } else {
            "No active autopilot"
        };
        self.notice = if errors.is_empty() && !was_active && workers.is_empty() {
            head.into()
        } else if errors.is_empty() {
            let tail = if was_active {
                " · /resume continues"
            } else {
                ""
            };
            format!(
                "{head} · cancellation requested for {} worker(s){tail}",
                workers.len()
            )
        } else {
            format!("{head} · cancellation issues: {}", errors.join("; "))
        };
        if planning_cancelled {
            self.notice
                .push_str(" · planning cancelled, no draft will appear");
        }
        self.notice.clone()
    }

    /// Abort the planner generation autopilot itself requested, so a late
    /// reply cannot surface as a draft. `/resume` plans again.
    fn cancel_planning(&mut self, tasks: &mut TaskControl) -> bool {
        let Some(saved) = self.saved.as_mut() else {
            return false;
        };
        // A landed draft means any generation now running is the user's revision.
        if saved.phase != Phase::Planning
            || saved.plan_request.is_none()
            || !tasks.planner.active()
            || tasks.planner.checkpoint().is_some()
        {
            return false;
        }
        tasks.planner.cancel();
        tasks.planner.visible = false;
        saved.plan_request = None;
        self.persist();
        true
    }

    /// Terminal command entry for `/go`, `/pause`, `/resume`, `/stop`, `/autopilot`, and the
    /// autopilot-gated `/clean`.
    pub fn command(
        &mut self,
        runtime: &Runtime,
        tasks: &mut TaskControl,
        text: &str,
        model: &str,
        max_repairs: u32,
    ) -> Result<String, String> {
        let text = text.trim();
        match text {
            "/pause" => Ok(self.pause(tasks)),
            "/resume" => self.resume(tasks),
            "/stop" => Ok(self.stop(runtime, tasks)),
            "/autopilot" => {
                let status = self
                    .status(tasks)
                    .ok_or("No autopilot goal; use /go GOAL")?;
                let report = match self.report() {
                    Some(summary) => summary.to_owned(),
                    None => {
                        let mut lines = vec![
                            format!(
                                "{} Autopilot {}",
                                status.state.marker(),
                                status.state.label()
                            ),
                            clean(&status.goal, 400),
                            String::new(),
                            format!(
                                "{:<LABEL$}{}/{} accepted",
                                "Tasks", status.done, status.total
                            ),
                            format!("{:<LABEL$}{}", "Repairs", status.repairs),
                            format!(
                                "{:<LABEL$}{}",
                                "Elapsed",
                                crate::dashboard::clock(status.elapsed)
                            ),
                        ];
                        if let Some(warning) = self.saved.as_ref().and_then(Saved::dirty_warning) {
                            lines.push(format!("{:<LABEL$}{warning}", "Warning"));
                        }
                        if !self.notice.is_empty() {
                            lines.push(String::new());
                            lines.push(format!("{:<LABEL$}{}", "Note", clean(&self.notice, 400)));
                        }
                        lines.join("\n")
                    }
                };
                tasks.set_visible(true);
                tasks.planner.visible = false;
                tasks.autopilot_report = Some(report);
                Ok("Autopilot status · /tasks returns to task details".into())
            }
            _ if text.split_whitespace().next() == Some("/clean") => crate::clean::command(
                runtime,
                tasks,
                self.running(),
                text.strip_prefix("/clean").unwrap_or(""),
            ),
            _ => match text.strip_prefix("/go") {
                Some(goal) if goal.starts_with(char::is_whitespace) && !goal.trim().is_empty() => {
                    self.start(goal, model, max_repairs, tasks)
                }
                Some("") => self.start_plan(model, max_repairs, tasks),
                _ => Err("Usage: /go GOAL, or /go to run the open plan".into()),
            },
        }
    }

    /// Planned and adopted task families of this run.
    pub fn roots(&self, _tasks: &TaskControl) -> BTreeSet<u64> {
        self.saved
            .as_ref()
            .filter(|saved| saved.first.is_some())
            .map(|saved| saved.roots().into_iter().collect())
            .unwrap_or_default()
    }

    /// Adopt an owner follow-up as a further family. A finished run reopens: the
    /// follow-up is reviewed like planned work and integrated on a new branch.
    pub fn adopt(&mut self, task: u64) -> bool {
        let Some(saved) = self.saved.as_mut().filter(|saved| saved.first.is_some()) else {
            return false;
        };
        if !saved.adopted.insert(task) {
            return false;
        }
        self.reopen();
        self.attempts.clear();
        self.persist();
        true
    }

    /// Owner-instructed work in one of this run's families after it finished:
    /// the run reopens to review it and integrates again on a new branch.
    pub fn reopen(&mut self) {
        let elapsed = self.elapsed();
        if let Some(saved) = self
            .saved
            .as_mut()
            .filter(|saved| saved.phase == Phase::Finishing && !saved.stale)
        {
            // The branch being built may already exist without this work; the
            // next integration takes a new name once the job is discarded.
            saved.stale = true;
            saved.round += 1;
            self.persist();
            return;
        }
        let Some(saved) = self
            .saved
            .as_mut()
            .filter(|saved| saved.phase == Phase::Done)
        else {
            return;
        };
        saved.phase = Phase::Running;
        saved.paused = false;
        saved.finished = None;
        saved.report = None;
        saved.hook_fired = false;
        if saved.branch.is_some() {
            saved.round += 1;
        }
        self.clock = (std::time::Instant::now(), elapsed);
        self.notice = "Autopilot resumed for your instruction".into();
        self.attempts.clear();
        self.persist();
    }

    /// An owner revision of the draft replaces the plan request autopilot waits for.
    pub fn adopt_revision(&mut self, correlation: &str) {
        let Some(saved) = self.saved.as_mut() else {
            return;
        };
        if matches!(saved.phase, Phase::Planning | Phase::Saving) && saved.save_request.is_none() {
            saved.phase = Phase::Planning;
            saved.plan_request = Some(correlation.to_owned());
            self.persist();
        }
    }

    pub fn status(&self, tasks: &TaskControl) -> Option<Status> {
        let saved = self.saved.as_ref()?;
        let mut state = match saved.phase {
            Phase::Done => RunState::Done,
            Phase::Failed => RunState::Failed,
            _ if saved.paused => RunState::Paused,
            Phase::Scoping | Phase::Planning | Phase::Saving => RunState::Planning,
            Phase::Running => RunState::Running,
            Phase::Finishing => RunState::Finishing,
        };
        let (mut done, mut failed, mut repairs) = (0, 0, 0);
        if let Some(snapshot) = &tasks.snapshot {
            for family in families(snapshot, saved, tasks) {
                repairs += family.repairs;
                match family.settled {
                    Settled::Success(_) => done += 1,
                    Settled::Stuck(_) => failed += 1,
                    Settled::Active | Settled::Cancelled => {}
                }
            }
        }
        let total = saved.roots().len();
        if state == RunState::Done {
            state = outcome(done, total);
        }
        Some(Status {
            state,
            goal: saved.goal.clone(),
            done,
            total,
            failed,
            repairs,
            max_repairs: saved.max_repairs,
            elapsed: Duration::from_secs(self.elapsed()),
            branch: saved.branch.clone(),
        })
    }

    /// Settled outcome of each planned task, for scripted (headless) reporting.
    pub fn outcomes(&self, tasks: &TaskControl) -> Vec<TaskOutcome> {
        let (Some(saved), Some(snapshot)) = (self.saved.as_ref(), tasks.snapshot.as_ref()) else {
            return vec![];
        };
        families(snapshot, saved, tasks)
            .into_iter()
            .map(|family| {
                let root = snapshot.tasks.iter().find(|task| task.id == family.root);
                TaskOutcome {
                    id: family.root,
                    title: root.map(|task| task.title.clone()).unwrap_or_default(),
                    status: root.map_or(TaskStatus::Proposed, |task| task.status.clone()),
                    head: family.head,
                    repairs: family.repairs,
                    result: match family.settled {
                        Settled::Success(_) => Some(Ok(())),
                        Settled::Stuck(reason) => Some(Err(reason)),
                        Settled::Active | Settled::Cancelled => None,
                    },
                }
            })
            .collect()
    }

    /// Keep the saved counts current so other missions can show them without
    /// reading this mission's tasks. Writes only when a count changed.
    fn record_progress(&mut self, tasks: &TaskControl) {
        if tasks.snapshot.is_none() || !self.saved.as_ref().is_some_and(Saved::active) {
            return;
        }
        let Some(status) = self.status(tasks) else {
            return;
        };
        let (done, total) = (status.done as u32, status.total as u32);
        let Some(saved) = self.saved.as_mut() else {
            return;
        };
        if (saved.done, saved.total) != (done, total) {
            saved.done = done;
            saved.total = total;
            self.persist();
        }
    }

    /// Choose at most one next command. Returns None while paused, waiting for an
    /// acknowledgment, or when there is nothing to do.
    pub fn tick(&mut self, runtime: &Runtime, tasks: &mut TaskControl) -> Option<Submission> {
        self.poll_integration(tasks);
        self.record_progress(tasks);
        self.announce_interrupted(tasks);
        if let Some(reason) = tasks.dispatch.contended.take() {
            if self.running() {
                self.halt(
                    tasks,
                    format!("Autopilot paused: {reason} · /resume retries"),
                );
                return None;
            }
        }
        let saved = self.saved.as_ref()?;
        if saved.paused || !saved.active() || saved.phase == Phase::Finishing {
            return None;
        }
        if tasks.pending
            || tasks.writing
            || self
                .last
                .as_ref()
                .is_some_and(|intent| tasks.intent_pending(intent))
        {
            return None;
        }
        tasks.snapshot.as_ref()?;
        let result = match saved.phase {
            Phase::Scoping => self.scoping(tasks),
            Phase::Planning => self.planning(tasks),
            Phase::Saving => self.saving(tasks),
            Phase::Running => self.running_step(runtime, tasks),
            _ => Ok(None),
        };
        match result {
            Ok(Some((key, text, intent))) => {
                // A refusal that wrote nothing (stale revision, busy store) is
                // neither an attempt nor progress; the refusal loaded current
                // state, so this decision was prepared again on it. Bounded.
                if self
                    .last
                    .as_ref()
                    .is_some_and(|last| tasks.intent_transient(last))
                {
                    if let Some(count) = self
                        .last_key
                        .as_ref()
                        .and_then(|last| self.attempts.get_mut(last))
                    {
                        *count = count.saturating_sub(1);
                    }
                    self.transient += 1;
                    if self.transient >= TRANSIENT_LIMIT {
                        let reason = self
                            .last
                            .as_ref()
                            .and_then(|intent| tasks.intent_error(intent))
                            .unwrap_or_else(|| tasks.notice.clone());
                        self.halt(
                            tasks,
                            format!(
                                "Autopilot paused: “{text}” refused {TRANSIENT_LIMIT} times; task state kept changing: {reason} · /resume retries"
                            ),
                        );
                        return None;
                    }
                } else {
                    self.transient = 0;
                }
                let attempts = self.attempts.entry(key.clone()).or_default();
                *attempts += 1;
                if *attempts > ATTEMPTS {
                    let reason = self
                        .last
                        .as_ref()
                        .and_then(|intent| tasks.intent_error(intent))
                        .unwrap_or_else(|| tasks.notice.clone());
                    self.halt(
                        tasks,
                        format!(
                            "Autopilot paused: “{text}” had no effect after {ATTEMPTS} attempts: {reason}"
                        ),
                    );
                    return None;
                }
                self.last = Some(intent.clone());
                self.last_key = Some(key);
                self.persist();
                Some(Submission {
                    text: format!("Autopilot · {text}"),
                    intent,
                })
            }
            Ok(None) => None,
            Err(error) => {
                self.halt(tasks, format!("Autopilot paused: {error}"));
                None
            }
        }
    }

    /// After a restore, say which recorded-running tasks lost their worker and what
    /// `/resume` does about it. Shown once; nothing is recovered until `/resume`.
    fn announce_interrupted(&mut self, tasks: &mut TaskControl) {
        if self.announced_interrupted
            || !self.saved.as_ref().is_some_and(|s| s.active() && s.paused)
        {
            return;
        }
        let Some(snapshot) = &tasks.snapshot else {
            return;
        };
        let ids: Vec<String> = snapshot
            .tasks
            .iter()
            .filter(|task| tasks.recoverable(task.id))
            .map(|task| format!("#{}", task.id))
            .collect();
        let unproven: Vec<String> = snapshot
            .tasks
            .iter()
            .filter(|task| tasks.interrupted(task.id) && !tasks.recoverable(task.id))
            .map(|task| format!("#{}", task.id))
            .collect();
        if ids.is_empty() && unproven.is_empty() {
            return;
        }
        self.announced_interrupted = true;
        let notice = if ids.is_empty() {
            format!(
                "Task {} was running when Alfredo exited and cannot be recovered safely; /tasks {0} or /recover {1} says why",
                unproven[0],
                unproven[0].trim_start_matches('#')
            )
        } else if ids.len() == 1 {
            format!(
                "Task {} was running when Alfredo exited; /resume re-runs it",
                ids[0]
            )
        } else {
            format!(
                "Tasks {} were running when Alfredo exited; /resume re-runs them",
                ids.join(", ")
            )
        };
        self.finished_notice = Some(notice.clone());
        self.set_notice(tasks, notice);
    }

    /// A head sent to `/recover N requeue` is re-run by this resume only once recovery
    /// recorded it cancelled. A saved result or a refusal leaves `retry_cancelled`
    /// alone, so a later `/stop` of that task is not retried automatically.
    fn settle_recovered(&mut self, snapshot: &Snapshot) {
        if self.recovering.is_empty() {
            return;
        }
        let mut retry = Vec::new();
        self.recovering.retain(|id| {
            match snapshot
                .tasks
                .iter()
                .find(|t| t.id == *id)
                .map(|t| &t.status)
            {
                Some(TaskStatus::Running) => true,
                Some(TaskStatus::Cancelled) => {
                    retry.push(*id);
                    false
                }
                _ => false,
            }
        });
        if !retry.is_empty() {
            if let Some(saved) = self.saved.as_mut() {
                saved.retry_cancelled.extend(retry);
            }
            self.persist();
        }
    }

    fn halt(&mut self, tasks: &mut TaskControl, notice: String) {
        if let Some(saved) = self.saved.as_mut() {
            saved.paused = true;
        }
        tasks.disable_dispatch();
        self.set_notice(tasks, notice);
        self.persist();
    }

    fn fail(&mut self, tasks: &mut TaskControl, notice: String) {
        self.fail_with_next(tasks, notice, None);
    }

    fn fail_with_next(
        &mut self,
        tasks: &mut TaskControl,
        notice: String,
        next: Option<(String, u32)>,
    ) {
        let elapsed = self.elapsed();
        let (accepted, repairs) = match (&tasks.snapshot, &self.saved) {
            (Some(snapshot), Some(saved)) => {
                let families = families(snapshot, saved, tasks);
                let accepted = families
                    .iter()
                    .filter(|family| matches!(family.settled, Settled::Success(_)))
                    .count();
                (accepted, families.iter().map(|family| family.repairs).sum())
            }
            _ => (0, 0),
        };
        let totals = tasks
            .snapshot
            .as_ref()
            .zip(self.saved.as_ref())
            .and_then(|(snapshot, saved)| run_usage(tasks, snapshot, saved).line());
        if let Some(saved) = self.saved.as_mut() {
            saved.phase = Phase::Failed;
            saved.finished = Some(saved.started + elapsed);
            let mut report = format!(
                "✗ Autopilot failed\n{}\n\n{:<LABEL$}{}",
                clean(&saved.goal, 400),
                "Reason",
                clean(&notice, 1024)
            );
            if let Some((next, repairs)) = &next {
                report.push_str(&format!("\n{:<LABEL$}{repairs}", "Repairs"));
                if !next.is_empty() {
                    report.push_str(&format!("\n{:<LABEL$}{}", "Next", clean(next, 300)));
                }
            }
            if let Some(warning) = saved.dirty_warning() {
                report.push_str(&format!("\n{:<LABEL$}{warning}", "Warning"));
            }
            if let Some(totals) = &totals {
                report.push_str(&format!("\n{:<LABEL$}{totals}", "Model"));
            }
            saved.report = Some(report);
        }
        self.fire_hook(RunState::Failed, accepted, repairs);
        let notice = match next.as_ref() {
            Some((next, _)) if !next.is_empty() => format!("{notice} {next}"),
            _ => notice,
        };
        let repairs = next
            .as_ref()
            .map(|(_, repairs)| format!("{GAP}{}", plural(*repairs, "repair")))
            .unwrap_or_default();
        let notice = format!("{notice}{repairs}");
        let totals = totals
            .map(|line| format!("{GAP}{line}"))
            .unwrap_or_default();
        self.finished_notice = Some(clean(
            &format!("Autopilot failed{GAP}{notice}{totals}"),
            1024,
        ));
        self.set_notice(tasks, notice);
        // A planning failure leaves the unsaved draft panel open; the summary
        // must be what the user sees, as after a task failure.
        tasks.planner.visible = false;
        tasks.autopilot_report = self.report().map(str::to_owned);
        self.persist();
    }

    fn advance(&mut self, phase: Phase) {
        if let Some(saved) = self.saved.as_mut() {
            saved.phase = phase;
        }
        self.attempts.clear();
        self.persist();
    }

    fn scoping(
        &mut self,
        tasks: &mut TaskControl,
    ) -> Result<Option<(String, String, Intent)>, String> {
        let Some(scope) = tasks.canonical_scope.clone() else {
            return Ok(None);
        };
        let saved = self.saved.as_ref().unwrap();
        if let Err(error) = scope_blocker(&scope, &saved.goal) {
            self.fail(tasks, error);
            return Ok(None);
        }
        let brief = goal_brief(&saved.goal);
        let needs_scope = if scope.confirmed {
            false
        } else if scope.brief.is_none() {
            crate::wayfinder::entry_mode(&saved.goal).is_some()
        } else {
            true
        };
        if !needs_scope {
            self.advance(Phase::Planning);
            return Ok(None);
        }
        let (label, action) = if scope.brief.as_ref() == Some(&brief) && !scope.confirmed {
            (
                format!("/scope-confirm {}", scope.draft_revision),
                understanding::Action::Confirm {
                    draft_revision: scope.draft_revision,
                },
            )
        } else {
            (
                "/scope (goal-derived minimal scope)".to_string(),
                understanding::Action::Draft { brief },
            )
        };
        let kind = if matches!(action, understanding::Action::Draft { .. }) {
            "draft"
        } else {
            "confirm"
        };
        let intent = Intent::Scope {
            request: understanding::Request {
                correlation: format!("autopilot-{}-scope-{kind}-{}", saved.id, scope.revision),
                expected_revision: scope.revision,
                action,
            },
        };
        intent.validate()?;
        Ok(Some((format!("scope-{kind}"), label, intent)))
    }

    fn planning(
        &mut self,
        tasks: &mut TaskControl,
    ) -> Result<Option<(String, String, Intent)>, String> {
        if tasks.planner.active() || tasks.owner.planning_held() {
            return Ok(None);
        }
        let saved = self.saved.as_ref().unwrap().clone();
        if let Some(request) = &saved.plan_request {
            let error = if draft_matches(tasks, request) {
                match tasks.planner.draft.as_ref().map(unattended_lint) {
                    Some(Err(error)) => {
                        // Same effect as /plan-cancel: discard the unsaved draft.
                        tasks.planner.cancel();
                        tasks.planner.draft = None;
                        error
                    }
                    _ => {
                        self.advance(Phase::Saving);
                        return Ok(None);
                    }
                }
            } else {
                clean(
                    tasks
                        .planner
                        .notice
                        .trim_end_matches(" · no action taken")
                        .trim_end_matches(" · previous draft retained"),
                    1024,
                )
            };
            let error = if error.is_empty() {
                "Planner stopped without a draft".to_string()
            } else {
                error
            };
            let saved = self.saved.as_mut().unwrap();
            saved.plan_attempts += 1;
            saved.plan_request = None;
            if error.contains(crate::worker::NO_COMMITS)
                || error.contains(crate::provider::MISSING_MODEL_MARKER)
            {
                // Deterministic: retrying cannot help.
                self.fail(tasks, error);
            } else if saved.plan_attempts >= PLAN_ATTEMPTS {
                // `error` stays raw for retry feedback; only the report is plain.
                self.fail(
                    tasks,
                    format!(
                        "Planning failed {PLAN_ATTEMPTS} times; autopilot stopped: {}",
                        crate::planner::plain_error(&error)
                    ),
                );
            } else {
                saved.plan_errors.push(error);
                self.persist();
            }
            return Ok(None);
        }
        if tasks.planner.checkpoint().is_some() {
            self.fail(
                tasks,
                "Planning failed: another plan draft is open; /plan-save or /plan-cancel it".into(),
            );
            return Ok(None);
        }
        let mut prompt = clean(&saved.goal, 4096);
        let errors: Vec<String> = saved
            .plan_error
            .iter()
            .chain(&saved.plan_errors)
            .enumerate()
            .map(|(index, error)| format!("attempt {}: {}", index + 1, clean(error, 1024)))
            .collect();
        if !errors.is_empty() {
            prompt.push_str(&format!(
                " | Earlier plans were rejected by validation; fix every issue: {}. Return a corrected complete plan.",
                errors.join("; ")
            ));
        }
        let text = format!("/plan {prompt}");
        let intent = tasks
            .prepare_command(&text, &saved.model)?
            .ok_or("Planner command unavailable")?;
        self.saved.as_mut().unwrap().plan_request = Some(intent.correlation().to_string());
        Ok(Some((
            format!("plan-{}", saved.plan_attempts),
            text,
            intent,
        )))
    }

    fn saving(
        &mut self,
        tasks: &mut TaskControl,
    ) -> Result<Option<(String, String, Intent)>, String> {
        if tasks.planner.active() || tasks.owner.planning_held() {
            return Ok(None);
        }
        let snapshot = tasks.snapshot.as_ref().unwrap();
        let saved = self.saved.as_ref().unwrap();
        if let Some(request) = &saved.save_request {
            if let Some(receipt) = snapshot
                .receipts
                .iter()
                .find(|receipt| &receipt.request.correlation == request)
            {
                let crate::tasks::Action::Plan { plan } = &receipt.request.action else {
                    return Err("Plan save receipt has an unexpected action".into());
                };
                let (first, count) = (receipt.task, plan.tasks.len() as u64);
                let saved = self.saved.as_mut().unwrap();
                saved.first = Some(first);
                saved.count = count;
                self.advance(Phase::Running);
                return Ok(None);
            }
        }
        if tasks.planner.checkpoint().is_none() {
            self.fail(
                tasks,
                "Planning failed: the draft disappeared before it was saved".into(),
            );
            return Ok(None);
        }
        let intent = tasks
            .prepare_command("/plan-save", &saved.model)?
            .ok_or("Plan save unavailable")?;
        self.saved.as_mut().unwrap().save_request = Some(intent.correlation().to_string());
        Ok(Some(("plan-save".into(), "/plan-save".into(), intent)))
    }

    fn running_step(
        &mut self,
        runtime: &Runtime,
        tasks: &mut TaskControl,
    ) -> Result<Option<(String, String, Intent)>, String> {
        let snapshot = tasks.snapshot.clone().unwrap();
        self.settle_recovered(&snapshot);
        let saved = self.saved.as_ref().unwrap().clone();
        let families = families(&snapshot, &saved, tasks);
        let find = |id: u64| snapshot.tasks.iter().find(|task| task.id == id);
        // An environment failure cannot be repaired, and says so even when the
        // repair budget is zero or spent: stop with the root cause quoted.
        for family in families.iter().filter(|f| !held_or_done(f, tasks)) {
            if let Some(cause) = find(family.head).and_then(|head| sandbox_failure(tasks, head)) {
                self.fail(tasks, format!("{cause}. {}", crate::doctor::BWRAP_FIX));
                return Ok(None);
            }
            if let Some(cause) = find(family.head).and_then(|head| too_large_failure(tasks, head)) {
                // The last sentence of the shared message is the way forward.
                let (reason, next) = cause.rsplit_once(". ").unwrap_or((&cause, ""));
                let repairs: u32 = families.iter().map(|family| family.repairs).sum();
                self.fail_with_next(
                    tasks,
                    format!("{reason}."),
                    Some((next.to_string(), repairs)),
                );
                return Ok(None);
            }
        }
        let mut command = None;
        let mut retry_recovered: Vec<u64> = Vec::new();
        // Families the owner is instructing are theirs until the instructed run starts.
        let held = tasks.owner.held();
        for family in families
            .iter()
            .filter(|f| f.settled == Settled::Active && !held.contains(&f.root))
        {
            let Some(head) = find(family.head) else {
                continue;
            };
            command = match head.status {
                TaskStatus::Proposed if head.policy.is_some() => Some((
                    format!("approve-{}", head.id),
                    format!("/approve {}", head.id),
                )),
                TaskStatus::ReviewReady => {
                    let criteria: Vec<_> = snapshot
                        .acceptance_for_task(head.id)
                        .iter()
                        .enumerate()
                        .map(|(index, _)| {
                            serde_json::json!({"criterion": index + 1, "met": true,
                                "note": "Autopilot: approved check passed with verified evidence; criterion not independently inspected"})
                        })
                        .collect();
                    let review = serde_json::json!({"outcome": "approved", "reason": "autopilot: check passed", "criteria": criteria});
                    Some((
                        format!("review-{}", head.id),
                        format!("/review {} {review}", head.id),
                    ))
                }
                TaskStatus::Accepted if head.repair_of.is_some() => Some((
                    format!("resolve-{}", head.id),
                    format!("/resolve-repair {}", head.id),
                )),
                // Alfredo exited mid-run: record the interruption with `/recover`'s proof
                // as a cancelled run (never a failure), then `/requeue` below. Nothing
                // is replayed and no repair is spent.
                TaskStatus::Running if tasks.recoverable(head.id) => {
                    retry_recovered.push(head.id);
                    Some((
                        format!("recover-{}", head.id),
                        format!("/recover {} requeue", head.id),
                    ))
                }
                // A run cancelled by the user is dispatched again as the same
                // task: no repair, no failure and no repair budget spent.
                TaskStatus::Cancelled => Some((
                    format!("requeue-{}", head.id),
                    format!("/requeue {}", head.id),
                )),
                TaskStatus::Failed => {
                    let reason =
                        clean(&format!("autopilot: {}", failure_detail(tasks, head)), 1800);
                    Some((
                        format!("repair-{}", head.id),
                        format!("/repair {} {reason}", head.id),
                    ))
                }
                _ => None,
            };
            if command.is_some() {
                break;
            }
        }
        if command.is_none() && !tasks.dispatch.enabled {
            let ready = families.iter().any(|family| {
                family.settled == Settled::Active
                    && find(family.head).is_some_and(|task| {
                        task.status == TaskStatus::Approved
                            && !tasks.workers.contains_key(&task.id)
                            && task
                                .dependencies
                                .iter()
                                .all(|dep| snapshot.dependency_source(*dep).is_ok())
                    })
            });
            if ready {
                command = Some((
                    format!("dispatch-{}", snapshot.revision),
                    "/dispatch on".into(),
                ));
            }
        }
        if let Some((key, text)) = command {
            let intent = tasks
                .prepare_command(&text, &saved.model)?
                .ok_or("Command unavailable")?;
            self.recovering.extend(retry_recovered);
            let display = if text.starts_with("/review ") {
                format!(
                    "{} (approved: check passed)",
                    &text[..text.find(" {").unwrap_or(text.len())]
                )
            } else {
                text
            };
            return Ok(Some((key, display, intent)));
        }
        if tasks.workers.is_empty()
            && families
                .iter()
                .all(|family| family.settled != Settled::Active)
        {
            self.integrate(runtime, tasks, &snapshot, &families);
        }
        Ok(None)
    }

    fn integrate(
        &mut self,
        runtime: &Runtime,
        tasks: &mut TaskControl,
        snapshot: &Snapshot,
        families: &[Family],
    ) {
        let saved = self.saved.as_ref().unwrap().clone();
        let sources: Vec<u64> = families
            .iter()
            .filter_map(|family| match family.settled {
                Settled::Success(source) => Some(source),
                _ => None,
            })
            .collect();
        tasks.disable_dispatch();
        if sources.is_empty() {
            self.complete(tasks, Err("no task was accepted".into()));
            return;
        }
        let first = saved.first.unwrap_or(1);
        let baseline = snapshot
            .plan_for_task(first)
            .and_then(|plan| plan.context.as_ref())
            .map(|context| context.baseline.clone())
            .or_else(|| {
                families
                    .iter()
                    .filter(|family| {
                        snapshot
                            .tasks
                            .iter()
                            .any(|t| t.id == family.root && t.dependencies.is_empty())
                    })
                    .find_map(|family| {
                        snapshot
                            .tasks
                            .iter()
                            .find(|t| t.id == family.root)
                            .and_then(|t| t.run.as_ref())
                            .map(|run| run.baseline.clone())
                    })
            });
        let Some(baseline) = baseline else {
            self.complete(tasks, Err("no recorded plan baseline".into()));
            return;
        };
        let name = match saved.round {
            0 => format!("alfredo/go-{}", &saved.id[..8]),
            round => format!("alfredo/go-{}-{}", &saved.id[..8], round + 1),
        };
        let (sender, receiver) = oneshot::channel();
        let store = tasks.store().clone();
        let workspace = snapshot.workspace.clone();
        let goal = saved.goal.clone();
        runtime.spawn(async move {
            let result =
                integration_branch(store, workspace, baseline, sources, name.clone(), goal)
                    .await
                    .map(|commit| format!("{name}\0{commit}"));
            let _ = sender.send(result);
        });
        self.job = Some(receiver);
        self.advance(Phase::Finishing);
        self.set_notice(
            tasks,
            "Autopilot composing accepted results on an integration branch".into(),
        );
    }

    fn poll_integration(&mut self, tasks: &mut TaskControl) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        let result = match job.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return,
            Err(oneshot::error::TryRecvError::Closed) => Err("integration stopped".into()),
        };
        self.job = None;
        if self.saved.as_ref().is_some_and(|saved| saved.stale) {
            // Follow-up work joined the run while the branch was built.
            if let Some(saved) = self.saved.as_mut() {
                saved.stale = false;
                saved.phase = Phase::Running;
            }
            self.attempts.clear();
            self.set_notice(tasks, "Autopilot resumed for your instruction".into());
            self.persist();
            return;
        }
        self.complete(tasks, result);
    }

    fn complete(&mut self, tasks: &mut TaskControl, result: Result<String, String>) {
        let Some(snapshot) = tasks.snapshot.clone() else {
            return;
        };
        let saved = self.saved.as_ref().unwrap().clone();
        let families = families(&snapshot, &saved, tasks);
        let accepted = families
            .iter()
            .filter(|family| matches!(family.settled, Settled::Success(_)))
            .count();
        let repairs: u32 = families.iter().map(|family| family.repairs).sum();
        let state = outcome(accepted, families.len());
        let totals = run_usage(tasks, &snapshot, &saved).line();
        // Labeled sections in the side pane language: dim labels, one blank
        // row between sections, one line per task.
        let mut lines = vec![
            format!("{} Autopilot {}", state.marker(), state.label()),
            clean(&saved.goal, 400),
            String::new(),
            format!("{:<LABEL$}{accepted}/{} accepted", "Tasks", families.len()),
            format!("{:<LABEL$}{repairs}", "Repairs"),
        ];
        if let Some(totals) = &totals {
            lines.push(format!("{:<LABEL$}{totals}", "Model"));
        }
        if let Some(warning) = saved.dirty_warning() {
            lines.push(format!("{:<LABEL$}{warning}", "Warning"));
        }
        let branch = match &result {
            Ok(value) => {
                let (name, _) = value.split_once('\0').unwrap_or((value, ""));
                lines.push(format!("{:<LABEL$}{name}", "Branch"));
                Some(name.to_string())
            }
            Err(error) => {
                lines.push(format!("{:<LABEL$}none", "Branch"));
                lines.push(format!(
                    "{:<LABEL$}{}",
                    "Reason",
                    if accepted > 0 {
                        format!("the accepted tasks do not compose cleanly: {error}")
                    } else {
                        error.clone()
                    }
                ));
                None
            }
        };
        lines.push(String::new());
        let mut stuck = 0;
        for family in &families {
            let title = snapshot
                .tasks
                .iter()
                .find(|t| t.id == family.root)
                .map(|t| clean(&t.title, 80))
                .unwrap_or_default();
            let (glyph, detail) = match &family.settled {
                Settled::Success(source) if *source == family.root => ("✓", None),
                Settled::Success(source) => ("✓", Some(format!("via repair #{source}"))),
                Settled::Stuck(reason) => {
                    stuck += 1;
                    ("✗", Some(reason.clone()))
                }
                Settled::Active => ("◌", Some("unfinished".into())),
                Settled::Cancelled => ("–", Some("cancelled; /resume runs it again".into())),
            };
            lines.push(format!("{glyph} #{}  {title}", family.root));
            if let Some(detail) = detail {
                let exhausted = detail.contains(BUDGET_EXHAUSTED);
                let interrupted = detail.starts_with(INTERRUPTED);
                lines.push(format!("      {detail}"));
                if interrupted {
                    lines.push(format!(
                        "      Next: /tasks #{0} for the retained run (do not rerun effects); /recover {0} says what it can prove, or start the work again with /go",
                        family.head
                    ));
                }
                if exhausted {
                    if let Some(head) = snapshot.tasks.iter().find(|t| t.id == family.head) {
                        lines.push(format!(
                            "      Last failure: {}",
                            last_failure_line(&failure_detail(tasks, head))
                        ));
                    }
                    lines.push(format!(
                        "      Next: Try a larger model, /repair {} <hint>, or raise --max-repairs",
                        family.head
                    ));
                }
            }
        }
        if let Some(name) = &branch {
            lines.push(String::new());
            lines.push(format!("{:<LABEL$}git switch {name}", "Review"));
            lines.push(format!("{:<LABEL$}git merge {name}", "Merge"));
            if stuck > 0 {
                lines.push(format!(
                    "{:<LABEL$}the branch holds the accepted tasks only",
                    "Note"
                ));
            }
        }
        let report = lines.join("\n");
        let mut notice = format!(
            "Autopilot {}{GAP}{accepted}/{} accepted",
            state.label(),
            families.len()
        );
        if repairs > 0 {
            notice.push_str(&format!("{GAP}{}", plural(repairs, "repair")));
        }
        if let Some(totals) = &totals {
            notice.push_str(&format!("{GAP}{totals}"));
        }
        notice.push_str(&match &branch {
            Some(name) => format!("{GAP}git switch {name}"),
            None => format!("{GAP}no integration branch"),
        });
        self.finished_notice = Some(notice.clone());
        let elapsed = self.elapsed();
        if let Some(saved) = self.saved.as_mut() {
            saved.phase = Phase::Done;
            saved.finished = Some(saved.started + elapsed);
            saved.report = Some(report.clone());
            saved.branch = branch;
            saved.done = accepted as u32;
            saved.total = saved.roots().len() as u32;
        }
        self.fire_hook(state, accepted, repairs);
        self.set_notice(tasks, notice);
        tasks.set_visible(true);
        tasks.planner.visible = false;
        tasks.autopilot_report = Some(report);
        self.persist();
    }
}

/// Planner, worker and repair usage of this run's own tasks: the saved plan's
/// receipt plus the model metrics in each task run's evidence. Anything the
/// server did not report adds nothing.
fn run_usage(tasks: &TaskControl, snapshot: &Snapshot, saved: &Saved) -> crate::metrics::Usage {
    let mut total = crate::metrics::Usage::default();
    let plan = saved.save_request.as_ref().and_then(|request| {
        snapshot
            .receipts
            .iter()
            .find(|receipt| &receipt.request.correlation == request)
    });
    if let Some(crate::tasks::Action::Plan { plan }) = plan.map(|receipt| &receipt.request.action) {
        if let Some(usage) = &plan.usage {
            total.add(usage);
        }
    }
    let Some(first) = saved.first else {
        return total;
    };
    let roots = saved.roots();
    for task in snapshot.tasks.iter().filter(|task| task.id >= first) {
        let in_run = snapshot
            .repair_root(task.id)
            .is_some_and(|root| roots.contains(&root));
        if !in_run || task.run.is_none() {
            continue;
        }
        let usage = tasks
            .store()
            .evidence(task.id)
            .ok()
            .and_then(|text| serde_json::from_str::<crate::worker::Evidence>(&text).ok())
            .and_then(|evidence| evidence.model_metrics)
            .and_then(|metrics| crate::metrics::Usage::from_metrics(&metrics));
        if let Some(usage) = usage {
            total.add(&usage);
        }
    }
    total
}

/// Root cause when the head failed because the sandbox could not start. The
/// run's evidence carries the typed `failure_code`; repairing cannot fix it.
fn sandbox_failure(tasks: &TaskControl, head: &Task) -> Option<String> {
    if head.status != TaskStatus::Failed {
        return None;
    }
    let evidence = tasks.store().evidence(head.id).ok()?;
    let evidence = serde_json::from_str::<crate::worker::Evidence>(&evidence).ok()?;
    (evidence.failure_code.as_deref() == Some(crate::execution::SANDBOX_UNAVAILABLE))
        .then(|| clean(&evidence.detail, 300))
}

/// Failure text when the head's approved files exceed the worker's bounds. The
/// run's evidence carries the typed `failure_code`; repairing cannot fix it.
fn too_large_failure(tasks: &TaskControl, head: &Task) -> Option<String> {
    if head.status != TaskStatus::Failed {
        return None;
    }
    let evidence = tasks.store().evidence(head.id).ok()?;
    let evidence = serde_json::from_str::<crate::worker::Evidence>(&evidence).ok()?;
    (evidence.failure_code.as_deref() == Some(crate::worker::FILE_TOO_LARGE))
        .then(|| clean(&evidence.detail, 300))
}

const INTERRUPTED: &str = "run interrupted";

/// Why an interrupted run cannot be recovered automatically.
fn interrupted_reason(tasks: &TaskControl, head: u64) -> String {
    match tasks.run_observations.get(&head) {
        Some(text) if text.contains("another process") => {
            format!("{INTERRUPTED}: its worker is still active in another Alfredo process")
        }
        Some(text) => format!(
            "{INTERRUPTED}: it cannot be recovered safely ({})",
            clean(text, 200)
        ),
        None => format!("{INTERRUPTED}: its worker state is unknown"),
    }
}

fn held_or_done(family: &Family, tasks: &TaskControl) -> bool {
    matches!(family.settled, Settled::Success(_)) || tasks.owner.held().contains(&family.root)
}

const BUDGET_EXHAUSTED: &str = "repair budget exhausted";

/// One line of a recorded failure for the summary: its first non-empty line,
/// shortened on a character boundary.
fn last_failure_line(detail: &str) -> String {
    let line = detail
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no detail recorded");
    let line = clean(line, 400);
    if line.chars().count() > 160 {
        let cut: String = line.chars().take(159).collect();
        format!("{cut}…")
    } else {
        line
    }
}

/// Repair reason detail: a failed check's bounded output tail from verified
/// evidence (naming a no-progress attempt), else the recorded run detail. Never empty.
pub(crate) fn failure_detail(tasks: &TaskControl, head: &Task) -> String {
    let failure = tasks
        .store()
        .evidence(head.id)
        .ok()
        .and_then(|raw| serde_json::from_str::<crate::worker::Evidence>(&raw).ok())
        .and_then(|evidence| crate::worker::check_failure(&evidence, 1780));
    if let Some(failure) = failure {
        return failure;
    }
    head.run
        .as_ref()
        .map(|run| clean(&run.detail, 1780))
        .filter(|detail| !detail.trim_end_matches(':').trim().is_empty())
        .unwrap_or_else(|| "worker failed; no detail recorded".into())
}

/// Autopilot approves without a human reading each policy, so it also refuses
/// plans a reader would reject: checks that can only fail and unordered writers.
fn unattended_lint(plan: &crate::planner::Plan) -> Result<(), String> {
    let findings = crate::plan_lint::findings(plan);
    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings.join("; "))
    }
}

/// Reviewing a plan is the approval of its files and checks, but autopilot
/// still refuses what it would refuse from the planner.
fn lint_refusal(plan: &crate::planner::Plan) -> Result<(), String> {
    unattended_lint(plan).map_err(|error| {
        format!(
            "The plan cannot run unattended and nothing changed: {}. Fix it with /plan-revise (draft) or run it by hand with /approve",
            clean(&error, 600)
        )
    })
}

/// The goal autopilot shows for a plan it adopts: the `/plan` prompt.
fn plan_goal(plan: &crate::planner::Plan) -> Result<String, String> {
    let goal = clean(plan.prompt.trim(), 4096);
    if text(&goal, 4096) {
        Ok(goal)
    } else {
        Err("The plan has no usable prompt to show as the goal".into())
    }
}

fn draft_matches(tasks: &TaskControl, request: &str) -> bool {
    tasks.planner.checkpoint().is_some_and(|draft| {
        draft
            .origin
            .as_ref()
            .is_some_and(|origin| origin.correlation == request)
            && draft.validate().is_ok()
    })
}

fn goal_brief(goal: &str) -> understanding::Brief {
    understanding::Brief {
        destination: format!("Autopilot goal: {}", clean(goal, 1900)),
        scope: "Only the tasks autopilot plans for this goal, each limited to its approved files and check.".into(),
        constraints: "Local isolated worktrees and approved file/check policies; no push; the checked-out branch, index and working files stay untouched.".into(),
        uncertainty: "Plan and code are model-generated; autopilot acceptance relies on the approved check passing, not independent inspection.".into(),
    }
}

/// A user-authored pending draft is never confirmed on the user's behalf. Only
/// an untouched Wayfinder entry placeholder or autopilot's own draft may proceed.
fn scope_blocker(scope: &understanding::Snapshot, goal: &str) -> Result<(), String> {
    if scope.brief.is_none() || scope.confirmed {
        return Ok(());
    }
    let placeholder = scope.flow.is_some() && scope.draft_revision == 1;
    if placeholder || scope.brief.as_ref() == Some(&goal_brief(goal)) {
        return Ok(());
    }
    Err(format!(
        "Scope draft {} awaits your review; inspect /scope and /scope-confirm {} (or revise it) before /go",
        scope.draft_revision, scope.draft_revision
    ))
}

/// Settle each planned task family from durable receipts plus local worker state.
fn families(snapshot: &Snapshot, saved: &Saved, tasks: &TaskControl) -> Vec<Family> {
    let Some(first) = saved.first else {
        return vec![];
    };
    let find = |id: u64| snapshot.tasks.iter().find(|task| task.id == id);
    let roots: BTreeMap<u64, u64> = snapshot
        .tasks
        .iter()
        .filter(|task| task.id >= first)
        .filter_map(|task| Some((task.id, snapshot.repair_root(task.id)?)))
        .collect();
    let held = tasks.owner.held();
    let mut result: Vec<Family> = Vec::new();
    for root in saved.roots() {
        let Some(task) = find(root) else {
            continue;
        };
        let members: Vec<&Task> = snapshot
            .tasks
            .iter()
            .filter(|t| roots.get(&t.id) == Some(&root))
            .collect();
        let head = members.iter().map(|t| t.id).max().unwrap_or(root);
        // A repair cancelled before it started (replaced by an owner
        // instruction) spent nothing; a steer's parent was cancelled, not failed.
        let repairs = members
            .iter()
            .filter(|t| !(t.status == TaskStatus::Cancelled && t.run.is_none()))
            .filter(|t| {
                t.repair_of
                    .and_then(find)
                    .is_some_and(|parent| parent.status == TaskStatus::Failed)
            })
            .count() as u32;
        let head_task = find(head).unwrap_or(task);
        let blocked = || {
            head_task.dependencies.iter().find_map(|dep| {
                let dependency = result
                    .iter()
                    .find(|family| family.root == snapshot.repair_root(*dep).unwrap_or(*dep))?;
                matches!(dependency.settled, Settled::Stuck(_))
                    .then(|| format!("blocked by #{dep}"))
            })
        };
        let settled = if held.contains(&root) {
            Settled::Active
        } else if let Some(source) = snapshot.resolution_for_family(root) {
            Settled::Success(source)
        } else if task.status == TaskStatus::Accepted {
            Settled::Success(root)
        } else {
            match head_task.status {
                TaskStatus::NeedsHumanReview => Settled::Stuck("held for human review".into()),
                TaskStatus::Rejected => Settled::Stuck("rejected by review".into()),
                TaskStatus::Failed if repairs >= saved.max_repairs => Settled::Stuck(format!(
                    "failed; {BUDGET_EXHAUSTED} ({repairs}/{})",
                    saved.max_repairs
                )),
                TaskStatus::Cancelled if head_task.run.is_none() => {
                    Settled::Stuck("cancelled before start".into())
                }
                TaskStatus::Cancelled if !saved.retry_cancelled.contains(&head) => {
                    Settled::Cancelled
                }
                // Alfredo exited mid-run: `/resume` recovers it with the `/recover` proof
                // and runs it again, so it is neither failed nor stuck while provable.
                TaskStatus::Running if tasks.recoverable(head) => Settled::Active,
                TaskStatus::Running if !tasks.workers.contains_key(&head) => {
                    Settled::Stuck(interrupted_reason(tasks, head))
                }
                TaskStatus::Proposed | TaskStatus::Approved => match blocked() {
                    Some(reason) => Settled::Stuck(reason),
                    None if head_task.status == TaskStatus::Approved
                        && !tasks.workers.contains_key(&head)
                        && crate::dispatch::approval(snapshot, head).is_some_and(|approval| {
                            tasks.dispatch.attempts.get(&head) == Some(&approval)
                        }) =>
                    {
                        let reason = tasks
                            .dispatch
                            .failures
                            .get(&head)
                            .map(|failure| clean(failure, 200))
                            .unwrap_or_else(|| "launch did not start".into());
                        Settled::Stuck(format!("launch failed: {reason}"))
                    }
                    None => Settled::Active,
                },
                _ => Settled::Active,
            }
        };
        result.push(Family {
            root,
            head,
            settled,
            repairs,
        });
    }
    result
}

/// Compose accepted candidates on the recorded baseline with the same object-only
/// merge used for dependency baselines, then create one new local branch ref.
async fn integration_branch(
    store: TaskStore,
    workspace: PathBuf,
    baseline: String,
    sources: Vec<u64>,
    name: String,
    goal: String,
) -> Result<String, String> {
    let mut candidates = Vec::new();
    for source in sources {
        let reader = store.clone();
        let raw = tokio::task::spawn_blocking(move || reader.evidence(source))
            .await
            .map_err(|_| "Evidence reader stopped".to_string())??;
        let evidence: crate::worker::Evidence =
            serde_json::from_str(&raw).map_err(|_| "Malformed accepted evidence")?;
        candidates.push((
            source,
            crate::worker::verify_candidate(&workspace, &evidence).await?,
        ));
    }
    let composed = crate::dependencies::compose(&workspace, baseline, &candidates).await?;
    // The tip names the goal; its tree is the composed tree, so the content is unchanged.
    let subject = crate::worker::commit_line(&goal, 72);
    let subject = if subject.is_empty() {
        "Alfredo autopilot".to_string()
    } else {
        subject
    };
    let message = format!(
        "{subject}\n\nAlfredo autopilot integration of {} accepted task{}",
        candidates.len(),
        if candidates.len() == 1 { "" } else { "s" }
    );
    let commit = crate::worker::git(
        &workspace,
        &[
            "-c",
            "user.name=Alfredo",
            "-c",
            "user.email=alfredo@localhost",
            "-c",
            "commit.gpgSign=false",
            "commit-tree",
            &format!("{composed}^{{tree}}"),
            "-p",
            &composed,
            "-m",
            &message,
        ],
    )
    .await?
    .trim()
    .to_string();
    let reference = format!("refs/heads/{name}");
    match crate::worker::git(&workspace, &["show-ref", "--verify", "--hash", &reference]).await {
        Ok(current) if current.trim() == commit => {}
        // A retry after a restart rebuilds the tip with a new timestamp. Accept a ref
        // sitting on the same composed tip (goal commit on top) or at it (a branch
        // made before the goal commit existed).
        Ok(current)
            if current.trim() == composed
                || crate::worker::git(
                    &workspace,
                    &["rev-parse", &format!("{}^", current.trim())],
                )
                .await
                .is_ok_and(|parent| parent.trim() == composed) =>
        {
            return Ok(current.trim().to_string());
        }
        Ok(_) => {
            return Err(format!(
                "{name} already exists and points elsewhere; it was not changed"
            ))
        }
        Err(_) => {
            crate::worker::git(
                &workspace,
                &[
                    "update-ref",
                    "--no-deref",
                    &reference,
                    &commit,
                    "0000000000000000000000000000000000000000",
                ],
            )
            .await?;
        }
    }
    Ok(commit)
}
