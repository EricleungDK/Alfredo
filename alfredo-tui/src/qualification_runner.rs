//! Isolated diagnostic fixtures. Their deterministic review oracle is never used
//! for production work, and only canonical reviewed outcomes count as accepted.
use crate::{
    assessment::{Criterion, Decision, Outcome},
    model::{Message, Update},
    planner::Planner,
    provider::Ollama,
    tasks::{Action, Receipt, Request, TaskStatus, TaskStore, WorkPolicy},
    worker::{self, Evidence, Observer},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{runtime::Runtime, sync::mpsc};

const SCENARIO_LIMIT: Duration = Duration::from_secs(240);
const CHECK_OK: &str = "QUALIFICATION_CHECK_OK";
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scenario {
    SmallEdit,
    RequiredSource,
    Repair,
    QueuedForeground,
}
impl Scenario {
    pub const ALL: [Self; 4] = [
        Self::SmallEdit,
        Self::RequiredSource,
        Self::Repair,
        Self::QueuedForeground,
    ];
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceManifest {
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureDefinition {
    pub scenario: Scenario,
    pub version: u32,
    pub digest: String,
    pub sources: Vec<SourceManifest>,
    pub writable_paths: Vec<String>,
    pub check: Vec<String>,
    pub goal_sha256: String,
    pub criteria_sha256: String,
    pub foreground_prompt_sha256: Option<String>,
}
impl FixtureDefinition {
    pub fn digest(&self) -> String {
        self.digest.clone()
    }
    pub fn validate(&self) -> Result<()> {
        if self != &fixture_definition(self.scenario) {
            return Err(
                "Qualification fixture definition does not match its pinned implementation".into(),
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptRef {
    pub revision: u64,
    pub task: u64,
    pub correlation: String,
    pub sha256: String,
}
impl ReceiptRef {
    fn from(receipt: &Receipt) -> Self {
        Self {
            revision: receipt.revision,
            task: receipt.task,
            correlation: receipt.request.correlation.clone(),
            sha256: hash(&serde_json::to_vec(receipt).expect("typed receipt")),
        }
    }
    fn valid(&self) -> bool {
        self.revision > 0
            && self.task > 0
            && bounded(&self.correlation, 160)
            && digest(&self.sha256)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRef {
    pub receipt: ReceiptRef,
    pub reviewed_task: u64,
    pub outcome: Outcome,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunResult {
    pub task: u64,
    pub run: String,
    pub repair_of: Option<u64>,
    pub status: TaskStatus,
    pub evidence_sha256: String,
    pub check_passed: bool,
    pub check_exit_code: Option<i32>,
    pub review: Option<ReviewRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForegroundResult {
    pub background_request: Option<usize>,
    pub queue_observed: bool,
    pub queue_ms: Option<u64>,
    pub completed: bool,
    pub check_passed: bool,
    pub response_sha256: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioOutcome {
    Accepted,
    Failed,
    Incomplete,
    ContextSelectionLimited,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioResult {
    pub scenario: Scenario,
    pub fixture_digest: String,
    pub artifact_directory: PathBuf,
    pub scope_revision: u64,
    pub plan_receipt: Option<ReceiptRef>,
    pub planner_sources: Vec<SourceManifest>,
    pub required_sources_present: bool,
    pub runs: Vec<RunResult>,
    pub foreground: Option<ForegroundResult>,
    pub generation_attempts: u32,
    pub outcome: ScenarioOutcome,
    pub elapsed_ms: u64,
    pub reviewed_elapsed_ms: Option<u64>,
    pub failure: Option<String>,
}
impl ScenarioResult {
    pub fn accepted(&self) -> bool {
        self.outcome == ScenarioOutcome::Accepted
    }
    pub fn reviewed_ms(&self) -> Option<u64> {
        self.reviewed_elapsed_ms
    }
    pub fn validate(&self) -> Result<()> {
        let definition = fixture_definition(self.scenario);
        let unique_sources: BTreeSet<_> = self.planner_sources.iter().map(|s| &s.path).collect();
        let unique_tasks: BTreeSet<_> = self.runs.iter().map(|r| r.task).collect();
        let unique_runs: BTreeSet<_> = self.runs.iter().map(|r| &r.run).collect();
        if self.fixture_digest != definition.digest
            || !self.artifact_directory.is_absolute()
            || self
                .artifact_directory
                .to_str()
                .is_none_or(|p| !bounded(p, 4096))
            || self.runs.len() > 2
            || self.generation_attempts > 4
            || self.elapsed_ms > 600_000
            || self.failure.as_ref().is_some_and(|s| !bounded(s, 2048))
            || self.plan_receipt.as_ref().is_some_and(|r| !r.valid())
            || self.planner_sources.len() > 8
            || unique_sources.len() != self.planner_sources.len()
            || unique_tasks.len() != self.runs.len()
            || unique_runs.len() != self.runs.len()
            || self.required_sources_present != required_present(&self.planner_sources, &definition)
            || self.runs.first().is_some_and(|r| {
                self.plan_receipt.as_ref().is_none_or(|p| p.task != r.task) || r.repair_of.is_some()
            })
            || self
                .planner_sources
                .iter()
                .any(|s| !bounded(&s.path, 512) || !digest(&s.sha256) || s.bytes > 8192)
            || self.runs.iter().any(|r| {
                r.task == 0
                    || !bounded(&r.run, 160)
                    || !digest(&r.evidence_sha256)
                    || (r.check_passed && r.check_exit_code != Some(0))
                    || r.review.as_ref().is_some_and(|review| {
                        !review.receipt.valid()
                            || review.reviewed_task != r.task
                            || (review.outcome != Outcome::NeedsRepair
                                && review.receipt.task != review.reviewed_task)
                    })
                    || (r.status == TaskStatus::Accepted
                        && (!r.check_passed
                            || r.review
                                .as_ref()
                                .is_none_or(|review| !review.outcome.approves())))
            })
            || self.foreground.as_ref().is_some_and(|f| {
                f.response_sha256.as_ref().is_some_and(|s| !digest(s))
                    || f.background_request.is_some_and(|n| n == 0 || n > 128)
                    || (f.queue_observed && f.background_request.is_none())
                    || f.queue_ms.is_some_and(|n| n > 600_000)
                    || (f.check_passed && (!f.completed || f.response_sha256.is_none()))
            })
        {
            return Err("Invalid or unbounded qualification scenario result".into());
        }
        if self.accepted() {
            if self.failure.is_some()
                || self.scope_revision != 2
                || self.plan_receipt.is_none()
                || !self.required_sources_present
                || self
                    .runs
                    .last()
                    .is_none_or(|r| r.status != TaskStatus::Accepted)
                || self.reviewed_elapsed_ms.is_none_or(|n| n > self.elapsed_ms)
                || (self.scenario != Scenario::Repair && self.runs.len() != 1)
                || (self.scenario == Scenario::Repair
                    && (self.runs.len() != 2
                        || self.runs[0].check_passed
                        || self.runs[0].check_exit_code.is_none_or(|code| code == 0)
                        || self.runs[0].status != TaskStatus::Rejected
                        || self.runs[1].repair_of != Some(self.runs[0].task)
                        || self.runs[0].review.as_ref().is_none_or(|r| {
                            r.outcome != Outcome::NeedsRepair || r.receipt.task != self.runs[1].task
                        })))
                || (self.scenario == Scenario::QueuedForeground
                    && self
                        .foreground
                        .as_ref()
                        .is_none_or(|f| !f.queue_observed || !f.check_passed))
            {
                return Err("Accepted scenario lacks its exact checked/reviewed path".into());
            }
        } else if self.reviewed_elapsed_ms.is_some() || self.failure.is_none() {
            return Err("Unaccepted scenario cannot claim reviewed latency".into());
        }
        Ok(())
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())
}
fn bounded(s: &str, limit: usize) -> bool {
    !s.trim().is_empty() && s.len() <= limit && !s.chars().any(char::is_control)
}
fn failure_text(s: &str) -> String {
    let mut text = String::new();
    for c in s.chars().map(|c| if c.is_control() { ' ' } else { c }) {
        if text.len() + c.len_utf8() > 2048 {
            break;
        }
        text.push(c);
    }
    if text.trim().is_empty() {
        "Qualification failed without a reason".into()
    } else {
        text
    }
}
const FOREGROUND_PROMPT: &str = "Classify the decimal port numbers 0, 1, 80, 443, 65535, 65536 using the inclusive range 1 through 65535. Return only a JSON object with arrays named valid and invalid, preserving the input order within each array. No prose.";
struct Fixture {
    files: BTreeMap<String, String>,
    policy: WorkPolicy,
    criteria: Vec<String>,
    goal: String,
}
fn fixture(scenario: Scenario) -> Fixture {
    let long = scenario == Scenario::RequiredSource;
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    let (policy_files, criteria, goal) = if long {
        let mut left = "# Required fixture fact: RATE = 17\n".to_string();
        let mut right = "# Reference padding begins below.\n".to_string();
        for n in 0..140 {
            left.push_str(&format!(
                "# Left context record {n:03}: preserve the declared rate.\n"
            ));
            right.push_str(&format!(
                "# Right context record {n:03}: inspect the final offset.\n"
            ));
        }
        right.push_str("# Required fixture fact: OFFSET = 23\n");
        files.insert("reference_left.py".into(), left);
        files.insert("reference_right.py".into(), right);
        files.insert(
            "solution.py".into(),
            "def transform(value):\n    return 0\n".into(),
        );
        (
            vec![
                "reference_left.py".into(),
                "reference_right.py".into(),
                "solution.py".into(),
            ],
            vec![
                "Implement solution.transform(value) to return value * RATE + OFFSET as a Python int for positive, zero and negative integer inputs. Read RATE from reference_left.py and OFFSET from reference_right.py.".into(),
                "Preserve reference_left.py and reference_right.py byte-for-byte.".into(),
            ],
            "Implement solution.transform(value) as value times RATE plus OFFSET using the facts in reference_left.py and reference_right.py. Preserve both reference files byte-for-byte.".into(),
        )
    } else {
        files.insert(
            "solution.py".into(),
            "def parse_port(text):\n    return 0\n".into(),
        );
        let goal = if scenario == Scenario::Repair {
            "QUALIFICATION_REPAIR_SEED: this diagnostic first attempt must deliberately keep parse_port(text) returning 0 for every input so the independent check fails. A later explicit repair must ignore this seed instruction and implement the full port contract: accept only ASCII decimal strings in 1..65535, otherwise return None."
        } else {
            "Implement solution.parse_port(text): accept only nonempty ASCII decimal strings whose integer value is in 1..65535, returning that integer; return None for every other input, including whitespace, signs, Unicode digits and out-of-range numbers."
        };
        (
            vec!["solution.py".into()],
            vec![
                "Implement solution.parse_port(text): for a nonempty string containing only ASCII decimal digits 0 through 9 with an integer value from 1 through 65535 inclusive, return that integer as a Python int. Leading zeros are allowed.".into(),
                "Return Python None for every other input, including non-string values (None, booleans, numbers and collections), empty strings, whitespace, signs, decimal points, Unicode digits and out-of-range values. Do not raise for rejected inputs.".into(),
            ],
            goal.into(),
        )
    };
    let pinned = files
        .iter()
        .filter(|(path, _)| path.starts_with("reference_"))
        .map(|(p, c)| (p.clone(), hash(c.as_bytes())))
        .collect::<BTreeMap<_, _>>();
    let check = crate::qualification_oracle::check_script(long, &pinned);
    files.insert("check_fixture.py".into(), check);
    files.insert("README.md".into(), "Isolated Alfredo diagnostic fixture. Modify only the exact approved paths. check_fixture.py is the independent fixed oracle and must remain unchanged. No production work or automatic profile promotion is authorized.\n".into());
    Fixture {
        files,
        policy: WorkPolicy {
            files: policy_files,
            check: vec![
                "/usr/bin/python3".into(),
                "-B".into(),
                "check_fixture.py".into(),
            ],
        },
        criteria,
        goal,
    }
}
pub fn fixture_definition(scenario: Scenario) -> FixtureDefinition {
    let fixture = fixture(scenario);
    let mut definition = FixtureDefinition {
        scenario,
        version: 2,
        digest: String::new(),
        sources: fixture
            .files
            .iter()
            .map(|(path, content)| SourceManifest {
                path: path.clone(),
                sha256: hash(content.as_bytes()),
                bytes: content.len(),
                required: fixture.policy.files.contains(path),
            })
            .collect(),
        writable_paths: fixture.policy.files,
        check: fixture.policy.check,
        goal_sha256: hash(fixture.goal.as_bytes()),
        criteria_sha256: hash(&serde_json::to_vec(&fixture.criteria).expect("fixture criteria")),
        foreground_prompt_sha256: (scenario == Scenario::QueuedForeground)
            .then(|| hash(FOREGROUND_PROMPT.as_bytes())),
    };
    definition.digest = hash(&serde_json::to_vec(&definition).expect("fixture definition"));
    definition
}

/// The supplied case directory must be absent or empty. Artifacts are retained.
pub async fn run_scenario(
    scratch: &Path,
    provider: Ollama,
    model: &str,
    scenario: Scenario,
) -> Result<ScenarioResult> {
    run_scenario_with_cancel(
        scratch,
        provider,
        model,
        scenario,
        Arc::new(AtomicBool::new(false)),
    )
    .await
}
pub async fn run_scenario_with_cancel(
    scratch: &Path,
    provider: Ollama,
    model: &str,
    scenario: Scenario,
    cancel: Arc<AtomicBool>,
) -> Result<ScenarioResult> {
    if !bounded(model, 200) {
        return Err("Qualification model identity is invalid".into());
    }
    let path = scratch.to_owned();
    let model = model.to_owned();
    let flag = cancel.clone();
    let thread = std::thread::spawn(move || run_owned(&path, provider, &model, scenario, flag));
    struct JoinGuard {
        thread: Option<std::thread::JoinHandle<Result<ScenarioResult>>>,
        cancel: Arc<AtomicBool>,
    }
    impl Drop for JoinGuard {
        fn drop(&mut self) {
            if let Some(thread) = self.thread.take() {
                self.cancel.store(true, Ordering::SeqCst);
                let _ = thread.join(); // Do not detach an effect-owning runtime on future cancellation.
            }
        }
    }
    let mut guard = JoinGuard {
        thread: Some(thread),
        cancel,
    };
    while !guard.thread.as_ref().expect("owned thread").is_finished() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    guard
        .thread
        .take()
        .expect("owned thread")
        .join()
        .map_err(|_| "Qualification fixture thread stopped".to_string())?
}

fn run_owned(
    scratch: &Path,
    provider: Ollama,
    model: &str,
    scenario: Scenario,
    cancel: Arc<AtomicBool>,
) -> Result<ScenarioResult> {
    if !scratch.is_absolute() || scratch.to_str().is_none_or(|p| !bounded(p, 4096)) {
        return Err("Qualification needs an absolute isolated case directory".into());
    }
    match fs::symlink_metadata(scratch) {
        Ok(meta)
            if meta.is_dir()
                && !meta.file_type().is_symlink()
                && fs::read_dir(scratch)
                    .map_err(|e| e.to_string())?
                    .next()
                    .is_none() => {}
        Ok(_) => return Err("Qualification case directory must be empty and not a symlink".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(scratch).map_err(|e| e.to_string())?
        }
        Err(e) => return Err(e.to_string()),
    }
    let scratch = scratch.canonicalize().map_err(|e| e.to_string())?;
    let runtime = Runtime::new().map_err(|e| e.to_string())?;
    let started = Instant::now();
    let definition = fixture_definition(scenario);
    let mut result = ScenarioResult {
        scenario,
        fixture_digest: definition.digest.clone(),
        artifact_directory: scratch.clone(),
        scope_revision: 0,
        plan_receipt: None,
        planner_sources: vec![],
        required_sources_present: false,
        runs: vec![],
        foreground: None,
        generation_attempts: 0,
        outcome: ScenarioOutcome::Incomplete,
        elapsed_ms: 0,
        reviewed_elapsed_ms: None,
        failure: None,
    };
    let completed = execute(
        &runtime,
        &scratch,
        provider,
        model,
        &definition,
        &mut result,
        &cancel,
        started,
    );
    result.elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    match completed {
        Ok(()) => {
            result.outcome = ScenarioOutcome::Accepted;
            result.reviewed_elapsed_ms = Some(result.elapsed_ms);
        }
        Err(error) => {
            if result.outcome != ScenarioOutcome::ContextSelectionLimited {
                result.outcome = if cancel.load(Ordering::SeqCst) {
                    ScenarioOutcome::Incomplete
                } else {
                    ScenarioOutcome::Failed
                };
            }
            result.failure = Some(failure_text(&error));
        }
    }
    runtime.shutdown_timeout(Duration::from_secs(5));
    result.validate()?;
    Ok(result)
}

fn stop(cancel: &Arc<AtomicBool>, started: Instant) -> bool {
    if started.elapsed() >= SCENARIO_LIMIT {
        cancel.store(true, Ordering::SeqCst);
    }
    cancel.load(Ordering::SeqCst)
}
fn transact(store: &TaskStore, action: Action) -> Result<Receipt> {
    let revision = store.snapshot()?.revision;
    store
        .transact(Request {
            correlation: format!("qualification-action-{revision}"),
            expected_revision: revision,
            action,
        })
        .map(|(_, r)| r)
}
fn source_manifest(
    context: &crate::planning_context::RepositoryContext,
    definition: &FixtureDefinition,
) -> Vec<SourceManifest> {
    context
        .sources
        .iter()
        .map(|source| SourceManifest {
            path: source.path.clone(),
            sha256: hash(source.content.as_bytes()),
            bytes: source.content.len(),
            required: definition
                .sources
                .iter()
                .any(|s| s.path == source.path && s.required),
        })
        .collect()
}
fn required_present(sources: &[SourceManifest], definition: &FixtureDefinition) -> bool {
    definition
        .sources
        .iter()
        .filter(|s| s.required)
        .all(|required| sources.contains(required))
}
#[allow(clippy::too_many_arguments)]
fn execute(
    runtime: &Runtime,
    scratch: &Path,
    provider: Ollama,
    model: &str,
    definition: &FixtureDefinition,
    result: &mut ScenarioResult,
    cancel: &Arc<AtomicBool>,
    started: Instant,
) -> Result<()> {
    if stop(cancel, started) {
        return Err("Qualification cancelled before fixture setup".into());
    }
    let fixture = fixture(definition.scenario);
    let workspace = scratch.join("workspace");
    fs::create_dir(&workspace).map_err(|e| e.to_string())?;
    for (path, content) in &fixture.files {
        fs::write(workspace.join(path), content).map_err(|e| e.to_string())?;
    }
    runtime.block_on(async {
        worker::git(
            &workspace,
            &["init", "--template=", "--initial-branch=main"],
        )
        .await?;
        worker::git(
            &workspace,
            &["config", "user.name", "Alfredo qualification fixture"],
        )
        .await?;
        worker::git(
            &workspace,
            &["config", "user.email", "qualification@localhost"],
        )
        .await?;
        worker::git(&workspace, &["add", "--", "."]).await?;
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::process::Command::new("/usr/bin/git")
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", "/nonexistent")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
                .args([
                    "-C",
                    workspace.to_str().ok_or("Invalid fixture path")?,
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgSign=false",
                    "commit",
                    "-qm",
                    "Pinned qualification fixture",
                ])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| "Fixture commit timed out")?
        .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("Fixture commit failed".to_string());
        }
        Ok::<_, String>(())
    })?;
    let store = TaskStore::new(&scratch.join("state"), &workspace, "qualification")?;
    store.select_mission(true)?;
    let scope = store.understanding();
    scope.transact(crate::understanding::Request {
        correlation: "qualification-scope-draft".into(),
        expected_revision: 0,
        action: crate::understanding::Action::Draft {
            brief: crate::understanding::Brief {
                destination: "Run the declared isolated diagnostic fixture".into(),
                scope: "Only this temporary fixture repository and its approved files".into(),
                constraints: "Fixed independent checks; no production tasks or profile promotion"
                    .into(),
                uncertainty: "Model quality and runtime qualification remain unproven".into(),
            },
        },
    })?;
    result.scope_revision = scope
        .transact(crate::understanding::Request {
            correlation: "qualification-scope-confirm".into(),
            expected_revision: 1,
            action: crate::understanding::Action::Confirm { draft_revision: 1 },
        })?
        .revision;
    let prompt = format!("{} Return exactly one task, with no dependencies, using model {model}. The exact policy must be {}. Preserve these exact ordered acceptance criteria: {}. The independent fixture check is fixed and cannot be edited.", fixture.goal, serde_json::to_string(&fixture.policy).map_err(|e|e.to_string())?, serde_json::to_string(&fixture.criteria).map_err(|e|e.to_string())?);
    let preflight = runtime.block_on(crate::planning_context::capture(&workspace, &prompt))?;
    if !required_present(&source_manifest(&preflight, definition), definition) {
        result.outcome = ScenarioOutcome::ContextSelectionLimited;
        return Err("Native context selection omitted required fixture source bytes; no model-quality conclusion".into());
    }
    if stop(cancel, started) {
        return Err("Qualification cancelled before planner dispatch".into());
    }
    let mut planner = Planner::default();
    let request = planner
        .prepare_command(
            "qualification-plan",
            &format!("/plan {prompt}"),
            model,
            &store.snapshot()?,
        )?
        .ok_or("Planner did not prepare the fixture request")?;
    result.generation_attempts += 1;
    if stop(cancel, started) {
        return Err("Qualification cancelled before planner dispatch".into());
    }
    planner.dispatch_command(runtime, provider.clone(), &request, store.clone())?;
    while planner.active() {
        planner.poll();
        if stop(cancel, started) {
            planner.cancel();
            return Err("Qualification cancelled during planning".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let plan = planner
        .draft
        .take()
        .ok_or_else(|| format!("Fixture planning failed: {}", planner.notice))?;
    result.planner_sources = source_manifest(
        plan.context
            .as_ref()
            .ok_or("Planner omitted its native source binding")?,
        definition,
    );
    result.required_sources_present = required_present(&result.planner_sources, definition);
    if !result.required_sources_present {
        result.outcome = ScenarioOutcome::ContextSelectionLimited;
        return Err("Actual planner source binding omitted required fixture bytes".into());
    }
    if plan.tasks.len() != 1
        || plan.tasks[0].policy != fixture.policy
        || plan.tasks[0].acceptance != fixture.criteria
        || plan.tasks[0].model != model
        || !plan.tasks[0].dependencies.is_empty()
    {
        return Err(
            "Fixture draft refused: exact paths, independent check, model or criteria changed"
                .into(),
        );
    }
    if definition.scenario == Scenario::Repair
        && !plan.tasks[0].title.contains("QUALIFICATION_REPAIR_SEED")
    {
        return Err("Repair draft omitted its declared failure-seed instruction".into());
    }
    let receipt = transact(&store, Action::Plan { plan })?;
    let task = receipt.task;
    result.plan_receipt = Some(ReceiptRef::from(&receipt));
    transact(&store, Action::Approve { task })?;
    run_worker(
        runtime,
        &store,
        task,
        provider.clone(),
        result,
        cancel,
        started,
        definition.scenario == Scenario::QueuedForeground,
    )?;
    if definition.scenario == Scenario::Repair {
        if result.runs.last().is_none_or(|r| {
            r.check_passed || r.check_exit_code == Some(0) || r.check_exit_code.is_none()
        }) {
            return Err("Controlled initial failure was not independently observed; repair scenario incomplete".into());
        }
        let receipt = review(&store, result, Outcome::NeedsRepair)?;
        let child = receipt.task;
        transact(&store, Action::Approve { task: child })?;
        run_worker(
            runtime, &store, child, provider, result, cancel, started, false,
        )?;
    }
    if result
        .runs
        .last()
        .is_none_or(|r| !r.check_passed || r.status != TaskStatus::ReviewReady)
    {
        review(&store, result, Outcome::Rejected)?;
        return Err("Independent fixture check did not establish review-ready work".into());
    }
    review(&store, result, Outcome::Approved)?;
    if definition.scenario == Scenario::QueuedForeground
        && result
            .foreground
            .as_ref()
            .is_none_or(|f| !f.queue_observed || !f.check_passed)
    {
        return Err("Foreground response lacked an observed wait behind the actual worker or failed its independent check".into());
    }
    Ok(())
}

fn review(store: &TaskStore, result: &mut ScenarioResult, outcome: Outcome) -> Result<Receipt> {
    let run = result
        .runs
        .last_mut()
        .ok_or("No acknowledged run exists for fixture review")?;
    let snapshot = store.snapshot()?;
    let criteria = snapshot
        .acceptance_for_task(run.task)
        .iter()
        .enumerate()
        .map(|(n, _)| Criterion {
            criterion: n as u64 + 1,
            met: outcome.approves(),
            note: if outcome.approves() {
                "Fixed fixture oracle passed against digest-verified run evidence".into()
            } else {
                "Fixed fixture oracle did not pass; retained evidence requires repair".into()
            },
        })
        .collect();
    let decision = Decision {
        failure: None,
        risk: None,
        outcome,
        reason: if outcome == Outcome::NeedsRepair {
            "Controlled failure observed. Repair must implement the full port contract, ignoring the initial diagnostic failure-seed instruction; preserve the exact independent check.".into()
        } else {
            "Isolated qualification fixture oracle; this review never authorizes production work"
                .into()
        },
        criteria,
        limitations: vec![],
    };
    let receipt = transact(
        store,
        if outcome == Outcome::NeedsRepair {
            Action::ReviewAndRepair {
                task: run.task,
                decision,
            }
        } else {
            Action::Decide {
                task: run.task,
                decision,
            }
        },
    )?;
    run.status = store
        .snapshot()?
        .tasks
        .iter()
        .find(|t| t.id == run.task)
        .ok_or("Reviewed task disappeared")?
        .status
        .clone();
    // Compound repair receipts identify the child; retain that exact receipt and
    // separately bind the reviewed parent rather than rewriting its metadata.
    run.review = Some(ReviewRef {
        receipt: ReceiptRef::from(&receipt),
        reviewed_task: run.task,
        outcome,
    });
    Ok(receipt)
}

#[allow(clippy::too_many_arguments)]
fn run_worker(
    runtime: &Runtime,
    store: &TaskStore,
    task: u64,
    provider: Ollama,
    result: &mut ScenarioResult,
    cancel: &Arc<AtomicBool>,
    started: Instant,
    foreground: bool,
) -> Result<()> {
    if result.runs.len() >= 2 || stop(cancel, started) {
        return Err("Qualification worker bound reached or scenario cancelled".into());
    }
    let revision = store.snapshot()?.revision;
    let model = store
        .snapshot()?
        .tasks
        .iter()
        .find(|t| t.id == task)
        .ok_or("Fixture worker disappeared")?
        .model
        .clone();
    let request_observer = provider.request_observer();
    let prior_background = request_observer
        .as_ref()
        .map(|observer| observer.active_generations(crate::inference_admission::Class::Background))
        .transpose()?
        .unwrap_or_default();
    let (observer, progress) = Observer::channel();
    // Recorded profiles compare schema-constrained worker requests; pinned so
    // reports stay comparable whatever the production worker format is.
    let worker = runtime.spawn(worker::start_observed(
        store.clone(),
        task,
        format!("qualification-worker-{task}"),
        revision,
        provider
            .clone()
            .with_worker_format(worker::WorkerFormat::Json),
        cancel.clone(),
        observer,
    ));
    result.generation_attempts += 1;
    let mut foreground_job = None;
    let mut events = None;
    let mut foreground_started = None;
    let mut response = String::new();
    let mut foreground_error = None;
    while !worker.is_finished()
        || foreground_job
            .as_ref()
            .is_some_and(|job: &tokio::task::JoinHandle<()>| !job.is_finished())
    {
        let background_request = request_observer.as_ref().and_then(|observer| {
            observer
                .active_generations(crate::inference_admission::Class::Background)
                .ok()?
                .into_iter()
                .find(|sequence| !prior_background.contains(sequence))
        });
        if stop(cancel, started) {
            if let Some(job) = &foreground_job {
                job.abort();
            }
        } else if foreground
            && foreground_job.is_none()
            && background_request.is_some()
            && matches!(
                progress.borrow().stage,
                "Waiting for model server" | "Thinking" | "Receiving model plan"
            )
        {
            let (sender, receiver) = mpsc::channel(128);
            let provider = provider
                .clone()
                .with_priority(crate::inference_admission::Class::Foreground);
            let model = model.clone();
            foreground_started = Some(Instant::now());
            foreground_job = Some(runtime.spawn(async move {
                provider
                    .chat(
                        0,
                        1,
                        model,
                        vec![Message {
                            role: "user".into(),
                            content: FOREGROUND_PROMPT.into(),
                        }],
                        sender,
                    )
                    .await;
            }));
            events = Some(receiver);
            result.generation_attempts += 1;
            result.foreground = Some(ForegroundResult {
                background_request,
                queue_observed: false,
                queue_ms: None,
                completed: false,
                check_passed: false,
                response_sha256: None,
            });
        }
        if let Some(receiver) = &mut events {
            while let Ok(event) = receiver.try_recv() {
                if !observe_foreground(
                    event.update,
                    result.foreground.as_mut().expect("foreground result"),
                    foreground_started,
                    &mut response,
                    request_observer.as_ref(),
                ) {
                    foreground_error =
                        Some("Foreground request failed or exceeded its fixture bound");
                    if let Some(job) = &foreground_job {
                        job.abort();
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if let Some(job) = foreground_job {
        let _ = runtime.block_on(job);
    }
    // Completion may enter the channel immediately before the job becomes ready.
    if let Some(receiver) = &mut events {
        while let Ok(event) = receiver.try_recv() {
            if !observe_foreground(
                event.update,
                result.foreground.as_mut().expect("foreground result"),
                foreground_started,
                &mut response,
                request_observer.as_ref(),
            ) {
                foreground_error = Some("Foreground request failed or exceeded its fixture bound");
            }
        }
    }
    let worker_outcome = runtime
        .block_on(worker)
        .map_err(|_| "Qualification worker task stopped".to_string())?;
    // Finish and evidence may exist even when the asynchronous return reports a later error.
    let snapshot = store.snapshot()?;
    let item = snapshot
        .tasks
        .iter()
        .find(|t| t.id == task)
        .ok_or("Fixture task disappeared")?;
    if let Some(run) = item
        .run
        .as_ref()
        .filter(|run| run.evidence_sha256.is_some())
    {
        let evidence: Evidence = serde_json::from_str(&store.evidence(task)?)
            .map_err(|_| "Malformed verified fixture evidence")?;
        let fixed_check = fs::read(
            store
                .run_directory(&run.id)?
                .join("worktree/check_fixture.py"),
        )
        .map_err(|e| e.to_string())?;
        let expected_check = fixture(result.scenario)
            .files
            .remove("check_fixture.py")
            .expect("fixed check");
        let passed = hash(&fixed_check) == hash(expected_check.as_bytes())
            && evidence.check.as_ref().is_some_and(|check| {
                check.status == "completed"
                    && check.exit_code == Some(0)
                    && !check.reconciliation_required
                    && check.stdout.lines().any(|line| line == CHECK_OK)
            });
        result.runs.push(RunResult {
            task,
            run: run.id.clone(),
            repair_of: item.repair_of,
            status: item.status.clone(),
            evidence_sha256: run.evidence_sha256.clone().expect("filtered digest"),
            check_passed: passed,
            check_exit_code: evidence.check.as_ref().and_then(|check| check.exit_code),
            review: None,
        });
    }
    worker_outcome?;
    if let Some(observation) = &mut result.foreground {
        observation.response_sha256 = (!response.is_empty()).then(|| hash(response.as_bytes()));
        observation.check_passed = observation.completed
            && foreground_error.is_none()
            && serde_json::from_str::<serde_json::Value>(&response).ok()
                == Some(serde_json::json!({"valid":[1,80,443,65535],"invalid":[0,65536]}));
    }
    if stop(cancel, started) {
        return Err("Qualification cancelled; inspect retained canonical worker result".into());
    }
    Ok(())
}

fn observe_foreground(
    update: Update,
    result: &mut ForegroundResult,
    started: Option<Instant>,
    response: &mut String,
    observer: Option<&crate::inference_profile::RequestObserver>,
) -> bool {
    match update {
        Update::QueueProgress(queue) => {
            result.queue_observed |= queue.class == crate::inference_admission::Class::Foreground
                && queue.active > 0
                && queue.position > 0
                && result.background_request.is_some_and(|sequence| {
                    observer.is_some_and(|observer| {
                        observer
                            .generation_active(
                                sequence,
                                crate::inference_admission::Class::Background,
                            )
                            .unwrap_or(false)
                    })
                })
        }
        Update::Admitted => {
            result.queue_ms = started.map(|start| start.elapsed().as_millis() as u64)
        }
        Update::Token(text) => {
            if response.len() + text.len() > 16 * 1024 {
                return false;
            }
            response.push_str(&text);
        }
        Update::Done => result.completed = true,
        Update::Failed(_) => return false,
        _ => {}
    }
    true
}
