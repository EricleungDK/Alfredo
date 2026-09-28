use alfredo_tui::{
    conversations::ConversationStore,
    provider::Ollama,
    selection_command::{Choice, MissionChoice, Origin, Phase, Request, WorkspaceChoice},
    selection_store::Store,
    tasks::TaskStore,
    workstation::Workstation,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::runtime::Runtime;
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    state: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-selection-lifecycle-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self {
            state: root.join("state"),
            root,
        }
    }
    fn provider(&self) -> Ollama {
        Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap()
    }
    fn create_choice(&self) -> Choice {
        Choice {
            workspace: WorkspaceChoice::Create {
                parent: self.root.clone(),
                name: "new repository".into(),
            },
            mission: MissionChoice::StartNew {
                name: "mission".into(),
            },
        }
    }
    fn source(&self, runtime: &Runtime) -> Workstation {
        let workspace = self.root.join("source");
        runtime
            .block_on(alfredo_tui::selection::acknowledge(
                &workspace,
                &self.state,
                true,
            ))
            .unwrap();
        TaskStore::new(&self.state, &workspace, "source")
            .unwrap()
            .select_mission(true)
            .unwrap();
        Workstation::open(
            &self.state,
            &workspace,
            "source",
            "default",
            "fixture",
            self.provider(),
        )
        .unwrap()
    }
    fn source_request(&self, work: &Workstation) -> Request {
        Request::new(
            Origin::Conversation {
                workspace: work.workspace().to_path_buf(),
                mission: work.mission().into(),
                conversation: "default".into(),
                session: 0,
            },
            self.create_choice(),
            "default".into(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn startup_needs_synced_journal_admission_before_creating_repository_or_mission() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let journal = fixture.state.join("rust-selection-v1");
    fs::create_dir_all(journal.join("selections.json")).unwrap();
    let request = Request::new(Origin::Startup, fixture.create_choice(), "default".into()).unwrap();
    assert!(Workstation::launch(
        &runtime,
        &fixture.state,
        request,
        "fixture",
        fixture.provider()
    )
    .is_err());
    assert!(!fixture.create_choice().target().exists());
    assert!(!fixture.state.join("rust-tasks-v1").exists());
    fs::remove_dir(journal.join("selections.json")).unwrap();
    let lock = fs::File::create(journal.join("selection.lock")).unwrap();
    lock.lock().unwrap();
    let request = Request::new(Origin::Startup, fixture.create_choice(), "default".into()).unwrap();
    let result = Workstation::launch(
        &runtime,
        &fixture.state,
        request,
        "fixture",
        fixture.provider(),
    );
    assert!(result.err().unwrap().contains("busy"));
    assert!(!fixture.create_choice().target().exists());
    assert!(!journal.join("selections.json").exists());
    assert!(!fixture.state.join("rust-tasks-v1").exists());
    lock.unlock().unwrap();
}

#[test]
fn failed_source_intent_save_does_not_admit_or_create_and_keeps_source_owner() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut work = fixture.source(&runtime);
    work.app.sessions[0].insert("keep this draft");
    work.autosave
        .finish(
            &runtime,
            &work.app,
            work.tasks.view_preferences(),
            work.tasks.planner.checkpoint(),
        )
        .unwrap();
    let tasks = TaskStore::new(&fixture.state, work.workspace(), work.mission()).unwrap();
    let saved = tasks.conversation_directory().unwrap().join(format!(
        "conversations-{:x}.json",
        Sha256::digest(b"default")
    ));
    fs::remove_file(&saved).unwrap();
    fs::create_dir(&saved).unwrap();
    let request = fixture.source_request(&work);
    assert!(work.select(&runtime, request).is_err());
    assert!(!fixture.create_choice().target().exists());
    assert!(Store::new(&fixture.state)
        .unwrap()
        .snapshot()
        .unwrap()
        .is_empty());
    assert_eq!(work.app.sessions[0].draft, "keep this draft");
    assert_eq!(work.mission(), "source");
    assert!(ConversationStore::open(&tasks, "default").is_err());
}

#[test]
fn repository_creation_survives_failed_mission_preparation_with_honest_history() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut work = fixture.source(&runtime);
    work.app.sessions[0].insert("source draft survives");
    let request = fixture.source_request(&work);
    let target = request.choice.target();
    // Existing mission content is an intentional conflict after repository creation.
    let identity = serde_json::to_vec(&(&target, "mission")).unwrap();
    let namespace = fixture
        .state
        .join("rust-tasks-v1")
        .join(format!("{:x}", Sha256::digest(identity)));
    fs::create_dir_all(&namespace).unwrap();
    fs::write(namespace.join("tasks.json"), b"existing mission evidence").unwrap();
    assert!(work.select(&runtime, request.clone()).is_err());
    assert!(target.join(".git").is_dir());
    assert_eq!(
        fs::read(namespace.join("tasks.json")).unwrap(),
        b"existing mission evidence"
    );
    assert_eq!(work.mission(), "source");
    assert_eq!(work.app.sessions[0].draft, "source draft survives");
    let records = Store::new(&fixture.state).unwrap().snapshot().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].request, request);
    assert_eq!(records[0].outcome.phase, Phase::RepositoryReady);
    assert!(records[0]
        .outcome
        .failure
        .as_ref()
        .unwrap()
        .contains("already exists"));
    assert!(records[0].dispatched);
    assert!(
        Store::new(&fixture.state).unwrap().admit(request).is_err(),
        "uncertain creation must not replay"
    );
}

#[test]
fn failed_startup_reports_created_artifact_without_claiming_a_retained_workstation() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let request = Request::new(Origin::Startup, fixture.create_choice(), "default".into()).unwrap();
    let target = request.choice.target();
    let identity = serde_json::to_vec(&(&target, "mission")).unwrap();
    let namespace = fixture
        .state
        .join("rust-tasks-v1")
        .join(format!("{:x}", Sha256::digest(identity)));
    fs::create_dir_all(&namespace).unwrap();
    fs::write(namespace.join("tasks.json"), b"existing mission evidence").unwrap();
    let error = Workstation::launch(
        &runtime,
        &fixture.state,
        request,
        "fixture",
        fixture.provider(),
    )
    .err()
    .unwrap();
    assert!(error.contains("No workspace selected"), "{error}");
    assert!(!error.contains("Current work retained"), "{error}");
    assert!(error.contains(target.to_str().unwrap()), "{error}");
    assert!(target.join(".git").is_dir());
}
