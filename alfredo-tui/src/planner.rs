//! Model-generated drafts have no task authority until an explicit Plan receipt.
use crate::{
    model::{Message, Update},
    provider::Ollama,
    tasks::WorkPolicy,
};
use serde::{Deserialize, Serialize};
use tokio::{runtime::Runtime, sync::mpsc, task::JoinHandle};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance: Vec<String>,
    pub model: String,
    pub dependencies: Vec<u64>,
    pub policy: WorkPolicy,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub prompt: String,
    pub planner: String,
    pub tasks: Vec<Step>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<crate::planning_context::RepositoryContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Box<crate::understanding::Binding>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<crate::architecture::Origin>,
}
impl Plan {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(scope) = &self.scope {
            scope.validate()?;
        }
        if let Some(context) = &self.context {
            context.validate()?;
        }
        if let Some(origin) = &self.architecture {
            origin.validate()?;
            if self.tasks.len() != 1
                || !self.tasks[0].dependencies.is_empty()
                || self.tasks[0].acceptance.is_empty()
            {
                return Err("Architect revision requires exactly one repair step with acceptance criteria and no new dependencies".into());
            }
        }
        let text = |s: &str, max| {
            !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
        };
        if !text(&self.prompt, 8192)
            || !text(&self.planner, 200)
            || self.tasks.is_empty()
            || self.tasks.len() > 16
        {
            return Err("Plan needs a bounded prompt, planner and 1–16 tasks".into());
        }
        for (index, step) in self.tasks.iter().enumerate() {
            if !text(&step.title, 8192) || !text(&step.model, 200) {
                return Err("Invalid plan task title or model".into());
            }
            let unique: std::collections::BTreeSet<_> = step.dependencies.iter().collect();
            if unique.len() != step.dependencies.len()
                || step
                    .dependencies
                    .iter()
                    .any(|id| *id == 0 || *id > index as u64)
            {
                return Err(
                    "Plan dependencies must name distinct earlier steps (starting at 1)".into(),
                );
            }
            let criteria: std::collections::BTreeSet<_> =
                step.acceptance.iter().map(|s| s.trim()).collect();
            if step.acceptance.len() > 16
                || criteria.len() != step.acceptance.len()
                || step.acceptance.iter().any(|s| !text(s, 1024))
            {
                return Err("Acceptance criteria need at most 16 distinct nonempty lines of up to 1024 bytes".into());
            }
            if step.policy.files.is_empty() {
                return Err(format!(
                    "Task {} lists no policy files; every task must write at least one file.",
                    index + 1
                ));
            }
            step.policy.validate()?;
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 128 * 1024 {
            return Err("Plan exceeds 128 KiB".into());
        }
        Ok(())
    }
}

enum PlannerEvent {
    Architecture {
        origin: crate::architecture::Origin,
        prompt: String,
        model: String,
        revision: u64,
    },
    Scope(crate::understanding::Binding),
    Context(crate::planning_context::RepositoryContext),
    Model(crate::model::Event),
}

/// Review data only: restoration grants no task or execution authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedDraft {
    pub plan: Plan,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<crate::planner_command::Request>,
}
impl SavedDraft {
    pub fn validate(&self) -> Result<(), String> {
        self.plan.validate()?;
        if let Some(origin) = &self.origin {
            use crate::planner_command::Operation;
            origin.validate()?;
            let matches = match &origin.operation {
                Operation::Generate {
                    prompt,
                    model,
                    revision,
                    ..
                } => {
                    self.revision == *revision
                        && self.plan.prompt == *prompt
                        && self.plan.planner == *model
                }
                Operation::Revise {
                    prompt, revision, ..
                } => {
                    self.revision == *revision
                        && self
                            .plan
                            .prompt
                            .ends_with(&format!(" | Revision request: {}", prompt.trim()))
                }
                Operation::Architect { origin, revision } => {
                    self.revision == *revision && self.plan.architecture.as_ref() == Some(origin)
                }
                Operation::Cancel { .. } => false,
            };
            if !matches {
                return Err("Saved draft does not match its planner origin".into());
            }
        }
        if self.revision == u64::MAX
            || serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 256 * 1024
        {
            return Err("Saved plan exceeds supported bounds".into());
        }
        Ok(())
    }
}

impl SavedDraft {
    pub fn digest(&self) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&self.plan, self.revision)).map_err(|e| e.to_string())?
            )
        ))
    }
    pub fn outcome_for(
        &self,
        request: &crate::planner_command::Request,
    ) -> Option<crate::planner_command::Outcome> {
        if self.origin.as_ref() != Some(request) || self.validate().is_err() {
            return None;
        }
        Some(crate::planner_command::Outcome::Generated {
            draft_sha256: self.digest().ok()?,
            tasks: self.plan.tasks.len(),
        })
    }
}

struct GenerationRequest {
    prompt: String,
    model: String,
    revision: u64,
    reference: Option<String>,
    architecture: Option<crate::architecture::Origin>,
    architecture_task: Option<u64>,
    expected_revision: Option<u64>,
}

struct PreviousDraft {
    plan: Plan,
    revision: u64,
    metrics: Option<crate::metrics::Metrics>,
    origin: Option<crate::planner_command::Request>,
}

#[derive(Default)]
pub struct Planner {
    pub draft: Option<Plan>,
    pub revision: u64,
    pub visible: bool,
    pub notice: String,
    pub partial: String,
    pub metrics: Option<crate::metrics::Metrics>,
    previous: Option<PreviousDraft>,
    job: Option<JoinHandle<()>>,
    events: Option<mpsc::Receiver<PlannerEvent>>,
    context: Option<crate::planning_context::RepositoryContext>,
    scope: Option<crate::understanding::Binding>,
    architecture: Option<crate::architecture::Origin>,
    prompt: String,
    model: String,
    draft_origin: Option<crate::planner_command::Request>,
    command: Option<crate::planner_command::Request>,
    command_events: Vec<crate::planner_command::Event>,
    generation: Option<String>,
    command_revision: Option<u64>,
    /// Validation findings for the last previewed draft.
    warnings: std::cell::RefCell<Option<(Plan, Vec<String>)>>,
    /// When the current generation was spawned (display only).
    started: Option<std::time::Instant>,
}

/// Retry and revision text appended to a planner request. The user's goal is
/// the text before the first marker; older saved plans used the first form.
const GOAL_MARKERS: [&str; 3] = [
    " | The previous plan was rejected by validation:",
    " | Earlier plans were rejected by validation",
    " | Revision request: ",
];

/// The user's original goal from a plan request, without planner retry text.
pub fn goal(prompt: &str) -> &str {
    let end = GOAL_MARKERS
        .iter()
        .filter_map(|marker| prompt.find(marker))
        .min()
        .unwrap_or(prompt.len());
    prompt[..end].trim()
}
impl Drop for Planner {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl Planner {
    pub fn prepare_command(
        &self,
        correlation: &str,
        text: &str,
        model: &str,
        snapshot: &crate::tasks::Snapshot,
    ) -> Result<Option<crate::planner_command::Request>, String> {
        use crate::planner_command::{Operation, Request};
        let text = text.trim();
        let draft = self.checkpoint();
        let base = draft.as_ref().map(SavedDraft::digest).transpose()?;
        let operation = if text == "/plan-cancel" {
            Operation::Cancel {
                generation: self.generation.clone(),
                draft_sha256: base,
            }
        } else if let Some(prompt) = text.strip_prefix("/plan-revise ") {
            if self.active() {
                return Err("Wait for the current plan before revising it".into());
            }
            Operation::Revise {
                prompt: prompt.trim().into(),
                revision: snapshot.revision,
                base_sha256: base.ok_or("No complete draft to revise")?,
            }
        } else if let Some(id) = text.strip_prefix("/architect-revise ") {
            if self.active() || self.draft.is_some() {
                return Err("Save or cancel the current plan before Architect revision".into());
            }
            let task = id.parse().map_err(|_| "Usage: /architect-revise ID")?;
            Operation::Architect {
                origin: snapshot.architecture_origin(task)?,
                revision: snapshot.revision,
            }
        } else if let Some(prompt) = text.strip_prefix("/plan ") {
            if self.active() {
                return Err("Wait for the current plan or explicitly cancel it first".into());
            }
            Operation::Generate {
                prompt: prompt.trim().into(),
                model: model.into(),
                revision: snapshot.revision,
                base_sha256: base,
            }
        } else {
            return Ok(None);
        };
        let request = Request {
            correlation: correlation.into(),
            operation,
        };
        request.validate()?;
        Ok(Some(request))
    }

    pub fn command_pending(&self, request: &crate::planner_command::Request) -> bool {
        self.command.as_ref() == Some(request)
    }

    pub fn take_command_events(&mut self) -> Vec<crate::planner_command::Event> {
        std::mem::take(&mut self.command_events)
    }

    pub fn dispatch_command(
        &mut self,
        runtime: &Runtime,
        provider: Ollama,
        request: &crate::planner_command::Request,
        store: crate::tasks::TaskStore,
    ) -> Result<(), String> {
        use crate::planner_command::{Event, Operation, Outcome};
        request.validate()?;
        if let Some(outcome) = self
            .checkpoint()
            .and_then(|saved| saved.outcome_for(request))
        {
            self.command_events.push(Event {
                request: request.clone(),
                outcome,
            });
            return Ok(());
        }
        let base = self
            .checkpoint()
            .as_ref()
            .map(SavedDraft::digest)
            .transpose()?;
        if let Operation::Cancel {
            generation,
            draft_sha256,
        } = &request.operation
        {
            if *generation != self.generation || *draft_sha256 != base {
                return Err(
                    "Planner cancellation target changed; inspect the current draft".into(),
                );
            }
            let was_active = self.active();
            self.cancel();
            if !was_active {
                self.draft = None;
                self.draft_origin = None;
            }
            self.notice = "Planning stopped · inspect retained draft before continuing".into();
            self.command_events.push(Event {
                request: request.clone(),
                outcome: Outcome::Stopped,
            });
            return Ok(());
        }
        if self.active() {
            return Err("A planner generation is already active".into());
        }
        let revision = match &request.operation {
            Operation::Generate {
                revision,
                base_sha256,
                ..
            } => {
                if *base_sha256 != base {
                    return Err("Plan draft changed before generation".into());
                }
                *revision
            }
            Operation::Revise {
                revision,
                base_sha256,
                ..
            } => {
                if base.as_ref() != Some(base_sha256) {
                    return Err("Plan revision base changed; inspect the current draft".into());
                }
                *revision
            }
            Operation::Architect { revision, .. } => {
                if base.is_some() {
                    return Err("An unsaved draft exists; save or cancel it first".into());
                }
                *revision
            }
            Operation::Cancel { .. } => unreachable!(),
        };
        self.command_revision = Some(revision);
        let result = match &request.operation {
            Operation::Generate { prompt, model, .. } => {
                self.start(runtime, provider, prompt, model, revision, store)
            }
            Operation::Revise { prompt, .. } => {
                self.revise(runtime, provider, prompt, revision, store)
            }
            Operation::Architect { origin, .. } => self.generate(
                runtime,
                provider,
                GenerationRequest {
                    prompt: "Architect repair revision".into(),
                    model: "pending".into(),
                    revision,
                    reference: None,
                    architecture: Some(origin.clone()),
                    architecture_task: Some(origin.task),
                    expected_revision: Some(revision),
                },
                store,
            ),
            Operation::Cancel { .. } => unreachable!(),
        };
        self.command_revision = None;
        result?;
        self.generation = Some(request.correlation.clone());
        self.command = Some(request.clone());
        Ok(())
    }

    pub fn checkpoint(&self) -> Option<SavedDraft> {
        if let Some(previous) = &self.previous {
            Some(SavedDraft {
                plan: previous.plan.clone(),
                revision: previous.revision,
                origin: previous.origin.clone(),
            })
        } else {
            self.draft.as_ref().map(|plan| SavedDraft {
                plan: plan.clone(),
                revision: self.revision,
                origin: self.draft_origin.clone(),
            })
        }
    }
    pub fn restore(&mut self, saved: SavedDraft) -> Result<(), String> {
        saved.validate()?;
        self.cancel();
        self.context = saved.plan.context.clone();
        self.scope = saved.plan.scope.as_deref().cloned();
        self.architecture = saved.plan.architecture.clone();
        self.prompt = saved.plan.prompt.clone();
        self.model = saved.plan.planner.clone();
        self.draft_origin = saved.origin;
        self.revision = saved.revision;
        self.draft = Some(saved.plan);
        self.partial.clear();
        self.metrics = None;
        self.visible = true;
        self.notice = "Restored plan draft · review before /plan-save; task state may have changed · no inference resumed".into();
        Ok(())
    }
    pub fn active(&self) -> bool {
        self.job.is_some()
    }
    /// Planner model of the current or last generation.
    pub fn model(&self) -> &str {
        &self.model
    }
    /// Start of the generation in flight, if any.
    pub fn started(&self) -> Option<std::time::Instant> {
        self.started.filter(|_| self.active())
    }
    fn finish_command(&mut self, outcome: crate::planner_command::Outcome) {
        if let Some(request) = self.command.take() {
            self.command_events
                .push(crate::planner_command::Event { request, outcome });
        }
    }
    pub fn cancel(&mut self) {
        self.finish_command(crate::planner_command::Outcome::Stopped);
        self.generation = None;
        if let Some(job) = self.job.take() {
            job.abort();
        }
        self.events = None;
        if let Some(previous) = self.previous.take() {
            self.draft_origin = previous.origin;
            self.revision = previous.revision;
            self.context = previous.plan.context.clone();
            self.scope = previous.plan.scope.as_deref().cloned();
            self.architecture = previous.plan.architecture.clone();
            self.prompt = previous.plan.prompt.clone();
            self.model = previous.plan.planner.clone();
            self.metrics = previous.metrics;
            self.draft = Some(previous.plan);
            self.notice.push_str(" · previous draft retained");
        }
    }
    pub fn start(
        &mut self,
        runtime: &Runtime,
        provider: Ollama,
        prompt: &str,
        model: &str,
        revision: u64,
        store: crate::tasks::TaskStore,
    ) -> Result<(), String> {
        self.generate(
            runtime,
            provider,
            GenerationRequest {
                prompt: prompt.into(),
                model: model.into(),
                revision,
                reference: None,
                architecture: None,
                architecture_task: None,
                expected_revision: self.command_revision,
            },
            store,
        )
    }
    pub fn revise(
        &mut self,
        runtime: &Runtime,
        provider: Ollama,
        request: &str,
        revision: u64,
        store: crate::tasks::TaskStore,
    ) -> Result<(), String> {
        if self.active() {
            return Err("Wait for the current plan before revising it".into());
        }
        if request.trim().is_empty() || request.chars().any(char::is_control) {
            return Err("Usage: /plan-revise REQUEST".into());
        }
        let plan = self
            .draft
            .clone()
            .ok_or("No complete draft to revise; use /plan REQUEST first")?;
        plan.validate()?;
        let reference = serde_json::to_string(&plan.tasks).map_err(|e| e.to_string())?;
        if reference.len() > 64 * 1024 {
            return Err(
                "Draft exceeds revision context limit; start a smaller /plan request".into(),
            );
        }
        let previous = PreviousDraft {
            plan: plan.clone(),
            revision: self.revision,
            metrics: self.metrics.clone(),
            origin: self.draft_origin.clone(),
        };
        self.generate(
            runtime,
            provider,
            GenerationRequest {
                prompt: format!("{} | Revision request: {}", plan.prompt, request.trim()),
                model: plan.planner,
                revision,
                reference: Some(reference),
                architecture: plan.architecture,
                architecture_task: None,
                expected_revision: self.command_revision,
            },
            store,
        )?;
        self.previous = Some(previous);
        Ok(())
    }
    pub fn revise_architecture(
        &mut self,
        runtime: &Runtime,
        provider: Ollama,
        task: u64,
        store: crate::tasks::TaskStore,
    ) -> Result<(), String> {
        if self.active() {
            return Err("Wait for the active plan before opening Architect revision".into());
        }
        if self.draft.is_some() {
            return Err("An unsaved plan draft exists; save or explicitly /plan-cancel it before Architect revision".into());
        }
        self.generate(
            runtime,
            provider,
            GenerationRequest {
                prompt: "Architect repair revision".into(),
                model: "pending".into(),
                revision: 0,
                reference: None,
                architecture: None,
                architecture_task: Some(task),
                expected_revision: self.command_revision,
            },
            store,
        )?;
        self.notice = "Reading verified architecture review evidence · no tasks saved".into();
        Ok(())
    }
    fn generate(
        &mut self,
        runtime: &Runtime,
        provider: Ollama,
        request: GenerationRequest,
        store: crate::tasks::TaskStore,
    ) -> Result<(), String> {
        let prompt = request.prompt.as_str();
        let model = request.model.as_str();
        let revision = request.revision;
        // Validate the caller's envelope before any inference request.
        Plan {
            prompt: prompt.into(),
            planner: model.into(),
            context: None,
            scope: None,
            architecture: request.architecture.clone(),
            tasks: vec![Step {
                title: "validation".into(),
                acceptance: if request.architecture.is_some() {
                    vec!["Validation placeholder".into()]
                } else {
                    vec![]
                },
                model: model.into(),
                dependencies: vec![],
                policy: WorkPolicy {
                    files: vec!["file".into()],
                    check: vec!["true".into()],
                },
            }],
        }
        .validate()?;
        self.cancel();
        self.draft = None;
        self.draft_origin = None;
        static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        self.generation = Some(format!(
            "planner-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        self.partial.clear();
        self.metrics = None;
        self.context = None;
        self.scope = None;
        self.architecture = request.architecture.clone();
        self.prompt = prompt.into();
        self.model = model.into();
        self.revision = revision;
        self.visible = true;
        self.notice = "Generating draft · no tasks saved · /plan-cancel stops inference".into();
        let (sender, receiver) = mpsc::channel(32);
        self.events = Some(receiver);
        self.notice = "Reading committed repository context · no tasks saved".into();
        self.started = Some(std::time::Instant::now());
        self.job = Some(runtime.spawn(async move {
            if let Some(expected) = request.expected_revision {
                let current_store = store.clone();
                let checked = tokio::task::spawn_blocking(move || {
                    if current_store.snapshot()?.revision != expected { return Err("Task state changed before planner dispatch".to_string()); }
                    Ok(())
                }).await;
                if !matches!(checked, Ok(Ok(()))) {
                    let reason = match checked { Ok(Err(reason)) => reason, _ => "Planner state reader stopped".into() };
                    let _ = sender.send(PlannerEvent::Model(crate::model::Event { session:0, attempt:0, update:Update::Failed(reason) })).await;
                    return;
                }
            }
            let mut prompt = request.prompt;
            let mut model = request.model;
            let mut reference = request.reference;
            let mut architecture = request.architecture;
            let mut architecture_baseline = None;
            let task = request.architecture_task.or_else(|| architecture.as_ref().map(|origin| origin.task));
            if let Some(task) = task {
                let context_store = store.clone();
                let context = match tokio::task::spawn_blocking(move || context_store.architecture_context(task)).await {
                    Ok(Ok(context)) => context,
                    result => {
                        let error = match result { Ok(Err(error)) => error, _ => "Architect context reader stopped".into() };
                        let _ = sender.send(PlannerEvent::Model(crate::model::Event { session: 0, attempt: 0, update: Update::Failed(error) })).await;
                        return;
                    }
                };
                if architecture.as_ref().is_some_and(|origin| origin != &context.origin)
                    || request.expected_revision.is_some_and(|revision| revision != context.revision)
                    || (request.architecture_task.is_none() && request.revision != context.revision) {
                    let _ = sender.send(PlannerEvent::Model(crate::model::Event { session: 0, attempt: 0, update: Update::Failed("Architect source or task state changed; refresh before revision".into()) })).await;
                    return;
                }
                if request.architecture_task.is_some() { prompt = context.prompt; }
                model = context.model;
                architecture_baseline = Some(context.baseline);
                reference = Some(match reference {
                    Some(previous) => format!("{}\nPrevious unsaved revision:\n{previous}", context.reference),
                    None => context.reference,
                });
                architecture = Some(context.origin.clone());
                if sender.send(PlannerEvent::Architecture { origin: context.origin, prompt: prompt.clone(), model: model.clone(), revision: context.revision }).await.is_err() { return; }
            }
            let envelope = Plan {
                prompt: prompt.clone(), planner: model.clone(), context: None, scope: None,
                architecture: architecture.clone(),
                tasks: vec![Step { title: "Validation".into(), acceptance: vec!["Validation".into()], model: model.clone(),
                    dependencies: vec![], policy: WorkPolicy { files: vec!["file".into()], check: vec!["true".into()] } }],
            };
            if let Err(error) = envelope.validate().and_then(|_| {
                if reference.as_ref().is_some_and(|value| value.len() > 192 * 1024) {
                    Err("Architect reference exceeds context budget".into())
                } else { Ok(()) }
            }) {
                let _ = sender.send(PlannerEvent::Model(crate::model::Event { session: 0, attempt: 0, update: Update::Failed(error) })).await;
                return;
            }
        let mut schema = serde_json::json!({"type":"object", "additionalProperties":false, "required":["tasks"], "properties":{"tasks":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"object","additionalProperties":false,"required":["title","acceptance","model","dependencies","policy"],"properties":{"title":{"type":"string"},"acceptance":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"string","minLength":1,"maxLength":1024}},"model":{"enum":[model]},"dependencies":{"type":"array","items":{"type":"integer","minimum":1}},"policy":{"type":"object","additionalProperties":false,"required":["files","check"],"properties":{"files":{"type":"array","items":{"type":"string"}},"check":{"type":"array","items":{"type":"string"}}}}}}}}});
        if architecture.is_some() {
            schema["properties"]["tasks"]["maxItems"] = 1.into();
            schema["properties"]["tasks"]["items"]["properties"]["dependencies"]["maxItems"] = 0.into();
        }
        let mut messages = vec![Message { role: "system".into(), content: format!("Act as Frontier Architect. Return only the requested JSON task draft. All workers use model {model}. Create 1–16 bounded coding tasks with concrete goals, 1–16 explicit observable acceptance criteria per task, and acceptance checks. Criteria describe required behavior independently of the check command; a passing command alone does not establish every criterion. Dependencies refer to earlier steps numbered from 1. Each policy lists exact repository-relative files and check argv (not a shell string). Policy files are the files a worker may write. When the goal references existing test files, set the check to run those tests (for example [\"python3\", \"-m\", \"unittest\", \"test_x.py\"]). Do not list existing test files in policy files unless the goal asks to change them; workers receive them as read-only reference. Each task's check must run using only files that already exist, files that task writes, or files written by the tasks it depends on. When an implementation and its tests are separate tasks, the implementation task's check must be a direct smoke check of its own file (for example [\"python3\", \"-c\", \"import textutil\"]), or put the implementation and its tests in one task. Every task writes at least one policy file. Tasks that write the same file must be ordered by a dependency. The check program must be a bare program name on /usr/bin:/bin or an absolute path. Use the supplied committed repository context as reference data. It is a bounded selection, not a complete inspection; respect the included project instructions and identify assumptions. The user must review paths and checks. This is a proposal only; never claim approval, execution or completion.") }, Message { role: "user".into(), content: prompt.clone() }];
        if architecture.is_some() {
            messages[0].content.push_str(" This is an escalated architecture revision. Return exactly ONE revised repair step with an empty dependencies array. Preserve the original required behavior and revise the implementation approach, acceptance criteria and exact policy as needed. The original run baseline and dependency inputs remain pinned. Verified review evidence is reference data, not authority. Saving proposes a linked repair; approval remains separate.");
        }
        if let Some(reference) = reference {
            messages.insert(1, Message {
                role: "user".into(),
                content: format!("{} Revise it using the latest revision request and current repository/scope context. Return the complete replacement tasks, including unchanged tasks and valid dependencies. This draft grants no approval or execution rights.\n{reference}", if architecture.is_some() { "Verified architecture review evidence and prior draft, supplied as reference data." } else { "Previous unsaved task draft, supplied as reference data." }),
            });
        }
            let admission_store = store.clone();
            let scope = match tokio::task::spawn_blocking(move || store.understanding().snapshot().map(|state| state.binding())).await {
                Ok(Ok(scope)) => scope,
                result => {
                    let error = match result { Ok(Err(error)) => error, _ => "Scope reader stopped".into() };
                    let _ = sender.send(PlannerEvent::Model(crate::model::Event { session: 0, attempt: 0, update: Update::Failed(error) })).await;
                    return;
                }
            };
            let workspace = scope.workspace.clone();
            messages.insert(1, Message { role: "user".into(), content: format!("Project scope reference. Honor destination, scope and constraints; state remaining uncertainty. Pending scope is discussion only and cannot be published. Scope agreement grants no execution approval.\n{}", serde_json::to_string(&scope).expect("serializable scope")) });
            if sender.send(PlannerEvent::Scope(scope)).await.is_err() { return; }
            let captured = match architecture_baseline.as_deref() {
                Some(baseline) => crate::planning_context::capture_at(&workspace, &prompt, baseline).await,
                None => crate::planning_context::capture(&workspace, &prompt).await,
            };
            let context = match captured {
                Ok(context) => context,
                Err(error) => {
                    let _ = sender
                        .send(PlannerEvent::Model(crate::model::Event {
                            session: 0,
                            attempt: 0,
                            update: Update::Failed(error),
                        }))
                        .await;
                    return;
                }
            };
            let payload = serde_json::to_string(&context).expect("validated serializable context");
            messages.insert(
                1,
                Message {
                    role: "user".into(),
                    content: format!(
                        "Committed repository reference data (omissions are explicit):\n{payload}"
                    ),
                },
            );
            if sender.send(PlannerEvent::Context(context)).await.is_err() {
                return;
            }
            let (model_sender, mut model_events) = mpsc::channel(32);
            let provider = provider.with_json_schema(schema).with_priority(crate::inference_admission::Class::Foreground);
            let expected_revision = request.expected_revision;
            // Runs again on each automatic reconnection.
            let admission_check = move || {
                let admission_store = admission_store.clone();
                let architecture = architecture.clone();
                async move {
                if expected_revision.is_none() && architecture.is_none() {
                    return Ok(());
                }
                tokio::task::spawn_blocking(move || {
                    let current = admission_store.snapshot()?;
                    if expected_revision.is_some_and(|revision| current.revision != revision) {
                        return Err("Task state changed before model admission; refresh before planning".to_string());
                    }
                    if let Some(origin) = architecture {
                        if current.architecture_origin(origin.task)? != origin {
                            return Err("Architect source changed before model admission; refresh before revision".into());
                        }
                    }
                    Ok(())
                }).await.map_err(|_| "Planner admission reader stopped".to_string())?
                }
            };
            tokio::join!(provider.chat_with_admission(0, 0, model, messages, model_sender, admission_check), async {
                while let Some(event) = model_events.recv().await {
                    if sender.send(PlannerEvent::Model(event)).await.is_err() {
                        break;
                    }
                }
            });
        }));
        Ok(())
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let event = match self.events.as_mut().map(|events| events.try_recv()) {
                Some(Ok(event)) => event,
                Some(Err(mpsc::error::TryRecvError::Disconnected)) => {
                    self.notice =
                        "Planner stopped without a complete response · no action taken".into();
                    self.finish_command(crate::planner_command::Outcome::failed(&self.notice));
                    self.cancel();
                    return true;
                }
                _ => break,
            };
            changed = true;
            let event = match event {
                PlannerEvent::Architecture {
                    origin,
                    prompt,
                    model,
                    revision,
                } => {
                    self.architecture = Some(origin);
                    self.prompt = prompt;
                    self.model = model;
                    self.revision = revision;
                    continue;
                }
                PlannerEvent::Scope(scope) => {
                    self.scope = Some(scope);
                    continue;
                }
                PlannerEvent::Context(context) => {
                    self.context = Some(context);
                    continue;
                }
                PlannerEvent::Model(event) => event,
            };
            match event.update {
                Update::Metrics(metrics) => self.metrics = Some(metrics),
                Update::Thinking => {
                    if self.partial.is_empty() {
                        self.notice = "Model thinking · no tasks saved".into();
                    }
                }
                Update::Queued => {
                    self.notice =
                        "Plan waiting for shared Alfredo capacity · no action taken".into()
                }
                Update::QueueProgress(queue) => {
                    self.notice = format!(
                        "{} · no action taken",
                        crate::client_timing::queue_summary(&queue)
                    );
                }
                Update::Admitted => {
                    self.notice = "Plan waiting for model server · no action taken".into()
                }
                Update::Retrying(retry) => {
                    self.notice = format!(
                        "Reconnecting to model server in {}s · retry {}/{} · no action taken",
                        retry.delay.as_secs(),
                        retry.retry,
                        retry.limit
                    )
                }
                Update::Token(token) => {
                    if self.partial.len() + token.len() > 128 * 1024 {
                        self.notice = "Plan output limit reached · no action taken".into();
                        self.finish_command(crate::planner_command::Outcome::failed(&self.notice));
                        self.cancel();
                        break;
                    }
                    self.partial.push_str(&token);
                    self.notice = format!(
                        "Generating draft · {} bytes · no tasks saved",
                        self.partial.len()
                    );
                }
                Update::Failed(error) => {
                    self.notice = format!("{error} · no action taken");
                    self.finish_command(crate::planner_command::Outcome::failed(&error));
                    self.cancel();
                    break;
                }
                Update::Done => {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Output {
                        tasks: Vec<Step>,
                    }
                    let result = serde_json::from_str::<Output>(&self.partial)
                        .map_err(|e| format!("Invalid plan JSON: {e}"))
                        .and_then(|output| {
                            let plan = Plan {
                                prompt: self.prompt.clone(),
                                planner: self.model.clone(),
                                tasks: output.tasks,
                                context: self.context.clone(),
                                scope: self.scope.clone().map(Box::new),
                                architecture: self.architecture.clone(),
                            };
                            plan.validate()?;
                            if plan.tasks.iter().any(|task| task.acceptance.is_empty()) {
                                return Err(
                                    "Generated tasks require explicit acceptance criteria".into()
                                );
                            }
                            if plan.tasks.iter().any(|task| task.model != self.model) {
                                return Err("Planner changed the selected worker model".into());
                            }
                            Ok(plan)
                        });
                    match result {
                        Ok(plan) => {
                            let revised = self.previous.take().is_some();
                            self.draft_origin = self.command.clone();
                            self.draft = Some(plan);
                            if let Some(saved) = self.checkpoint() {
                                if let Some(request) = self.command.as_ref() {
                                    if let Some(outcome) = saved.outcome_for(request) {
                                        self.finish_command(outcome);
                                    }
                                }
                            }
                            self.notice = format!("Review {} paths, checks and dependencies · /plan-revise REQUEST refines; /plan-save proposes; approval remains separate", if revised { "revised draft" } else { "draft" });
                        }
                        Err(error) => {
                            self.notice = format!("{error} · no action taken");
                            self.finish_command(crate::planner_command::Outcome::failed(&error));
                        }
                    }
                    self.cancel();
                    break;
                }
            }
        }
        changed
    }
    pub fn preview(&self) -> String {
        let context = self
            .context
            .as_ref()
            .map(|context| {
                format!(
                    "Commit {} · read {} source files · {} other files not read\nSources: {}",
                    context.baseline,
                    context.sources.len(),
                    context.omitted_sources,
                    context
                        .sources
                        .iter()
                        .map(|source| source.path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .unwrap_or_else(|| "Repository context not collected".into());
        let timing = self
            .metrics
            .as_ref()
            .map(|metrics| format!("\n{}", metrics.summary()))
            .unwrap_or_default();
        let warnings = self.draft.as_ref().map_or_else(String::new, |draft| {
            let mut cache = self.warnings.borrow_mut();
            if cache.as_ref().is_none_or(|(plan, _)| plan != draft) {
                *cache = Some((draft.clone(), crate::plan_lint::findings(draft)));
            }
            let findings = &cache.as_ref().expect("cached findings").1;
            if findings.is_empty() {
                return String::new();
            }
            format!(
                "\nValidation warnings · /plan-save still allowed; autopilot would re-plan:\n{}",
                findings
                    .iter()
                    .map(|finding| format!("- {finding}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
        format!("{}{timing}\n\nFrontier Architect draft · Local Agent worker assignments\nNot saved or approved · committed context only; working edits excluded\n{}{warnings}\n\n{}", self.notice, context, self.draft.as_ref().and_then(|p| serde_json::to_string_pretty(p).ok()).unwrap_or_else(|| self.partial.clone()))
    }
}
