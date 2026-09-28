use alfredo_tui::{
    conversations::{ConversationStore, TaskView},
    model::Update,
    provider::Ollama,
    tasks::{Action, Request, TaskStore},
    workstation::Workstation,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    a: TaskStore,
    b: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-workstation-{}-{}",
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
        }
        let a = TaskStore::new(&root.join("state"), &root.join("a"), "alpha").unwrap();
        let b = TaskStore::new(&root.join("state"), &root.join("b"), "beta").unwrap();
        a.select_mission(true).unwrap();
        b.select_mission(true).unwrap();
        Self { root, a, b }
    }
    fn open(&self) -> Workstation {
        Workstation::open(
            &self.root.join("state"),
            &self.root.join("a"),
            "alpha",
            "default",
            "fixture",
            Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
        )
        .unwrap()
    }
    fn wait(work: &mut Workstation) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while work.tasks.pending {
            work.tasks.poll();
            assert!(Instant::now() < deadline, "{}", work.tasks.notice);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn handoff_restores_each_missions_drafts_models_task_view_and_owner_without_task_mutation() {
    let f = Fixture::new();
    let runtime = Runtime::new().unwrap();
    f.a.transact(Request {
        correlation: "task".into(),
        expected_revision: 0,
        action: Action::Propose {
            title: "Retained task".into(),
            model: "fixture".into(),
            dependencies: vec![],
        },
    })
    .unwrap();
    let before = serde_json::to_value(f.a.snapshot().unwrap()).unwrap();
    let mut work = f.open();
    work.app.sessions[0].insert("retained prompt");
    work.app.sessions[0].begin().unwrap();
    work.app.sessions[0].apply(1, Update::Token("retained answer".into()));
    work.app.sessions[0].apply(1, Update::Done);
    work.app.add_session();
    work.app.sessions[1].model = "other-model".into();
    work.app.sessions[1].insert("older draft");
    work.tasks
        .restore_view(TaskView {
            visible: true,
            selected: Some(1),
            query: "#1".into(),
        })
        .unwrap();
    work.autosave
        .checkpoint(
            &runtime,
            &work.app,
            work.tasks.view_preferences(),
            work.tasks.planner.checkpoint(),
        )
        .unwrap();
    work.app.sessions[1].clear_draft();
    work.app.sessions[1].insert("latest 界 draft");
    assert!(work.switch_to(&runtime, &f.root.join("b"), "beta").unwrap());
    Fixture::wait(&mut work);
    assert_eq!(work.mission(), "beta");
    assert_eq!(work.app.sessions.len(), 1);
    assert!(work.app.sessions[0].messages.is_empty());
    assert!(work.tasks.snapshot.as_ref().unwrap().tasks.is_empty());
    assert!(!work.tasks.dispatch.enabled);
    let old = ConversationStore::open(&f.a, "default").unwrap();
    let saved = old.load().unwrap().unwrap();
    assert_eq!(saved.restore().sessions[1].draft, "latest 界 draft");
    drop(old);
    work.app.sessions[0].insert("beta draft");
    assert!(work
        .switch_to(&runtime, &f.root.join("a"), "alpha")
        .unwrap());
    Fixture::wait(&mut work);
    assert_eq!(work.app.selected, 1);
    assert_eq!(work.app.sessions[1].model, "other-model");
    assert_eq!(work.app.sessions[1].draft, "latest 界 draft");
    assert_eq!(work.app.sessions[0].messages[1].content, "retained answer");
    assert_eq!(work.tasks.selected_task().unwrap().id, 1);
    assert_eq!(work.tasks.task_query, "#1");
    assert!(work.tasks.visible);
    assert!(!work.tasks.dispatch.enabled);
    assert_eq!(
        serde_json::to_value(f.a.snapshot().unwrap()).unwrap(),
        before
    );
    let beta = ConversationStore::open(&f.b, "default")
        .unwrap()
        .load()
        .unwrap()
        .unwrap()
        .restore();
    assert_eq!(beta.sessions[0].draft, "beta draft");
    assert!(!work
        .switch_to(&runtime, &f.root.join("a"), "alpha")
        .unwrap());
}

#[test]
fn unavailable_target_and_failed_final_save_keep_current_work_and_release_target_owner() {
    let f = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut work = f.open();
    work.app.sessions[0].insert("never lose this draft");
    let target_owner = ConversationStore::open(&f.b, "default").unwrap();
    assert!(work
        .switch_to(&runtime, &f.root.join("b"), "beta")
        .unwrap_err()
        .contains("in use"));
    drop(target_owner);
    assert_eq!(work.mission(), "alpha");
    assert!(ConversationStore::open(&f.a, "default").is_err());
    assert!(work
        .switch_to(&runtime, &f.root.join("missing"), "beta")
        .is_err());
    work.autosave
        .finish(
            &runtime,
            &work.app,
            work.tasks.view_preferences(),
            work.tasks.planner.checkpoint(),
        )
        .unwrap();
    let file = fs::read_dir(f.a.conversation_directory().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("conversations-")
                && p.extension().is_some_and(|ext| ext == "json")
        })
        .unwrap();
    let bytes = fs::read(&file).unwrap();
    fs::remove_file(&file).unwrap();
    fs::create_dir(&file).unwrap();
    assert!(work.switch_to(&runtime, &f.root.join("b"), "beta").is_err());
    assert_eq!(work.mission(), "alpha");
    assert_eq!(work.app.sessions[0].draft, "never lose this draft");
    assert!(ConversationStore::open(&f.a, "default").is_err());
    assert!(ConversationStore::open(&f.b, "default").is_ok());
    fs::remove_dir(&file).unwrap();
    fs::write(&file, bytes).unwrap();
    assert!(work.switch_to(&runtime, &f.root.join("b"), "beta").unwrap());
}

#[test]
fn switching_requires_quiescence_without_cancelling_or_starting_work() {
    let f = Fixture::new();
    let mut work = f.open();
    work.app.sessions[0].insert("active");
    work.app.sessions[0].begin().unwrap();
    assert!(work.can_switch().unwrap_err().contains("conversations"));
    assert!(work.app.sessions[0].status.active());
    work.app.sessions[0].cancel();
    work.tasks.dispatch.enabled = true;
    assert!(work.can_switch().unwrap_err().contains("dispatch off"));
    assert!(work.tasks.dispatch.enabled);
    work.tasks.dispatch.enabled = false;
    let cancellation = Arc::new(AtomicBool::new(false));
    work.tasks.workers.insert(1, cancellation.clone());
    assert!(work.can_switch().unwrap_err().contains("coding workers"));
    assert!(!cancellation.load(Ordering::Relaxed));
    work.tasks.workers.clear();
    work.tasks.writing = true;
    assert!(work.can_switch().unwrap_err().contains("task operation"));
    work.tasks.writing = false;
    work.app.models_pending = true;
    assert!(work.can_switch().unwrap_err().contains("model discovery"));
    work.app.models_pending = false;
    work.tasks.planner.draft = Some(alfredo_tui::planner::Plan {
        architecture: None,
        prompt: "draft".into(),
        planner: "fixture".into(),
        context: None,
        scope: None,
        tasks: vec![],
    });
    assert!(work.can_switch().unwrap_err().contains("plan-cancel"));
    work.tasks.planner.draft = None;
    assert!(work.can_switch().is_ok());
}

#[test]
fn cancelled_conversation_does_not_abandon_a_pending_scope_receipt() {
    let f = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut work = f.open();
    let scope = f.a.understanding();
    work.app.sessions[0].insert("Build a new project");
    let messages = work.app.sessions[0].begin().unwrap();
    work.wayfinder
        .start(
            &runtime,
            alfredo_tui::wayfinder::Turn {
                session: 0,
                attempt: 1,
                model: "fixture".into(),
                messages,
            },
            Some(0),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let prepared = loop {
        assert!(work.wayfinder.poll(&runtime).is_empty());
        if let Some((_, request)) = work.wayfinder.prepared().pop() {
            break request;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(scope.snapshot().unwrap().revision, 0);
    let guard = scope.lock().unwrap();
    work.wayfinder
        .dispatch_prepared(&runtime, 0, &prepared)
        .unwrap();
    work.app.sessions[0].cancel();
    assert!(!work.wayfinder.withdraw_prepared(0, &prepared));
    assert!(work.can_switch().unwrap_err().contains("scope receipt"));
    drop(guard);
    let deadline = Instant::now() + Duration::from_secs(5);
    let decision = loop {
        if let Some(completion) = work.wayfinder.poll(&runtime).pop() {
            assert_eq!(completion.turn.session, 0);
            assert_eq!(completion.turn.attempt, 1);
            assert_eq!(completion.request, Some(prepared));
            break completion.result.unwrap();
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(decision.acknowledgment.is_some());
    assert_eq!(scope.snapshot().unwrap().revision, 1);
    assert!(!scope.snapshot().unwrap().confirmed);
    assert!(matches!(
        work.app.sessions[0].status,
        alfredo_tui::model::Status::Cancelled
    ));
    assert!(work.can_switch().is_ok());
    assert!(f.a.snapshot().unwrap().tasks.is_empty());
}

#[test]
fn restart_restores_reviewable_plan_without_inference_and_retains_stale_save_guard() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut work = fixture.open();
    let plan = alfredo_tui::planner::Plan {
        architecture: None,
        prompt: "Implement calculation".into(),
        planner: "fixture".into(),
        context: None,
        scope: None,
        tasks: vec![alfredo_tui::planner::Step {
            acceptance: vec![],
            title: "Implement calculation".into(),
            model: "fixture".into(),
            dependencies: vec![],
            policy: alfredo_tui::tasks::WorkPolicy {
                files: vec!["calc.py".into()],
                check: vec!["true".into()],
            },
        }],
    };
    work.tasks.planner.draft = Some(plan.clone());
    work.autosave
        .finish(
            &runtime,
            &work.app,
            work.tasks.view_preferences(),
            work.tasks.planner.checkpoint(),
        )
        .unwrap();
    assert!(work
        .switch_to(&runtime, &fixture.root.join("b"), "beta")
        .unwrap());
    Fixture::wait(&mut work);
    assert!(work.tasks.planner.draft.is_none());
    assert!(work
        .switch_to(&runtime, &fixture.root.join("a"), "alpha")
        .unwrap());
    Fixture::wait(&mut work);
    assert_eq!(work.tasks.planner.draft, Some(plan.clone()));
    drop(work);
    fixture
        .a
        .transact(Request {
            correlation: "other".into(),
            expected_revision: 0,
            action: Action::Propose {
                title: "Other task".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        })
        .unwrap();
    let mut work = fixture.open();
    assert_eq!(work.tasks.planner.draft, Some(plan));
    assert!(!work.tasks.planner.active());
    assert!(work.tasks.planner.visible);
    assert_eq!(work.tasks.planner.revision, 0);
    work.tasks
        .command(&runtime, "/plan-save", "fixture")
        .unwrap();
    Fixture::wait(&mut work);
    assert!(work.tasks.notice.contains("changed"));
    assert_eq!(fixture.a.snapshot().unwrap().tasks.len(), 1);
    work.tasks
        .command(&runtime, "/plan-cancel", "fixture")
        .unwrap();
    work.autosave
        .finish(
            &runtime,
            &work.app,
            work.tasks.view_preferences(),
            work.tasks.planner.checkpoint(),
        )
        .unwrap();
    drop(work);
    assert!(fixture.open().tasks.planner.draft.is_none());
}
