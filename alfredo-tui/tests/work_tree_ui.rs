//! Render-only fixtures: no executable receipt proofs or task mutations.
use alfredo_tui::{
    conversations::TaskView,
    model::App,
    planner::{Plan, Step},
    task_control::TaskControl,
    tasks::{Action, Receipt, Request, Snapshot, Task, TaskRun, TaskStatus, TaskStore, WorkPolicy},
    ui,
};
use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    control: TaskControl,
    app: App,
}

impl Fixture {
    fn new(tasks: Vec<Task>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-work-tree-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let store = TaskStore::new(&root.join("state"), &workspace, "Tree fixture").unwrap();
        let mut control = TaskControl::new(store);
        control.visible = true;
        control.snapshot = Some(Snapshot {
            schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
            workspace,
            mission: "Tree fixture".into(),
            revision: 0,
            tasks,
            receipts: vec![],
        });
        let mut app = App::new("conversation-model".into());
        app.sessions[0].insert("keep draft");
        Self { root, control, app }
    }

    fn select(&mut self, task: u64) {
        self.control
            .restore_view(TaskView {
                visible: true,
                selected: Some(task),
                query: String::new(),
            })
            .unwrap();
    }

    fn render(&self, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &self.app, &self.control))
            .unwrap();
        terminal.backend().buffer().clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn task(
    id: u64,
    title: &str,
    status: TaskStatus,
    dependencies: Vec<u64>,
    repair_of: Option<u64>,
) -> Task {
    Task {
        id,
        title: title.into(),
        model: "exact-worker-model".into(),
        dependencies,
        status: status.clone(),
        policy: None,
        repair_of,
        run: matches!(
            status,
            TaskStatus::Running | TaskStatus::Rejected | TaskStatus::Accepted
        )
        .then(|| TaskRun {
            id: format!("task-{id}-run-1"),
            baseline: "a".repeat(40),
            inputs: vec![],
            evidence_sha256: (status != TaskStatus::Running).then(|| "b".repeat(64)),
            detail: "Recorded outcome".into(),
        }),
    }
}

fn receipt(snapshot: &mut Snapshot, task: u64, action: Action) {
    let previous = snapshot.revision;
    snapshot.revision += 1;
    snapshot.receipts.push(Receipt {
        task,
        revision: snapshot.revision,
        request: Request {
            correlation: format!("saved-r{}", snapshot.revision),
            expected_revision: previous,
            action,
        },
    });
}

fn hierarchy() -> Fixture {
    let mut fixture = Fixture::new(vec![
        task(1, "Original parser", TaskStatus::Rejected, vec![], None),
        task(2, "Read inputs", TaskStatus::Approved, vec![1], None),
        task(3, "Write outputs", TaskStatus::Proposed, vec![1], None),
        task(
            4,
            "Check integration",
            TaskStatus::Proposed,
            vec![2, 3],
            None,
        ),
        task(5, "Repair parser", TaskStatus::Accepted, vec![], Some(1)),
        task(6, "Manual audit", TaskStatus::Proposed, vec![], None),
    ]);
    let snapshot = fixture.control.snapshot.as_mut().unwrap();
    let steps = snapshot
        .tasks
        .iter()
        .take(4)
        .map(|task| Step {
            title: task.title.clone(),
            acceptance: vec!["Behavior retained".into()],
            model: task.model.clone(),
            dependencies: task.dependencies.clone(),
            policy: WorkPolicy {
                files: vec!["parser.rs".into()],
                check: vec!["true".into()],
            },
        })
        .collect();
    receipt(
        snapshot,
        1,
        Action::Plan {
            plan: Plan {
                prompt: "Build a parser".into(),
                planner: "recorded-planner-model".into(),
                tasks: steps,
                context: None,
                scope: None,
                architecture: None,
            },
        },
    );
    receipt(
        snapshot,
        5,
        Action::Repair {
            task: 1,
            reason: "Retain Unicode".into(),
        },
    );
    receipt(snapshot, 5, Action::ResolveRepair { task: 5 });
    fixture.select(1);
    fixture
}

fn region(buffer: &Buffer, x: u16, y: u16, width: u16, height: u16) -> String {
    (y..y + height)
        .map(|row| {
            (x..x + width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn all(buffer: &Buffer) -> String {
    region(buffer, 0, 0, buffer.area.width, buffer.area.height)
}

#[test]
fn hierarchy_names_task_counts_and_renders_dependency_edges_once() {
    let fixture = hierarchy();
    let buffer = fixture.render(140, 40);
    let tree = region(&buffer, 0, 2, 40, 33);
    assert!(tree.contains("Mission Work · 6/6 tasks"), "{tree}");
    assert!(tree.contains("Plan r1 · 5 tasks"), "{tree}");
    assert!(tree.contains("Build a parser"), "{tree}");
    assert!(tree.contains("Manual tasks · 1 task"), "{tree}");
    for status in [
        "#1 Rejected",
        "#2 Approved",
        "#3 Proposed",
        "#4 Proposed",
        "#5 Accepted",
        "#6 Proposed",
    ] {
        assert_eq!(tree.matches(status).count(), 1, "{status}: {tree}");
    }
    assert_eq!(tree.matches("Depends on #1").count(), 2, "{tree}");
    assert!(tree.contains("Depends on #2, #3"), "{tree}");
    let original = tree
        .lines()
        .find(|line| line.contains("#1 Rejected"))
        .unwrap();
    let repair = tree
        .lines()
        .find(|line| line.contains("#5 Accepted"))
        .unwrap();
    let column =
        |line: &str| unicode_width::UnicodeWidthStr::width(line.split('#').next().unwrap());
    assert!(column(repair) > column(original), "{tree}");
    assert!(tree.contains("↳ #5"), "{tree}");
    assert!(all(&buffer).contains("Resolved by accepted repair #5"));
    assert!(all(&buffer).contains("keep draft"));
}

#[test]
fn group_selection_removes_stale_task_model_evidence_and_actions() {
    let mut fixture = hierarchy();
    assert!(all(&fixture.render(140, 40)).contains("Model: exact-worker-model"));
    fixture.control.select_task(false);
    assert!(fixture.control.selected_task().is_none());
    let buffer = fixture.render(140, 40);
    let inspector = region(&buffer, 40, 2, 100, 33);
    assert!(inspector.contains("Work group"), "{inspector}");
    assert!(inspector.contains("5 tasks in this group"), "{inspector}");
    assert!(inspector.contains("Select a task"), "{inspector}");
    assert!(inspector.contains("no task action target"), "{inspector}");
    assert!(inspector.contains("0 local workers"), "{inspector}");
    for stale in [
        "exact-worker-model",
        "task-1-run-1",
        "Evidence recorded",
        "Task actions",
        "/approve",
        "/run",
        "/repair",
    ] {
        assert!(!inspector.contains(stale), "stale {stale}: {inspector}");
    }
    assert!(fixture.control.collapse_work_node());
    let collapsed = all(&fixture.render(140, 40));
    assert!(collapsed.contains("▸ Plan r1 · 5 tasks"));
    assert!(!collapsed.contains("#5 Accepted"));
    assert!(collapsed.contains("Mission Work · 6/6 tasks"));
}

#[test]
fn minimum_size_keeps_selected_tree_row_composer_and_scrollable_exact_inspector() {
    let mut fixture = hierarchy();
    fixture.select(5);
    let saved = serde_json::to_value(&fixture.app.sessions[0]).unwrap();
    let mut observed = String::new();
    for _ in 0..100 {
        let buffer = fixture.render(32, 10);
        let text = all(&buffer);
        assert!(text.contains("#5 Accepted"), "{text}");
        assert!(text.contains("Prompt"), "{text}");
        assert!(text.contains("keep draft"), "{text}");
        assert!(text.contains("Alt+←/→"), "{text}");
        observed.push_str(&region(&buffer, 0, 3, 32, 2));
        observed.push('\n');
        fixture.control.scroll_rows(1);
    }
    for expected in [
        "Repair parser",
        "task-5-run-1",
        "Repair of #1",
        "Evidence recorded",
        "exact-worker-model",
    ] {
        assert!(
            observed.contains(expected),
            "missing {expected}: {observed}"
        );
    }
    assert!(!observed.contains("task-1-run-1"));
    assert_eq!(
        serde_json::to_value(&fixture.app.sessions[0]).unwrap(),
        saved
    );
    fixture.select(1);
    fixture.control.select_task(false);
    let mut group_details = String::new();
    for _ in 0..20 {
        let buffer = fixture.render(32, 10);
        let text = all(&buffer);
        assert!(text.contains("Plan r1 · 5 tasks"), "{text}");
        assert!(text.contains("keep draft"), "{text}");
        group_details.push_str(&region(&buffer, 0, 3, 32, 2));
        fixture.control.scroll_rows(1);
    }
    assert!(group_details.contains("Select a task"), "{group_details}");
}

#[test]
fn running_task_exposes_actual_model_run_and_missing_observation_without_claiming_liveness() {
    let mut fixture = Fixture::new(vec![task(
        1,
        "Inspect the source",
        TaskStatus::Running,
        vec![],
        None,
    )]);
    receipt(
        fixture.control.snapshot.as_mut().unwrap(),
        1,
        Action::Propose {
            title: "Inspect the source".into(),
            model: "exact-worker-model".into(),
            dependencies: vec![],
        },
    );
    fixture.select(1);
    let text = all(&fixture.render(160, 40));
    for expected in [
        "#1 Running",
        "Model: exact-worker-model",
        "task-1-run-1",
        "Current observation unavailable",
        "recorded run does not prove a live worker",
        "Recent saved activity",
        "saved-r1",
        "Evidence: no completed run evidence recorded",
        "0 local workers",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    fixture.control.run_observations.insert(
        1,
        "Owner probe stale; /refresh for a new observation".into(),
    );
    let text = all(&fixture.render(160, 40));
    assert!(text.contains("Owner probe stale"));
    assert!(!text.contains("Current observation unavailable"));
}

#[test]
fn empty_filtered_unavailable_and_untrusted_labels_remain_readable() {
    let mut fixture = Fixture::new(vec![]);
    for size in [(140, 30), (32, 10)] {
        let text = all(&fixture.render(size.0, size.1));
        assert!(text.contains("No tasks proposed"), "{text}");
        assert!(text.contains("keep draft"), "{text}");
    }
    fixture.control.snapshot = None;
    fixture.control.notice = "Read failed · /refresh".into();
    let text = all(&fixture.render(32, 10));
    assert!(text.contains("Task state unavailable"), "{text}");
    assert!(text.contains("Read failed"), "{text}");
    let mut fixture = Fixture::new(vec![task(
        1,
        "Safe\u{1b}[2J\u{7} title\nnext",
        TaskStatus::Proposed,
        vec![],
        None,
    )]);
    let text = all(&fixture.render(140, 40));
    assert!(!text.contains('\u{1b}') && !text.contains('\u{7}'));
    fixture.control.task_query = "missing query".into();
    let text = all(&fixture.render(140, 40));
    assert!(text.contains("No matching tasks"), "{text}");
    assert!(text.contains("Showing 0 / 1 tasks"), "{text}");
    assert!(!text.contains("Task actions"), "{text}");
}

#[test]
fn focused_evidence_at_96_columns_keeps_check_result_in_the_initial_view() {
    let mut fixture = Fixture::new(vec![task(
        1,
        "Persisted fix",
        TaskStatus::ReviewReady,
        vec![],
        None,
    )]);
    fixture.select(1);
    let evidence = serde_json::json!({
        "run": "task-1-run-5",
        "baseline": "a".repeat(40),
        "candidate_commit": "b".repeat(40),
        "status": TaskStatus::ReviewReady,
        "detail": "Approved check passed; changes await human review",
        "patch": "diff --git a/answer.py b/answer.py\n-VALUE = 0\n+VALUE = 42",
        "agent": {
            "agent": "task-1-run-5",
            "model": "exact-worker-model",
            "reason": "New task",
            "continued_from": null,
            "transcript_sha256": null
        },
        "model_metrics": {
            "load_duration": 500_000_000u64,
            "eval_duration": 2_000_000_000u64,
            "eval_count": 40
        },
        "generation": {"thinking": "off", "num_predict": 4096, "temperature": 0},
        "check": null
    });
    fixture.control.evidence = Some(
        alfredo_tui::review::View::from_verified(1, &evidence.to_string())
            .unwrap()
            .with_acceptance(&[]),
    );
    let text = all(&fixture.render(96, 24));
    for expected in [
        "Verified run evidence",
        "task-1-run-5",
        "Requested generation: thinking off",
        "Check result",
        "keep draft",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert_eq!(fixture.control.scroll, 0);
}
