use alfredo_tui::{
    command_intent::Intent,
    task_control::TaskControl,
    tasks::{Action, Request, TaskStatus, TaskStore},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    store: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-command-adapter-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("workspace")).unwrap();
        let store =
            TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
        Self { root, store }
    }
    fn propose(&self, title: &str) {
        let revision = self.store.snapshot().unwrap().revision;
        self.store
            .transact(Request {
                correlation: format!("seed-{revision}"),
                expected_revision: revision,
                action: Action::Propose {
                    title: title.into(),
                    model: "fixture".into(),
                    dependencies: vec![],
                },
            })
            .unwrap();
    }
    fn control(&self) -> TaskControl {
        let mut control = TaskControl::new(self.store.clone());
        control.snapshot = Some(self.store.snapshot().unwrap());
        control
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn wait(control: &mut TaskControl) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn prepared_task_captures_selection_and_never_mutates_before_dispatch() {
    let fixture = Fixture::new();
    fixture.propose("First");
    fixture.propose("Second");
    let mut control = fixture.control();
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let bytes = fs::read(&path).unwrap();
    let intent = control
        .prepare_command("/approve", "ignored-current-model")
        .unwrap()
        .unwrap();
    assert!(matches!(
        &intent,
        Intent::Task {
            request: Request {
                action: Action::Approve { task: 1 },
                ..
            }
        }
    ));
    intent.validate().unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(!control.pending && control.workers.is_empty());
    let restored: Intent = serde_json::from_slice(&serde_json::to_vec(&intent).unwrap()).unwrap();
    assert_eq!(intent, restored);
    // Serialization/restoration is inert; changing selection cannot retarget dispatch.
    assert_eq!(fs::read(&path).unwrap(), bytes);
    control.task_query = "#2".into();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    control.dispatch_prepared(&runtime, &restored).unwrap();
    assert!(control.intent_pending(&intent));
    wait(&mut control);
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Approved);
    assert_eq!(snapshot.tasks[1].status, TaskStatus::Proposed);
    assert!(intent.reconcile(Some(&snapshot), None).is_some());
    assert!(!control.intent_pending(&intent));
    assert!(control.intent_error(&intent).is_none());
    assert_eq!(
        control
            .prepare_command("/retry-task", "changed-model")
            .unwrap(),
        Some(intent.clone())
    );
    let revision = snapshot.revision;
    control.dispatch_prepared(&runtime, &intent).unwrap();
    wait(&mut control);
    assert_eq!(fixture.store.snapshot().unwrap().revision, revision);
    let mut mismatch = intent;
    if let Intent::Task { request } = &mut mismatch {
        request.action = Action::Approve { task: 2 };
    }
    assert!(mismatch.reconcile(Some(&snapshot), None).is_none());
}
#[test]
fn prepared_scope_uses_its_exact_domain_and_saved_revision() {
    let fixture = Fixture::new();
    let mut control = fixture.control();
    control.scope_view = Some(fixture.store.understanding().snapshot().unwrap());
    let intent = control.prepare_command(r#"/scope {"destination":"Goal","scope":"Bounded work","constraints":"Keep tests","uncertainty":"Performance"}"#, "fixture").unwrap().unwrap();
    assert_eq!(
        fixture.store.understanding().snapshot().unwrap().revision,
        0
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    control.dispatch_prepared(&runtime, &intent).unwrap();
    wait(&mut control);
    let scope = fixture.store.understanding().snapshot().unwrap();
    assert!(intent
        .acknowledgment(control.snapshot.as_ref(), Some(&scope))
        .unwrap()
        .starts_with("Scope receipt r1"));
    assert!(intent.reconcile(control.snapshot.as_ref(), None).is_none());
    let mut wrong = intent.clone();
    if let Intent::Scope { request } = &mut wrong {
        request.expected_revision = 1;
    }
    assert!(wrong
        .reconcile(control.snapshot.as_ref(), Some(&scope))
        .is_none());
    assert_eq!(
        control.prepare_command("/scope-retry", "fixture").unwrap(),
        Some(intent.clone())
    );
    assert!(intent
        .reconcile(control.snapshot.as_ref(), control.canonical_scope.as_ref())
        .is_some());
    control.command(&runtime, "/tasks", "fixture").unwrap();
    wait(&mut control);
    assert!(control.scope_view.is_none());
    assert!(intent
        .reconcile(control.snapshot.as_ref(), control.canonical_scope.as_ref())
        .is_some());
    let mut restarted = TaskControl::new(fixture.store.clone());
    restarted.refresh(&runtime);
    wait(&mut restarted);
    assert!(restarted.scope_view.is_none());
    assert!(intent
        .reconcile(
            restarted.snapshot.as_ref(),
            restarted.canonical_scope.as_ref()
        )
        .is_some());
    restarted.canonical_scope = None;
    restarted.refresh_background(&runtime);
    let deadline = Instant::now() + Duration::from_secs(5);
    while restarted.canonical_scope.is_none() {
        restarted.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(restarted.scope_view.is_none());
    assert!(intent
        .reconcile(
            restarted.snapshot.as_ref(),
            restarted.canonical_scope.as_ref()
        )
        .is_some());
}
#[test]
fn stale_prepared_mutation_reports_its_own_error_without_acknowledgment() {
    let fixture = Fixture::new();
    fixture.propose("First");
    let mut control = fixture.control();
    let intent = control
        .prepare_command("/approve 1", "fixture")
        .unwrap()
        .unwrap();
    fixture.propose("Intervening task");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    control.dispatch_prepared(&runtime, &intent).unwrap();
    wait(&mut control);
    assert!(!control.intent_pending(&intent));
    assert!(control
        .intent_error(&intent)
        .is_some_and(|error| error.contains("state changed")));
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Proposed);
    assert!(intent.reconcile(Some(&snapshot), None).is_none());
}
#[test]
fn prepared_run_is_inert_and_keeps_exact_claim_identity() {
    let fixture = Fixture::new();
    fixture.propose("First");
    let mut control = fixture.control();
    let intent = control
        .prepare_command("/run 1", "different-model")
        .unwrap()
        .unwrap();
    let Intent::Run {
        task,
        expected_revision,
        correlation,
    } = &intent
    else {
        panic!("Not run intent")
    };
    assert_eq!((*task, *expected_revision), (1, 1));
    assert!(!correlation.is_empty());
    assert!(control.workers.is_empty());
    assert!(intent.reconcile(control.snapshot.as_ref(), None).is_none());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert!(control.dispatch_prepared(&runtime, &intent).is_err()); // No observed open scope/policy/approval.
    assert!(control.workers.is_empty());
    assert_eq!(fixture.store.snapshot().unwrap().revision, 1);
}

#[test]
fn run_receipts_require_exact_claim_and_run_bound_result() {
    use alfredo_tui::tasks::{Receipt, TaskRun};
    let fixture = Fixture::new();
    fixture.propose("First");
    let mut snapshot = fixture.store.snapshot().unwrap();
    // Synthetic projections exercise hostile mismatches without manufacturing
    // worker evidence on disk; real transport tests cover canonical admission.
    let intent = Intent::Run {
        correlation: "claim-one".into(),
        expected_revision: 1,
        task: 1,
    };
    let baseline = "a".repeat(40);
    let digest = "b".repeat(64);
    snapshot.receipts.push(Receipt {
        request: Request {
            correlation: "claim-one".into(),
            expected_revision: 1,
            action: Action::Start {
                task: 1,
                baseline: baseline.clone(),
                inputs: vec![],
            },
        },
        revision: 2,
        task: 1,
    });
    snapshot.revision = 2;
    snapshot.tasks[0].status = TaskStatus::Running;
    snapshot.tasks[0].run = Some(TaskRun {
        id: "task-1-run-2".into(),
        baseline,
        inputs: vec![],
        evidence_sha256: None,
        detail: "Claimed".into(),
    });
    assert_eq!(intent.task_receipts(Some(&snapshot)).len(), 1);
    // A terminal task status or recorded hash cannot invent a Finish receipt.
    snapshot.tasks[0].status = TaskStatus::ReviewReady;
    snapshot.tasks[0].run.as_mut().unwrap().evidence_sha256 = Some(digest.clone());
    assert_eq!(intent.task_receipts(Some(&snapshot)).len(), 1);
    snapshot.receipts.push(Receipt {
        request: Request {
            correlation: "finish:task-1-run-2".into(),
            expected_revision: 2,
            action: Action::Finish {
                task: 1,
                run: "task-1-run-2".into(),
                status: TaskStatus::ReviewReady,
                evidence_sha256: digest,
                detail: "Complete".into(),
            },
        },
        revision: 3,
        task: 1,
    });
    snapshot.revision = 3;
    let receipts = intent.task_receipts(Some(&snapshot));
    assert_eq!(receipts.len(), 2);
    assert!(
        matches!(&receipts[1], alfredo_tui::command_intent::Acknowledgment::Task { revision: 3, task: 1, correlation } if correlation == "finish:task-1-run-2")
    );
    let cancel = Intent::Control {
        request: alfredo_tui::control_command::Request {
            correlation: "cancel-one".into(),
            controller: "local-controller".into(),
            operation: alfredo_tui::control_command::Operation::CancelWorker {
                task: 1,
                start_correlation: "claim-one".into(),
                expected_start_revision: 2,
            },
        },
    };
    cancel.validate().unwrap();
    // A request to cancel never becomes a canonical cancellation acknowledgment.
    // Its independent result slot may truthfully show a successful Finish.
    assert!(cancel.reconcile(Some(&snapshot), None).is_none());
    assert_eq!(cancel.task_receipts(Some(&snapshot)), receipts);
    assert_eq!(
        cancel.displayed_task_receipts(Some(&snapshot)),
        receipts[1..]
    );
    assert_eq!(intent.displayed_task_receipts(Some(&snapshot)), receipts);
    for wrong_revision in [0, 1, 3, u64::MAX] {
        let mut wrong = cancel.clone();
        if let Intent::Control { request } = &mut wrong {
            if let alfredo_tui::control_command::Operation::CancelWorker {
                expected_start_revision,
                ..
            } = &mut request.operation
            {
                *expected_start_revision = wrong_revision;
            }
        }
        assert!(wrong.task_receipts(Some(&snapshot)).is_empty());
    }
    // Human review changes the task outcome, never the original worker result.
    snapshot.tasks[0].status = TaskStatus::Rejected;
    assert_eq!(intent.task_receipts(Some(&snapshot)), receipts);
    for status in [TaskStatus::Failed, TaskStatus::Cancelled] {
        let mut variant = snapshot.clone();
        if let Action::Finish { status: actual, .. } = &mut variant.receipts[2].request.action {
            *actual = status;
        }
        assert_eq!(intent.task_receipts(Some(&variant)).len(), 2);
    }
    for mismatch in 0..9 {
        let mut variant = snapshot.clone();
        match mismatch {
            0 => variant.receipts[2].request.correlation = "other-finish".into(),
            1 => variant.receipts[2].revision = 1,
            2 => variant.receipts[2].task = 2,
            3 => variant.tasks[0].run.as_mut().unwrap().baseline = "c".repeat(40),
            4 => variant.tasks[0].run.as_mut().unwrap().id = "task-2-run-2".into(),
            other => {
                if let Action::Finish {
                    task,
                    run,
                    status,
                    evidence_sha256,
                    ..
                } = &mut variant.receipts[2].request.action
                {
                    match other {
                        5 => *task = 2,
                        6 => *run = "other-run".into(),
                        7 => *status = TaskStatus::Accepted,
                        8 => *evidence_sha256 = "d".repeat(64),
                        _ => unreachable!(),
                    }
                }
            }
        }
        assert_eq!(
            intent.task_receipts(Some(&variant)).len(),
            1,
            "mismatch {mismatch}"
        );
        assert_eq!(cancel.task_receipts(Some(&variant)).len(), 1);
    }
    snapshot.receipts[1].request.correlation = "different-command".into();
    assert!(intent.task_receipts(Some(&snapshot)).is_empty());
    assert!(intent.task_receipts(None).is_empty());
}

#[test]
fn planner_preparation_is_inert_and_never_a_task_receipt() {
    use alfredo_tui::{
        console_command::{CommandState, ConsoleCommand},
        conversations::{Autosave, ConversationStore},
        model::App,
        planner_command::{Operation, Outcome},
    };
    let fixture = Fixture::new();
    let mut control = fixture.control();
    let intent = control
        .prepare_command("/plan Add integration coverage", "fixture")
        .unwrap()
        .unwrap();
    assert!(
        matches!(&intent, Intent::Planner { request } if matches!(&request.operation, Operation::Generate { prompt, model, revision: 0, base_sha256: None } if prompt == "Add integration coverage" && model == "fixture"))
    );
    assert!(!control.planner.active());
    assert!(control.planner.checkpoint().is_none());
    assert!(fixture.store.snapshot().unwrap().receipts.is_empty());
    assert!(intent.reconcile(control.snapshot.as_ref(), None).is_none());
    assert!(intent.task_receipts(control.snapshot.as_ref()).is_empty());
    let mut app = App::new("fixture".into());
    let id = app.sessions[0]
        .submit_command("/plan Add integration coverage".into(), intent.clone())
        .unwrap();
    let saved = app.sessions[0].commands()[0].clone();
    let store = ConversationStore::open(&fixture.store, "planner-gate").unwrap();
    let mut autosave = Autosave::new(store);
    assert!(!autosave.contains_saved_command(0, &saved));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    assert!(autosave.contains_saved_command(0, &saved));
    assert!(!control.planner.active());
    assert!(app.sessions[0].messages.is_empty());
    app.sessions[0].set_command_state(
        &id,
        CommandState::Planner {
            outcome: Outcome::Generated {
                draft_sha256: "a".repeat(64),
                tasks: 2,
            },
        },
    );
    assert!(app.sessions[0].retry_command(&id).is_err());
    let mut forged: ConsoleCommand = saved;
    forged.intent = Intent::Task {
        request: Request {
            correlation: "not-a-plan".into(),
            expected_revision: 0,
            action: Action::Propose {
                title: "Task".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        },
    };
    forged.id = ConsoleCommand::identity(&forged.intent);
    forged.state = CommandState::Planner {
        outcome: Outcome::Stopped,
    };
    assert!(!forged.valid(0));
}
