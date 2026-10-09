//! One admitted workspace/mission and its conversation owner. Handoff is prepared
//! before replacing the current work; failures leave its UI and owner intact.
use crate::{
    console_command::{bounded_reason, CommandState},
    conversations::{Autosave, ConversationStore},
    model::App,
    provider::Ollama,
    selection_command::{Choice, MissionChoice, Origin, Outcome, Phase, Request, WorkspaceChoice},
    selection_store::{Operation, Store as SelectionStore},
    task_control::TaskControl,
    tasks::TaskStore,
};
use std::path::{Path, PathBuf};
use tokio::runtime::Runtime;

pub struct Workstation {
    pub app: App,
    pub tasks: TaskControl,
    pub wayfinder: crate::wayfinder::Router,
    pub autosave: Autosave,
    pub autopilot: crate::autopilot::Autopilot,
    workspace: PathBuf,
    mission: String,
    state: PathBuf,
    conversation: String,
    default_model: String,
    provider: Ollama,
    /// Dropped with this workstation: switching or quitting stops the poll.
    health: Option<crate::health::Monitor>,
}
impl Workstation {
    pub fn open(
        state: &Path,
        workspace: &Path,
        mission: &str,
        conversation: &str,
        model: &str,
        provider: Ollama,
    ) -> Result<Self, String> {
        let store = TaskStore::new(state, workspace, mission)?;
        store.select_mission(false)?;
        let snapshot = store.snapshot()?;
        let conversations = ConversationStore::open(&store, conversation)?;
        let autopilot =
            crate::autopilot::Autopilot::open(&store.conversation_directory()?, conversation)?;
        let restored = conversations.load()?;
        let plan = restored.as_ref().and_then(|saved| saved.plan_draft.clone());
        let view = restored.as_ref().and_then(|saved| saved.task_view.clone());
        let mut app = restored
            .map(|saved| saved.restore())
            .unwrap_or_else(|| App::new(model.into()));
        reconcile_selection(&mut app, state);
        if let Err(error) = crate::missions::remember(&store, workspace, mission) {
            app.notice = format!("Saved-mission discovery unavailable: {error}");
        }
        let wayfinder = crate::wayfinder::Router::new(store.understanding());
        let owner =
            crate::instruct::Instructions::open(&store.conversation_directory()?, conversation);
        let mut tasks = TaskControl::new(store);
        match owner {
            Ok(owner) => tasks.owner = owner,
            Err(error) => app.notice = format!("{error}; file preserved, instructions start empty"),
        }
        if let Some(notice) = tasks.load_agent_drafts(conversation) {
            app.notice = notice;
        }
        tasks.snapshot = Some(snapshot);
        if let Some(view) = view {
            tasks.restore_view(view)?;
        }
        if let Some(plan) = plan {
            tasks.planner.restore(plan)?;
            tasks.visible = true;
        }
        tasks.set_provider(provider.clone());
        if app.notice.is_empty() && crate::worker::lacks_commits(workspace) {
            app.notice = crate::worker::NO_COMMITS.into();
        }
        Ok(Self {
            app,
            tasks,
            wayfinder,
            autosave: Autosave::new(conversations),
            autopilot,
            workspace: workspace.canonicalize().map_err(|e| e.to_string())?,
            mission: mission.into(),
            state: state.into(),
            conversation: conversation.into(),
            default_model: model.into(),
            provider,
            health: None,
        })
    }
    /// Start the server health poll and warm the selected model; neither blocks.
    fn start_health(&mut self, runtime: &Runtime) {
        let monitor = crate::health::Monitor::start(
            runtime.handle(),
            self.provider.clone(),
            crate::health::POLL_INTERVAL,
        );
        self.app.health = monitor.view();
        self.app
            .health
            .preload(&self.app.sessions[self.app.selected].model);
        self.health = Some(monitor);
    }
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
    pub fn mission(&self) -> &str {
        &self.mission
    }
    /// Refresh the read-only autopilot projection. A finished loop replaces the
    /// stale footer notice (such as “Autopilot started …”) with its result, once.
    pub fn sync_autopilot(&mut self) -> bool {
        let mut changed = false;
        let status = self.autopilot.status(&self.tasks);
        if status != self.tasks.autopilot {
            self.tasks.autopilot = status;
            changed = true;
        }
        if let Some(notice) = self.autopilot.take_finished_notice() {
            self.app.notice = notice;
            changed = true;
        }
        let roots = self.autopilot.roots(&self.tasks);
        if roots != self.tasks.autopilot_roots {
            self.tasks.autopilot_roots = roots;
            changed = true;
        }
        if let Some(notice) = self.tasks.owner.take_notice() {
            self.app.notice = notice;
            changed = true;
        }
        changed
    }
    pub fn can_switch(&self) -> Result<(), String> {
        if self.wayfinder.active() {
            return Err("Wait for the Wayfinder scope receipt before switching work".into());
        }
        if self
            .app
            .sessions
            .iter()
            .any(|session| session.status.active())
        {
            return Err("Finish or cancel active conversations before switching work".into());
        }
        if self.app.models_pending {
            return Err("Wait for model discovery before switching work".into());
        }
        if self.autopilot.running() {
            return Err("Pause autopilot (/pause or F5) before switching work".into());
        }
        self.tasks.can_switch()
    }
    pub fn conversation(&self) -> &str {
        &self.conversation
    }
    fn save(&mut self, runtime: &Runtime) -> Result<(), String> {
        // A best-effort companion save; failing to keep a draft never blocks a handoff.
        crate::agent_view::persist(&self.app, &mut self.tasks);
        self.autosave.finish(
            runtime,
            &self.app,
            self.tasks.view_preferences(),
            self.tasks.planner.checkpoint(),
        )
    }
    /// Startup also needs a journal admission: no conversation owner exists yet.
    pub fn launch(
        runtime: &Runtime,
        state: &Path,
        request: Request,
        model: &str,
        provider: Ollama,
    ) -> Result<Self, String> {
        request.validate()?;
        if request.origin != Origin::Startup {
            return Err("Startup selection requires its launcher origin".into());
        }
        let mut progress = SelectionProgress::begin(state, request)?;
        let prepared = (|| {
            let mut next = Self::prepare_target(runtime, state, model, provider, &mut progress)?;
            let session = next.app.selected;
            let id = next.app.sessions[session].submit_selection_arrival(
                progress.request().clone(),
                Outcome::at(Phase::HandoffPrepared),
            )?;
            next.save(runtime)?;
            progress.advance(Phase::HandoffPrepared)?;
            Ok::<_, String>((next, session, id))
        })();
        let (mut next, session, id) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return Err(progress.fail(&error).1),
        };
        // Returning this workstation is the startup handoff; report publication
        // failures as a notice on the selected target, never as a fictitious rollback.
        let publication = progress.advance(Phase::Selected);
        next.app.sessions[session].set_command_state(&id, observed_state(&progress, &publication));
        let save = next.save(runtime);
        if let Err(error) = publication.and(save) {
            next.app.notice = format!("Workspace selected; selection history unconfirmed: {error}");
        }
        Ok(next)
    }
    fn prepare_target(
        runtime: &Runtime,
        state: &Path,
        model: &str,
        provider: Ollama,
        progress: &mut SelectionProgress,
    ) -> Result<Self, String> {
        let request = progress.request().clone();
        let target = request.choice.target();
        let create = matches!(request.choice.workspace, WorkspaceChoice::Create { .. });
        let checked =
            runtime.block_on(crate::selection::prepare_workspace(&target, state, create))?;
        if checked != request.choice.workspace {
            return Err(
                "Workspace identity changed after selection; inspect and choose it again".into(),
            );
        }
        let prepared = runtime.block_on(crate::selection::acknowledge_prepared(
            &request.choice.workspace,
            state,
        ))?;
        if prepared != target {
            return Err(
                "Prepared workspace differs from the saved selection; inspect both paths".into(),
            );
        }
        progress.advance(Phase::RepositoryReady)?;
        let tasks = TaskStore::new(state, &target, request.choice.mission.name())?;
        tasks.select_mission(request.choice.mission.start_new())?;
        progress.advance(Phase::MissionReady)?;
        let mut next = Self::open(
            state,
            &target,
            request.choice.mission.name(),
            &request.conversation,
            model,
            provider,
        )?;
        progress.advance(Phase::TargetLoaded)?;
        next.start_health(runtime);
        Ok(next)
    }
    pub fn select(&mut self, runtime: &Runtime, request: Request) -> Result<bool, String> {
        self.select_with(runtime, request, |_, _| Ok(()))
    }
    fn select_with(
        &mut self,
        runtime: &Runtime,
        request: Request,
        before_handoff: impl FnOnce(&Self, &Self) -> Result<(), String>,
    ) -> Result<bool, String> {
        request.validate()?;
        self.can_switch()?;
        let session = match &request.origin {
            Origin::Conversation {
                workspace,
                mission,
                conversation,
                session,
            } if workspace == &self.workspace
                && mission == &self.mission
                && conversation == &self.conversation
                && *session < self.app.sessions.len() =>
            {
                *session
            }
            _ => {
                return Err(
                    "Selection no longer belongs to this workspace, mission and session".into(),
                )
            }
        };
        let text = format!(
            "Select workspace {} · {} mission {}",
            request.choice.target().display(),
            if request.choice.mission.start_new() {
                "new"
            } else {
                "resume"
            },
            request.choice.mission.name()
        );
        let id = self.app.sessions[session].submit_selection_command(text, request.clone())?;
        let admission = self.save(runtime).and_then(|()| {
            let command = self.app.sessions[session]
                .commands()
                .iter()
                .find(|c| c.id == id)
                .ok_or("Selection source command disappeared")?;
            if !self.autosave.contains_saved_command(session, command) {
                return Err("Exact selection source command was not saved".into());
            }
            SelectionProgress::begin(&self.state, request)
        });
        let mut progress = match admission {
            Ok(progress) => progress,
            Err(error) => {
                self.app.sessions[session].set_command_state(
                    &id,
                    CommandState::Refused {
                        reason: bounded_reason(&format!(
                            "Selection admission failed before preparation: {error}"
                        )),
                    },
                );
                let _ = self.save(runtime);
                return Err(error);
            }
        };
        if progress.request().already_current() {
            let result = progress.advance(Phase::AlreadyCurrent);
            self.app.sessions[session].set_command_state(&id, observed_state(&progress, &result));
            let saved = self.save(runtime);
            result.and(saved)?;
            return Ok(false);
        }
        let prepared = (|| {
            let mut next = Self::prepare_target(
                runtime,
                &self.state,
                &self.default_model,
                self.provider.clone(),
                &mut progress,
            )?;
            let arrival_session = next.app.selected;
            let arrival = next.app.sessions[arrival_session].submit_selection_arrival(
                progress.request().clone(),
                Outcome::at(Phase::HandoffPrepared),
            )?;
            self.app.sessions[session].set_command_state(
                &id,
                CommandState::Selection {
                    outcome: progress.outcome.clone(),
                },
            );
            // Both owners remain held through the last source checkpoint and the
            // destination capacity/save gate. Tests inject real save failure here.
            before_handoff(self, &next)?;
            self.save(runtime)?;
            next.save(runtime)?;
            progress.advance(Phase::HandoffPrepared)?;
            Ok::<_, String>((next, arrival_session, arrival))
        })();
        let (next, arrival_session, arrival) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                let (state, reason) = progress.fail(&error);
                self.app.sessions[session].set_command_state(&id, state);
                let _ = self.save(runtime);
                return Err(reason);
            }
        };
        let mut previous = std::mem::replace(self, next);
        let publication = progress.advance(Phase::Selected);
        let state = observed_state(&progress, &publication);
        previous.app.sessions[session].set_command_state(&id, state.clone());
        self.app.sessions[arrival_session].set_command_state(&arrival, state);
        let source_save = previous.save(runtime);
        let target_save = self.save(runtime);
        self.tasks.refresh(runtime);
        if let Err(error) = publication.and(source_save).and(target_save) {
            self.app.notice = format!("Workspace selected; selection history unconfirmed: {error}");
        }
        Ok(true)
    }
    /// Compatibility entry point; it still uses exact origin admission and the journal.
    pub fn switch_to(
        &mut self,
        runtime: &Runtime,
        workspace: &Path,
        mission: &str,
    ) -> Result<bool, String> {
        let workspace = runtime.block_on(crate::selection::prepare_workspace(
            workspace,
            &self.state,
            false,
        ))?;
        let request = Request::new(
            Origin::Conversation {
                workspace: self.workspace.clone(),
                mission: self.mission.clone(),
                conversation: self.conversation.clone(),
                session: self.app.selected,
            },
            Choice {
                workspace,
                mission: MissionChoice::Resume {
                    name: mission.into(),
                },
            },
            self.conversation.clone(),
        )?;
        self.select(runtime, request)
    }
}

struct SelectionProgress {
    store: SelectionStore,
    operation: Operation,
    outcome: Outcome,
}
impl SelectionProgress {
    fn begin(state: &Path, request: Request) -> Result<Self, String> {
        let store = SelectionStore::new(state)?;
        let admitted = store.admit(request)?;
        let operation = store.begin(admitted)?;
        Ok(Self {
            store,
            operation,
            outcome: Outcome::at(Phase::Admitted),
        })
    }
    fn request(&self) -> &Request {
        self.operation.request()
    }
    fn advance(&mut self, phase: Phase) -> Result<(), String> {
        self.outcome = Outcome::at(phase);
        self.store.record(&self.operation, self.outcome.clone())
    }
    fn fail(&mut self, error: &str) -> (CommandState, String) {
        self.outcome = self.outcome.clone().failed(error);
        let publication = self.store.record(&self.operation, self.outcome.clone());
        let retained = if matches!(self.request().origin, Origin::Startup) {
            "No workspace selected"
        } else {
            "Current work retained"
        };
        let reason = format!("Selection stopped after {} at {}: {error}. {retained}; inspect partial target artifacts before a new Open or Resume choice", self.outcome.phase.label(), self.request().choice.target().display());
        (observed_state(self, &publication), reason)
    }
}
fn observed_state(progress: &SelectionProgress, publication: &Result<(), String>) -> CommandState {
    match publication {
        Ok(()) => CommandState::Selection { outcome: progress.outcome.clone() },
        Err(error) => CommandState::Unknown { reason: bounded_reason(&format!("Selection last observed {} but journal publication is unconfirmed: {error}; inspect before choosing again", progress.outcome.phase.label())) },
    }
}
fn reconcile_selection(app: &mut App, state: &Path) {
    if !app.sessions.iter().any(|s| {
        s.commands()
            .iter()
            .any(|c| c.intent.selection_request().is_some())
    }) {
        return;
    }
    let records = SelectionStore::new(state).and_then(|store| store.snapshot());
    for session in &mut app.sessions {
        for command in session.commands().to_vec() {
            let Some(request) = command.intent.selection_request() else {
                continue;
            };
            let record = records
                .as_ref()
                .ok()
                .and_then(|records| records.iter().find(|record| &record.request == request));
            let arrival = matches!(
                command.intent,
                crate::command_intent::Intent::SelectionArrival { .. }
            );
            let state = match record {
                Some(record) if (record.outcome.failure.is_some() || matches!(record.outcome.phase, Phase::Selected | Phase::AlreadyCurrent)) && (!arrival || matches!(record.outcome.phase, Phase::HandoffPrepared | Phase::Selected)) => CommandState::Selection { outcome: record.outcome.clone() },
                Some(record) => CommandState::Unknown { reason: bounded_reason(&format!("Selection interrupted after {}; effects and handoff may be unconfirmed. Inspect before a new Open or Resume choice", record.outcome.phase.label())) },
                None => CommandState::Unknown { reason: "Exact selection journal unavailable; outcome unconfirmed. Inspect before a new Open or Resume choice".into() },
            };
            session.set_command_state(&command.id, state);
        }
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };
    static ID: AtomicU64 = AtomicU64::new(0);
    #[test]
    fn final_source_save_failure_after_target_loading_retains_source_and_releases_target() {
        let root = std::env::temp_dir().join(format!(
            "alfredo-selection-final-save-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        for name in ["a", "b"] {
            fs::create_dir_all(root.join(name)).unwrap();
            assert!(std::process::Command::new("git")
                .args(["init", "-q", "--template="])
                .arg(root.join(name))
                .status()
                .unwrap()
                .success());
            TaskStore::new(&root.join("state"), &root.join(name), name)
                .unwrap()
                .select_mission(true)
                .unwrap();
        }
        let a = TaskStore::new(&root.join("state"), &root.join("a"), "a").unwrap();
        let b = TaskStore::new(&root.join("state"), &root.join("b"), "b").unwrap();
        let runtime = Runtime::new().unwrap();
        let mut work = Workstation::open(
            &root.join("state"),
            &root.join("a"),
            "a",
            "default",
            "fixture",
            Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
        )
        .unwrap();
        work.app.sessions[0].insert("keep this unsent draft");
        let request = Request::new(
            Origin::Conversation {
                workspace: root.join("a"),
                mission: "a".into(),
                conversation: "default".into(),
                session: 0,
            },
            Choice {
                workspace: WorkspaceChoice::Existing {
                    path: root.join("b"),
                },
                mission: MissionChoice::Resume { name: "b".into() },
            },
            "default".into(),
        )
        .unwrap();
        let original = request.clone();
        let mut removed = None;
        let error = work
            .select_with(&runtime, request, |source, target| {
                assert_eq!(target.mission(), "b");
                assert!(ConversationStore::open(&a, "default").is_err());
                assert!(ConversationStore::open(&b, "default").is_err());
                assert_eq!(target.app.sessions[0].commands().len(), 1);
                assert_eq!(source.app.sessions[0].draft, "keep this unsent draft");
                let file = fs::read_dir(a.conversation_directory().unwrap())
                    .unwrap()
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .find(|p| {
                        p.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("conversations-")
                            && p.extension().is_some_and(|e| e == "json")
                    })
                    .unwrap();
                let bytes = fs::read(&file).unwrap();
                fs::remove_file(&file).unwrap();
                fs::create_dir(&file).unwrap();
                removed = Some((file, bytes));
                Ok(())
            })
            .unwrap_err();
        assert!(error.contains("target loaded"), "{error}");
        assert_eq!(work.mission(), "a");
        assert_eq!(work.app.sessions[0].draft, "keep this unsent draft");
        assert!(ConversationStore::open(&a, "default").is_err());
        assert!(ConversationStore::open(&b, "default").is_ok());
        let record = SelectionStore::new(&root.join("state"))
            .unwrap()
            .snapshot()
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(record.request, original);
        assert_eq!(record.outcome.phase, Phase::TargetLoaded);
        assert!(record.outcome.failure.is_some());
        let (file, bytes) = removed.unwrap();
        fs::remove_dir(&file).unwrap();
        fs::write(file, bytes).unwrap();
        drop(work);
        fs::remove_dir_all(root).unwrap();
    }
}
