use alfredo_tui::{
    conversations::TaskView,
    mission_work::NodeId,
    planner::{Plan, Step},
    task_control::TaskControl,
    tasks::{Action, Request, Snapshot, TaskStatus, TaskStore, WorkPolicy},
    worker::Evidence,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

static ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    store: TaskStore,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-work-navigation-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("workspace")).unwrap();
        let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "tree").unwrap();
        let fixture = Self { root, store };
        fixture.action(Action::Plan {
            plan: Plan {
                architecture: None,
                prompt: "Implement a dependency diamond".into(),
                planner: "fixture".into(),
                context: None,
                scope: None,
                tasks: [
                    ("Foundation", vec![]),
                    ("Left branch", vec![1]),
                    ("Right branch", vec![1]),
                    ("Join branches", vec![2, 3]),
                ]
                .into_iter()
                .map(|(title, dependencies)| Step {
                    title: title.into(),
                    acceptance: vec!["Preserve the requested behavior".into()],
                    model: "fixture".into(),
                    dependencies,
                    policy: WorkPolicy {
                        files: vec!["task.txt".into()],
                        check: vec!["true".into()],
                    },
                })
                .collect(),
            },
        });
        fixture.action(Action::Propose {
            title: "Manual notes".into(),
            model: "fixture".into(),
            dependencies: vec![],
        });
        fixture
    }

    fn action(&self, action: Action) -> Snapshot {
        let revision = self.store.snapshot().unwrap().revision;
        self.store
            .transact(Request {
                correlation: format!("navigation-fixture-{revision}"),
                expected_revision: revision,
                action,
            })
            .unwrap()
            .0
    }

    fn fail(&self, task: u64) -> String {
        self.action(Action::Approve { task });
        let snapshot = self.action(Action::Start {
            task,
            baseline: "a".repeat(40),
            inputs: vec![],
        });
        let run = snapshot
            .tasks
            .iter()
            .find(|item| item.id == task)
            .unwrap()
            .run
            .as_ref()
            .unwrap();
        let bytes = serde_json::to_vec(&Evidence {
            agent: None,
            run: run.id.clone(),
            baseline: run.baseline.clone(),
            status: TaskStatus::Failed,
            detail: format!("Exact retained failure for task #{task}"),
            patch: String::new(),
            candidate_commit: None,
            model_metrics: None,
            failure_code: None,
            generation: None,
            check: None,
        })
        .unwrap();
        let directory = self.store.run_directory(&run.id).unwrap();
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("evidence.json"), &bytes).unwrap();
        self.action(Action::Finish {
            task,
            run: run.id.clone(),
            status: TaskStatus::Failed,
            evidence_sha256: format!("{:x}", Sha256::digest(&bytes)),
            detail: format!("Exact retained failure for task #{task}"),
        });
        run.id.clone()
    }

    fn control(&self, runtime: &Runtime) -> TaskControl {
        let mut control = TaskControl::new(self.store.clone());
        control.refresh(runtime);
        settle(&mut control);
        assert!(control.newly_observed_receipts().is_empty());
        control
    }

    fn bytes(&self) -> Vec<u8> {
        fs::read(
            self.store
                .conversation_directory()
                .unwrap()
                .join("tasks.json"),
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn settle(control: &mut TaskControl) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(2));
    }
}

fn command(control: &mut TaskControl, runtime: &Runtime, text: &str) {
    control.command(runtime, text, "fixture").unwrap();
    settle(control);
}

#[test]
fn groups_are_navigable_without_task_authority_or_receipts() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut control = fixture.control(&runtime);
    let before = fixture.bytes();
    assert_eq!(control.selected_task().unwrap().id, 1);
    let tree = control.work_tree();
    control.select_task(false);
    assert_eq!(control.focused_work_node(), Some(NodeId::Plan(1)));
    assert!(control.selected_task().is_none());
    assert_eq!(control.view_preferences().selected, Some(1));
    assert!(
        Arc::ptr_eq(&tree, &control.work_tree()),
        "focus must reuse canonical projection"
    );
    for action in [
        "/approve",
        "/run",
        "/accept",
        "/reject",
        "/evidence",
        "/recover",
    ] {
        assert_eq!(
            control.prepare_command(action, "fixture").unwrap_err(),
            "Select a task first"
        );
        assert_eq!(
            control.command(&runtime, action, "fixture").unwrap_err(),
            "Select a task first"
        );
    }
    assert!(control.collapse_work_node());
    assert_eq!(
        control
            .work_tree()
            .rows
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![NodeId::Plan(1), NodeId::Manual, NodeId::Task(5)]
    );
    control.select_task(true);
    assert_eq!(control.focused_work_node(), Some(NodeId::Manual));
    assert!(control.selected_task().is_none());
    assert!(control.expand_work_node());
    assert_eq!(control.selected_task().unwrap().id, 5);
    assert_eq!(control.view_preferences().selected, Some(5));
    assert_eq!(fixture.bytes(), before);
    assert!(control.newly_observed_receipts().is_empty());
    assert!(control.take_control_events().is_empty());
}

#[test]
fn hidden_task_anchor_survives_filters_refresh_and_restart_until_explicit_navigation() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut control = fixture.control(&runtime);
    command(&mut control, &runtime, "/tasks #3");
    command(&mut control, &runtime, "/tasks");
    command(&mut control, &runtime, "/tasks Left branch");
    assert_eq!(control.view_preferences().selected, Some(3));
    assert!(control.selected_task().is_none());
    assert!(control.focused_work_node().is_none());
    assert!(control.prepare_command("/run", "fixture").is_err());
    let hidden = control.view_preferences();
    let bytes = fixture.bytes();
    let mut restored = TaskControl::new(fixture.store.clone());
    restored.restore_view(hidden).unwrap();
    restored.refresh(&runtime);
    settle(&mut restored);
    assert!(restored.selected_task().is_none());
    assert_eq!(restored.view_preferences().selected, Some(3));
    command(&mut restored, &runtime, "/tasks");
    assert_eq!(restored.selected_task().unwrap().id, 3);
    assert_eq!(fixture.bytes(), bytes);

    fixture.action(Action::Propose {
        title: "New external task".into(),
        model: "fixture".into(),
        dependencies: vec![],
    });
    restored.refresh(&runtime);
    settle(&mut restored);
    assert_eq!(restored.selected_task().unwrap().id, 3);
    assert_eq!(restored.work_tree().total_tasks, 6);
    command(&mut restored, &runtime, "/tasks no match");
    assert!(restored.selected_task().is_none());
    command(&mut restored, &runtime, "/tasks Left branch");
    restored.select_task(true);
    assert_eq!(restored.focused_work_node(), Some(NodeId::Plan(1)));
    assert!(restored.selected_task().is_none());
    restored.select_task(true);
    assert_eq!(restored.selected_task().unwrap().id, 2);
    command(&mut restored, &runtime, "/tasks #5");
    assert_eq!(restored.selected_task().unwrap().id, 5);
}

#[test]
fn collapsed_groups_restore_exact_task_and_explicit_ids_reveal_their_path() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut control = fixture.control(&runtime);
    command(&mut control, &runtime, "/tasks #3");
    command(&mut control, &runtime, "/tasks");
    assert!(control.collapse_work_node());
    assert_eq!(control.focused_work_node(), Some(NodeId::Plan(1)));
    assert!(control.collapse_work_node());
    assert!(control.selected_task().is_none());
    assert_eq!(control.view_preferences().selected, Some(3));
    let bytes = fixture.bytes();
    let mut restored = TaskControl::new(fixture.store.clone());
    restored.restore_view(control.view_preferences()).unwrap();
    assert!(restored.selected_task().is_none());
    restored.refresh(&runtime);
    settle(&mut restored);
    assert_eq!(restored.selected_task().unwrap().id, 3);
    assert_eq!(restored.work_tree().rows.len(), 7);
    command(&mut control, &runtime, "/tasks #3");
    command(&mut control, &runtime, "/tasks");
    assert_eq!(control.selected_task().unwrap().id, 3);
    assert_eq!(control.work_tree().rows.len(), 7);
    assert_eq!(fixture.bytes(), bytes);
}

#[test]
fn repair_order_does_not_duplicate_diamond_and_evidence_reveals_exact_run() {
    let fixture = Fixture::new();
    fixture.fail(1);
    fixture.action(Action::Repair {
        task: 1,
        reason: "Repair foundation".into(),
    });
    let run = fixture.fail(6);
    fixture.action(Action::Repair {
        task: 6,
        reason: "Repair retained failure".into(),
    });
    let runtime = Runtime::new().unwrap();
    let mut control = fixture.control(&runtime);
    let before = fixture.bytes();
    let tree = control.work_tree();
    assert_eq!(
        tree.rows
            .iter()
            .filter_map(|row| row.task)
            .collect::<Vec<_>>(),
        vec![1, 6, 7, 2, 3, 4, 5]
    );
    assert_eq!(
        tree.rows
            .iter()
            .find(|row| row.id == NodeId::Task(7))
            .unwrap()
            .parent,
        Some(NodeId::Task(6))
    );
    let join = tree
        .rows
        .iter()
        .find(|row| row.id == NodeId::Task(4))
        .unwrap();
    assert_eq!(join.parent, Some(NodeId::Plan(1)));
    assert!(join.detail.contains("#2") && join.detail.contains("#3"));
    assert!(control.collapse_work_node()); // Hide the repair descendants of #1.
    assert!(control.collapse_work_node()); // Focus its Plan.
    assert!(control.collapse_work_node()); // Hide that Plan.
    assert!(control.selected_task().is_none());
    command(&mut control, &runtime, "/evidence 6");
    assert_eq!(control.selected_task().unwrap().id, 6);
    assert_eq!(control.evidence.as_ref().unwrap().task, 6);
    let text = control
        .evidence
        .as_ref()
        .unwrap()
        .lines()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains(&run), "{text}");
    assert!(text.contains("Exact retained failure for task #6"));
    assert_eq!(control.view_preferences().selected, Some(6));
    assert_eq!(
        control
            .work_tree()
            .rows
            .iter()
            .filter_map(|row| row.task)
            .collect::<Vec<_>>(),
        vec![1, 6, 7, 2, 3, 4, 5]
    );
    assert_eq!(fixture.bytes(), before);
    assert!(control.newly_observed_receipts().is_empty());
}

#[test]
fn pending_evidence_cannot_replace_a_later_view_choice() {
    let fixture = Fixture::new();
    fixture.fail(1);
    let runtime = Runtime::new().unwrap();
    let mut control = fixture.control(&runtime);
    command(&mut control, &runtime, "/tasks #5");
    let before = fixture.bytes();
    control.command(&runtime, "/evidence 1", "fixture").unwrap();
    control.select_task(false);
    assert_eq!(control.selected_task().unwrap().id, 5); // Preserve pending-operation guard.
    assert!(!control.collapse_work_node());
    control.command(&runtime, "/chat", "fixture").unwrap();
    settle(&mut control);
    assert!(!control.visible);
    assert!(control.evidence.is_none());
    assert_eq!(control.selected_task().unwrap().id, 5);
    assert_eq!(fixture.bytes(), before);
    assert!(control.newly_observed_receipts().is_empty());

    control.command(&runtime, "/evidence 1", "fixture").unwrap();
    control.set_visible(false); // F2 may hide and reopen before the read completes.
    control.set_visible(true);
    settle(&mut control);
    assert!(control.visible);
    assert!(control.evidence.is_none());
    assert_eq!(control.selected_task().unwrap().id, 5);
    assert_eq!(fixture.bytes(), before);
}

#[test]
fn missing_saved_identity_never_defaults_and_cache_tracks_scope_and_filters() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut control = fixture.control(&runtime);
    control
        .restore_view(TaskView {
            visible: true,
            selected: Some(999),
            query: String::new(),
        })
        .unwrap();
    assert!(control.selected_task().is_none());
    assert_eq!(control.view_preferences().selected, Some(999));
    let before = control.work_tree();
    control.scope_status.blocked = !control.scope_status.blocked;
    assert!(!Arc::ptr_eq(&before, &control.work_tree()));
    let before = control.work_tree();
    command(&mut control, &runtime, "/tasks Left branch");
    assert!(!Arc::ptr_eq(&before, &control.work_tree()));
    assert!(control.selected_task().is_none());
    assert_eq!(control.view_preferences().selected, Some(999));
}
