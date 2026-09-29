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
    tasks::{Snapshot, Task, TaskStatus, TaskStore},
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
}
fn is_zero(value: &u32) -> bool {
    *value == 0
}
impl Saved {
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

/// One command chosen by autopilot. The caller admits it exactly like typed input.
#[derive(Clone, Debug)]
pub struct Submission {
    pub text: String,
    pub intent: Intent,
}

pub fn is_command(text: &str) -> bool {
    let text = text.trim();
    let verb = text.split_whitespace().next().unwrap_or("");
    verb == "/go" || matches!(text, "/pause" | "/resume" | "/stop" | "/autopilot")
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Settled {
    Active,
    Success(u64),
    Stuck(String),
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

/// Read another mission's saved loop state without locks or writes.
/// `Ok(None)`: no loop was ever started. The word is the saved phase as
/// written; a loop left active by another process is reported as saved.
pub fn peek(path: &Path) -> Result<Option<&'static str>, String> {
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
    Ok(Some(match saved.phase {
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
    }))
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
    /// One-line result, handed once to the terminal footer when the loop finishes.
    finished_notice: Option<String>,
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
        })
    }

    /// The finished loop's one-line result, once; it replaces the stale start notice.
    pub fn take_finished_notice(&mut self) -> Option<String> {
        self.finished_notice.take()
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
            return Err("Usage: /go GOAL (1–4096 bytes without control characters)".into());
        }
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
        if tasks.planner.active() || tasks.planner.checkpoint().is_some() {
            return Err("A plan draft is open; /plan-save or /plan-cancel it before /go".into());
        }
        if let Some(scope) = &tasks.canonical_scope {
            scope_blocker(scope, goal)?;
        }
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
        self.saved = Some(Saved {
            version: VERSION,
            id,
            goal: goal.into(),
            model: model.into(),
            max_repairs,
            phase: Phase::Scoping,
            paused: false,
            started: now(),
            finished: None,
            plan_attempts: 0,
            plan_error: None,
            plan_errors: vec![],
            plan_request: None,
            save_request: None,
            first: None,
            count: 0,
            retry_cancelled: BTreeSet::new(),
            notice: String::new(),
            report: None,
            branch: None,
            adopted: BTreeSet::new(),
            round: 0,
        });
        self.last = None;
        self.last_key = None;
        self.attempts.clear();
        self.transient = 0;
        self.job = None;
        self.finished_notice = None;
        self.clock = (std::time::Instant::now(), 0);
        self.notice = format!(
            "Autopilot started · plan → approve → dispatch → review/repair (max {max_repairs} repairs per task) · F5 pauses"
        );
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
        self.pause(tasks);
        let workers: Vec<u64> = tasks.workers.keys().copied().collect();
        let mut errors = Vec::new();
        for task in &workers {
            if let Err(error) = tasks.command(runtime, &format!("/cancel-task {task}"), &model) {
                errors.push(format!("#{task}: {error}"));
            }
        }
        self.notice = if errors.is_empty() {
            format!(
                "Autopilot stopped · cancellation requested for {} worker(s) · /resume continues",
                workers.len()
            )
        } else {
            format!(
                "Autopilot stopped · cancellation issues: {}",
                errors.join("; ")
            )
        };
        self.notice.clone()
    }

    /// Terminal command entry for `/go`, `/pause`, `/resume`, `/stop`, `/autopilot`.
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
                        if !self.notice.is_empty() {
                            lines.push(String::new());
                            lines.push(format!("{:<LABEL$}{}", "Note", clean(&self.notice, 400)));
                        }
                        lines.join("\n")
                    }
                };
                tasks.set_visible(true);
                tasks.autopilot_report = Some(report);
                Ok("Autopilot status · /tasks returns to task details".into())
            }
            _ => match text.strip_prefix("/go") {
                Some(goal) if goal.starts_with(char::is_whitespace) && !goal.trim().is_empty() => {
                    self.start(goal, model, max_repairs, tasks)
                }
                _ => Err("Usage: /go GOAL".into()),
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
        let elapsed = self.elapsed();
        let Some(saved) = self.saved.as_mut().filter(|saved| saved.first.is_some()) else {
            return false;
        };
        if !saved.adopted.insert(task) {
            return false;
        }
        if saved.phase == Phase::Done {
            saved.phase = Phase::Running;
            saved.paused = false;
            saved.finished = None;
            saved.report = None;
            saved.round += 1;
            self.clock = (std::time::Instant::now(), elapsed);
            self.notice = "Autopilot resumed for your follow-up".into();
        }
        self.attempts.clear();
        self.persist();
        true
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
                    Settled::Active => {}
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
            elapsed: Duration::from_secs(self.elapsed()),
            branch: saved.branch.clone(),
        })
    }

    /// Choose at most one next command. Returns None while paused, waiting for an
    /// acknowledgment, or when there is nothing to do.
    pub fn tick(&mut self, runtime: &Runtime, tasks: &mut TaskControl) -> Option<Submission> {
        self.poll_integration(tasks);
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

    fn halt(&mut self, tasks: &mut TaskControl, notice: String) {
        if let Some(saved) = self.saved.as_mut() {
            saved.paused = true;
        }
        tasks.disable_dispatch();
        self.set_notice(tasks, notice);
        self.persist();
    }

    fn fail(&mut self, tasks: &mut TaskControl, notice: String) {
        let elapsed = self.elapsed();
        if let Some(saved) = self.saved.as_mut() {
            saved.phase = Phase::Failed;
            saved.finished = Some(saved.started + elapsed);
            saved.report = Some(format!(
                "✗ Autopilot failed\n{}\n\n{:<LABEL$}{}",
                clean(&saved.goal, 400),
                "Reason",
                clean(&notice, 1024)
            ));
        }
        self.finished_notice = Some(clean(&format!("Autopilot failed{GAP}{notice}"), 1024));
        self.set_notice(tasks, notice);
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
            if saved.plan_attempts >= PLAN_ATTEMPTS {
                self.fail(
                    tasks,
                    format!("Planning failed {PLAN_ATTEMPTS} times; autopilot stopped: {error}"),
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
        let saved = self.saved.as_ref().unwrap().clone();
        let families = families(&snapshot, &saved, tasks);
        let find = |id: u64| snapshot.tasks.iter().find(|task| task.id == id);
        let mut command = None;
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
                TaskStatus::Failed | TaskStatus::Cancelled => {
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
                            && task.run.is_none()
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
        runtime.spawn(async move {
            let result = integration_branch(store, workspace, baseline, sources, name.clone())
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
        // Labeled sections in the side pane language: dim labels, one blank
        // row between sections, one line per task.
        let mut lines = vec![
            format!("{} Autopilot {}", state.marker(), state.label()),
            clean(&saved.goal, 400),
            String::new(),
            format!("{:<LABEL$}{accepted}/{} accepted", "Tasks", families.len()),
            format!("{:<LABEL$}{repairs}", "Repairs"),
        ];
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
            };
            lines.push(format!("{glyph} #{}  {title}", family.root));
            if let Some(detail) = detail {
                lines.push(format!("      {detail}"));
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
        }
        self.set_notice(tasks, notice);
        tasks.set_visible(true);
        tasks.autopilot_report = Some(report);
        self.persist();
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
                    "failed; repair budget exhausted ({repairs}/{})",
                    saved.max_repairs
                )),
                TaskStatus::Cancelled if head_task.run.is_none() => {
                    Settled::Stuck("cancelled before start".into())
                }
                TaskStatus::Cancelled if !saved.retry_cancelled.contains(&head) => {
                    Settled::Stuck("cancelled; /resume retries it".into())
                }
                TaskStatus::Running if !tasks.workers.contains_key(&head) => {
                    Settled::Stuck(format!("run interrupted; inspect with /recover {head}"))
                }
                TaskStatus::Proposed | TaskStatus::Approved => match blocked() {
                    Some(reason) => Settled::Stuck(reason),
                    None if head_task.status == TaskStatus::Approved
                        && head_task.run.is_none()
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
    let commit = crate::dependencies::compose(&workspace, baseline, &candidates).await?;
    let reference = format!("refs/heads/{name}");
    match crate::worker::git(&workspace, &["show-ref", "--verify", "--hash", &reference]).await {
        Ok(current) if current.trim() == commit => {}
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
