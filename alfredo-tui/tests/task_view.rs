//! Projection fixtures deliberately omit executable receipt proofs; they never write task state.
use alfredo_tui::{
    assessment::{Decision, FailureKind, Outcome},
    planner::{Plan, Step},
    task_control::TaskControl,
    task_view::WorkStatus,
    tasks::{Action, Receipt, Request, Snapshot, Task, TaskRun, TaskStatus, TaskStore, WorkPolicy},
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    control: TaskControl,
}
impl Fixture {
    fn new(tasks: Vec<Task>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-work-status-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let store = TaskStore::new(&root.join("state"), &workspace, "mission").unwrap();
        let mut control = TaskControl::new(store);
        control.snapshot = Some(Snapshot {
            schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
            workspace,
            mission: "mission".into(),
            revision: 0,
            tasks,
            receipts: vec![],
        });
        Self { root, control }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn task(id: u64, status: TaskStatus, parent: Option<u64>, executed: bool) -> Task {
    Task {
        id,
        title: format!("Task {id}"),
        model: "fixture".into(),
        dependencies: vec![],
        status,
        policy: None,
        repair_of: parent,
        run: executed.then(|| TaskRun {
            id: format!("task-{id}-run-1"),
            baseline: "a".repeat(40),
            inputs: vec![],
            evidence_sha256: Some("b".repeat(64)),
            detail: "Recorded fixture outcome".into(),
        }),
    }
}
fn counts(status: &WorkStatus) -> [usize; 7] {
    [
        status.workers,
        status.recorded,
        status.review,
        status.held,
        status.repair,
        status.architect,
        status.resolve,
    ]
}
fn receipt(snapshot: &mut Snapshot, task: u64, action: Action) {
    let previous = snapshot.revision;
    snapshot.revision += 1;
    snapshot.receipts.push(Receipt {
        task,
        revision: snapshot.revision,
        request: Request {
            correlation: format!("projection-{}", snapshot.revision),
            expected_revision: previous,
            action,
        },
    });
}
fn architecture_decision() -> Decision {
    Decision {
        outcome: Outcome::NeedsRepair,
        reason: "Architecture needs revision".into(),
        criteria: vec![],
        limitations: vec![],
        risk: None,
        failure: Some(FailureKind::Architecture),
    }
}
fn architect_fixture() -> Fixture {
    let mut fixture = Fixture::new(vec![
        task(1, TaskStatus::Rejected, None, true),
        task(2, TaskStatus::Rejected, Some(1), true),
    ]);
    let snapshot = fixture.control.snapshot.as_mut().unwrap();
    receipt(
        snapshot,
        2,
        Action::ReviewArchitecture {
            task: 1,
            decision: architecture_decision(),
        },
    );
    receipt(
        snapshot,
        2,
        Action::ReviewArchitecture {
            task: 2,
            decision: architecture_decision(),
        },
    );
    assert!(snapshot.architecture_required(2));
    fixture
}
fn adopt(snapshot: &mut Snapshot, status: TaskStatus) {
    let origin = snapshot.architecture_origin(2).unwrap();
    snapshot.tasks.push(task(3, status, Some(2), false));
    receipt(
        snapshot,
        3,
        Action::Plan {
            plan: Plan {
                prompt: "Revise architecture".into(),
                planner: "fixture".into(),
                context: None,
                scope: None,
                architecture: Some(origin),
                tasks: vec![Step {
                    title: "New architecture".into(),
                    acceptance: vec!["Required behavior".into()],
                    model: "fixture".into(),
                    dependencies: vec![],
                    policy: WorkPolicy {
                        files: vec!["source.rs".into()],
                        check: vec!["true".into()],
                    },
                }],
            },
        },
    );
}

#[test]
fn work_status_separates_local_workers_from_recorded_runs_and_actionable_results() {
    let mut fixture = Fixture::new(vec![
        task(1, TaskStatus::Running, None, true),
        task(2, TaskStatus::Running, None, true),
        task(3, TaskStatus::ReviewReady, None, true),
        task(4, TaskStatus::NeedsHumanReview, None, true),
        task(5, TaskStatus::Failed, None, true),
        task(6, TaskStatus::Rejected, None, true),
        task(7, TaskStatus::Cancelled, None, true),
        task(8, TaskStatus::Cancelled, None, false),
        task(9, TaskStatus::Proposed, None, false),
        task(10, TaskStatus::Approved, None, false),
        task(11, TaskStatus::Accepted, None, true),
    ]);
    fixture
        .control
        .workers
        .insert(1, Arc::new(AtomicBool::new(false)));
    // A pending cancellation is still locally supervised until worker completion.
    fixture
        .control
        .workers
        .insert(99, Arc::new(AtomicBool::new(true)));
    let before = serde_json::to_vec(fixture.control.snapshot.as_ref().unwrap()).unwrap();
    let status = fixture.control.work_status();
    assert_eq!(counts(&status), [2, 1, 1, 1, 3, 0, 0]);
    assert_eq!(status.attention(), 6);
    assert_eq!(
        serde_json::to_vec(fixture.control.snapshot.as_ref().unwrap()).unwrap(),
        before
    );
    fixture.control.workers.remove(&1);
    assert_eq!(
        counts(&fixture.control.work_status()),
        [1, 2, 1, 1, 3, 0, 0]
    );
    fixture.control.snapshot = None;
    assert_eq!(
        counts(&fixture.control.work_status()),
        [1, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn work_status_counts_leaf_repairs_and_preserves_historical_human_holds() {
    let mut fixture = Fixture::new(vec![
        task(1, TaskStatus::Rejected, None, true),
        task(2, TaskStatus::Rejected, Some(1), true),
        task(3, TaskStatus::ReviewReady, Some(2), true),
        task(4, TaskStatus::NeedsHumanReview, None, true),
        task(5, TaskStatus::Proposed, Some(4), false),
        task(6, TaskStatus::Failed, None, true),
        task(7, TaskStatus::Accepted, Some(6), true),
    ]);
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 1, 1, 1, 0, 1]
    );
    let snapshot = fixture.control.snapshot.as_mut().unwrap();
    receipt(snapshot, 7, Action::ResolveRepair { task: 7 });
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 1, 1, 1, 0, 0]
    );
    fixture.control.snapshot.as_mut().unwrap().tasks[2].status = TaskStatus::Accepted;
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 0, 1, 1, 0, 1]
    );
    let snapshot = fixture.control.snapshot.as_mut().unwrap();
    receipt(snapshot, 3, Action::ResolveRepair { task: 3 });
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 0, 1, 1, 0, 0]
    );
}

#[test]
fn work_status_tracks_architect_gate_through_adoption_and_unstarted_cancellation() {
    let mut fixture = architect_fixture();
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 0, 0, 0, 1, 0]
    );
    adopt(
        fixture.control.snapshot.as_mut().unwrap(),
        TaskStatus::Proposed,
    );
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 0, 0, 1, 0, 0]
    );
    let snapshot = fixture.control.snapshot.as_mut().unwrap();
    assert!(snapshot.architecture_obsolete(1));
    assert!(snapshot.architecture_obsolete(2));
    snapshot.tasks[2].status = TaskStatus::Cancelled;
    assert!(snapshot.architecture_required(2));
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 0, 0, 0, 1, 0]
    );
    let snapshot = fixture.control.snapshot.as_mut().unwrap();
    snapshot.tasks[2].status = TaskStatus::Accepted;
    snapshot.tasks[2].run = Some(TaskRun {
        id: "task-3-run-1".into(),
        baseline: "a".repeat(40),
        inputs: vec![],
        evidence_sha256: Some("b".repeat(64)),
        detail: "Accepted".into(),
    });
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 0, 0, 0, 0, 1]
    );
    receipt(
        fixture.control.snapshot.as_mut().unwrap(),
        3,
        Action::ResolveRepair { task: 3 },
    );
    assert_eq!(counts(&fixture.control.work_status()), [0; 7]);
}

#[test]
fn cancelled_unstarted_repair_restores_parent_attention_but_executed_child_owns_it() {
    let mut fixture = Fixture::new(vec![
        task(1, TaskStatus::Failed, None, true),
        task(2, TaskStatus::Cancelled, Some(1), false),
    ]);
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 0, 0, 1, 0, 0]
    );
    fixture.control.snapshot.as_mut().unwrap().tasks[1] =
        task(2, TaskStatus::Cancelled, Some(1), true);
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 0, 0, 1, 0, 0]
    );
    fixture.control.snapshot.as_mut().unwrap().tasks[1] =
        task(2, TaskStatus::Proposed, Some(1), false);
    assert_eq!(
        counts(&fixture.control.work_status()),
        [0, 0, 0, 0, 1, 0, 0]
    );
    assert_eq!(fixture.control.work_status().attention(), 1);
    fixture.control.snapshot.as_mut().unwrap().tasks[1].status = TaskStatus::Approved;
    assert_eq!(counts(&fixture.control.work_status()), [0; 7]);
}

#[test]
fn work_status_copy_distinguishes_unavailable_idle_and_recorded_runs_at_narrow_width() {
    let mut fixture = Fixture::new(vec![]);
    let idle = fixture.control.work_status();
    assert!(idle.loaded);
    assert!(idle.concise(120).contains("no pending review"));
    fixture.control.snapshot = None;
    let unavailable = fixture.control.work_status();
    assert!(!unavailable.loaded);
    assert!(unavailable.concise(120).contains("state unavailable"));
    assert!(!unavailable.concise(120).contains("no pending review"));
    let status = WorkStatus {
        workers: 2,
        recorded: 1,
        review: 3,
        held: 1,
        repair: 2,
        architect: 1,
        resolve: 1,
        loaded: true,
    };
    assert_eq!(status.attention(), 9);
    let narrow = status.concise(32);
    assert!(narrow.contains("2 work") && narrow.contains("9 alerts"));
    assert!(unicode_width::UnicodeWidthStr::width(narrow.as_str()) <= 32);
    let wide = status.concise(160);
    for expected in [
        "2 local",
        "1 recorded run",
        "3 review",
        "1 held",
        "2 repair",
        "1 Architect",
        "1 resolve",
    ] {
        assert!(wide.contains(expected), "Missing {expected}: {wide}");
    }
    assert!(!wide.contains("running"));
}

#[test]
#[ignore = "explicit bounded work-status and chat redraw performance measurement"]
fn measure_deep_repair_history_chat_redraws() {
    use std::{hint::black_box, time::Instant};
    let mut fixture = Fixture::new(vec![]);
    let snapshot = fixture.control.snapshot.as_mut().unwrap();
    let policy = WorkPolicy {
        files: vec!["source.rs".into()],
        check: vec!["/bin/true".into()],
    };
    // This is a projection-only history at the task bound. Each task has a
    // proposal, 11 policy edits, approval, start, finish and (except the leaf)
    // rejection. Digests are synthetic; no store or execution proof is claimed.
    for id in 1..=256_u64 {
        let title = if id == 1 {
            "Initial calculation".into()
        } else {
            format!("Repair #{}: Revise calculation", id - 1)
        };
        let action = if id == 1 {
            Action::Propose {
                title: title.clone(),
                model: "fixture".into(),
                dependencies: vec![],
            }
        } else {
            Action::Repair {
                task: id - 1,
                reason: "Revise calculation".into(),
            }
        };
        receipt(snapshot, id, action);
        for _ in 0..11 {
            receipt(
                snapshot,
                id,
                Action::Permit {
                    task: id,
                    policy: policy.clone(),
                },
            );
        }
        receipt(snapshot, id, Action::Approve { task: id });
        let run = format!("task-{id}-run-{}", snapshot.revision + 1);
        receipt(
            snapshot,
            id,
            Action::Start {
                task: id,
                baseline: "a".repeat(40),
                inputs: vec![],
            },
        );
        receipt(
            snapshot,
            id,
            Action::Finish {
                task: id,
                run: run.clone(),
                status: TaskStatus::ReviewReady,
                evidence_sha256: "b".repeat(64),
                detail: "Retained check result".into(),
            },
        );
        if id != 256 {
            receipt(
                snapshot,
                id,
                Action::Review {
                    task: id,
                    accept: false,
                },
            );
        }
        let mut projected = task(
            id,
            if id == 256 {
                TaskStatus::ReviewReady
            } else {
                TaskStatus::Rejected
            },
            (id > 1).then_some(id - 1),
            true,
        );
        projected.title = title;
        projected.policy = Some(policy.clone());
        projected.run.as_mut().unwrap().id = run;
        projected.run.as_mut().unwrap().detail = "Retained check result".into();
        snapshot.tasks.push(projected);
    }
    let task_count = snapshot.tasks.len();
    let receipt_count = snapshot.receipts.len();
    let snapshot_bytes = serde_json::to_vec(snapshot).unwrap().len();
    assert_eq!(task_count, 256);
    assert_eq!(receipt_count, 4095);
    assert!(snapshot_bytes < 4 * 1024 * 1024);
    let mut app = alfredo_tui::model::App::new("fixture".into());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let cold = Instant::now();
    terminal
        .draw(|frame| {
            alfredo_tui::ui::draw_with_tasks(frame, black_box(&app), black_box(&fixture.control))
        })
        .unwrap();
    black_box(terminal.backend().buffer());
    let cold_us = cold.elapsed().as_micros();
    let warm = Instant::now();
    for _ in 0..20 {
        app.sessions[0].insert("x");
        terminal
            .draw(|frame| {
                alfredo_tui::ui::draw_with_tasks(
                    frame,
                    black_box(&app),
                    black_box(&fixture.control),
                )
            })
            .unwrap();
        black_box(terminal.backend().buffer());
    }
    let warm_us = warm.elapsed().as_micros();
    let direct = Instant::now();
    let status = black_box(black_box(&fixture.control).work_status());
    let direct_us = direct.elapsed().as_micros();
    assert_eq!(counts(&status), [0, 0, 1, 0, 0, 0, 0]);
    println!(
        "{}",
        serde_json::json!({
            "fixture": "256-task-4095-receipt-repair-chain", "tasks": task_count,
            "receipts": receipt_count, "snapshot_bytes": snapshot_bytes,
            "viewport": [100, 24], "cold_draw_us": cold_us,
            "warm_draws": 20, "warm_total_us": warm_us,
            "direct_work_status_us": direct_us, "status_counts": counts(&status),
        })
    );
}
