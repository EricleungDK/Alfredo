use alfredo_tui::{
    task_control::parse,
    tasks::{Action, Refusal, Request, TaskStatus, TaskStore},
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Barrier,
    },
};

static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-tasks-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("workspace")).unwrap();
        Self { root }
    }
    fn store(&self, mission: &str) -> TaskStore {
        TaskStore::new(
            &self.root.join("state"),
            &self.root.join("workspace"),
            mission,
        )
        .unwrap()
    }
    fn file(&self) -> PathBuf {
        fs::read_dir(self.root.join("state/rust-tasks-v1"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("tasks.json")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn proposal(correlation: &str, revision: u64, dependencies: Vec<u64>) -> Request {
    Request {
        correlation: correlation.into(),
        expected_revision: revision,
        action: Action::Propose {
            title: "Implement the requested regression 🦀".into(),
            model: "fixture".into(),
            dependencies,
        },
    }
}

#[test]
fn committed_tasks_and_approvals_restore_and_exact_retry_does_not_duplicate() {
    let fixture = Fixture::new();
    let request = proposal("create", 0, vec![]);
    let (snapshot, receipt) = fixture.store("mission").transact(request.clone()).unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Proposed);
    let approval = Request {
        correlation: "approve".into(),
        expected_revision: 1,
        action: Action::Approve { task: 1 },
    };
    fixture.store("mission").transact(approval).unwrap();
    let (restored, replay) = fixture.store("mission").transact(request).unwrap();
    assert_eq!(replay, receipt);
    assert_eq!(restored.tasks.len(), 1);
    assert_eq!(restored.tasks[0].status, TaskStatus::Approved);
    assert_eq!(restored.revision, 2);
}

#[test]
fn stale_or_conflicting_requests_leave_committed_bytes_unchanged() {
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    store.transact(proposal("first", 0, vec![])).unwrap();
    let before = fs::read(fixture.file()).unwrap();
    assert!(store
        .transact(proposal("second", 0, vec![]))
        .unwrap_err()
        .contains("changed"));
    assert!(store
        .transact(proposal("first", 1, vec![]))
        .unwrap_err()
        .contains("Correlation"));
    assert_eq!(fs::read(fixture.file()).unwrap(), before);
}

#[test]
fn dependency_graph_rejects_missing_self_and_duplicate_edges() {
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    store.transact(proposal("first", 0, vec![])).unwrap();
    for dependencies in [vec![0], vec![2], vec![99], vec![1, 1]] {
        assert!(store
            .transact(proposal("invalid", 1, dependencies))
            .is_err());
        assert_eq!(store.snapshot().unwrap().revision, 1);
    }
    let (snapshot, _) = store.transact(proposal("dependent", 1, vec![1])).unwrap();
    assert_eq!(snapshot.tasks[1].dependencies, vec![1]);
}

#[test]
fn cancelled_task_cannot_be_approved_after_restart() {
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    store.transact(proposal("first", 0, vec![])).unwrap();
    store
        .transact(Request {
            correlation: "cancel".into(),
            expected_revision: 1,
            action: Action::Cancel { task: 1 },
        })
        .unwrap();
    assert!(fixture
        .store("mission")
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: 2,
            action: Action::Approve { task: 1 }
        })
        .is_err());
    assert_eq!(
        store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Cancelled
    );
}

#[test]
fn corrupt_or_fabricated_approval_never_overwrites_original_state() {
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    store.transact(proposal("first", 0, vec![])).unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.file()).unwrap()).unwrap();
    let mut forged = original.clone();
    forged["tasks"][0]["status"] = "approved".into();
    let mut future = original.clone();
    future["schema_version"] = 99.into();
    for bytes in [
        b"{broken".to_vec(),
        serde_json::to_vec(&forged).unwrap(),
        serde_json::to_vec(&future).unwrap(),
        vec![b'x'; 4 * 1024 * 1024 + 1],
    ] {
        fs::write(fixture.file(), &bytes).unwrap();
        assert!(store.snapshot().is_err());
        assert!(store.transact(proposal("second", 1, vec![])).is_err());
        assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
    }
}

#[test]
fn missions_and_workspaces_do_not_share_task_authority() {
    let fixture = Fixture::new();
    fixture
        .store("first")
        .transact(proposal("create", 0, vec![]))
        .unwrap();
    assert!(fixture.store("second").snapshot().unwrap().tasks.is_empty());
    fs::create_dir(fixture.root.join("other-workspace")).unwrap();
    let other = TaskStore::new(
        &fixture.root.join("state"),
        &fixture.root.join("other-workspace"),
        "first",
    )
    .unwrap();
    assert!(other.snapshot().unwrap().tasks.is_empty());
}

#[test]
fn concurrent_exact_submissions_have_one_effect_and_release_the_lock() {
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    store.snapshot().unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let jobs: Vec<_> = (0..8)
        .map(|_| {
            let barrier = barrier.clone();
            let store = store.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.transact(proposal("same", 0, vec![]))
            })
        })
        .collect();
    let outcomes: Vec<_> = jobs.into_iter().map(|job| job.join().unwrap()).collect();
    assert!(outcomes.iter().any(Result::is_ok));
    let (snapshot, receipt) = store.transact(proposal("same", 0, vec![])).unwrap();
    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.revision, 1);
    assert_eq!(receipt.revision, 1);
}

#[test]
fn state_cannot_be_created_inside_the_coding_workspace() {
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    assert!(TaskStore::new(&workspace.join("state"), &workspace, "mission").is_err());
    assert!(!workspace.join("state").exists());
}

#[cfg(unix)]
#[test]
fn symlink_store_does_not_read_or_replace_the_link_target() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    store.transact(proposal("first", 0, vec![])).unwrap();
    fs::remove_file(fixture.file()).unwrap();
    let target = fixture.root.join("private");
    fs::write(&target, "untouched").unwrap();
    symlink(&target, fixture.file()).unwrap();
    assert!(store.snapshot().is_err());
    assert!(store.transact(proposal("new", 0, vec![])).is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "untouched");
}

#[test]
fn terminal_commands_are_typed_and_do_not_infer_model_authority() {
    assert!(
        matches!(parse("/after 1,2 repair tests", "worker").unwrap(), Action::Propose { dependencies, .. } if dependencies == [1, 2])
    );
    assert_eq!(
        parse("/approve 1", "worker").unwrap(),
        Action::Approve { task: 1 }
    );
    for text in [
        "model says approved",
        "/approve 1; touch /tmp/file",
        "/after foo task",
        "/task ",
    ] {
        assert!(parse(text, "worker").is_err());
    }
}

#[test]
fn keyboard_task_selection_targets_one_task_and_survives_background_refresh() {
    use alfredo_tui::{model::App, task_control::TaskControl, ui};
    use ratatui::{backend::TestBackend, Terminal};
    use std::time::{Duration, Instant};
    let fixture = Fixture::new();
    let store = fixture.store("navigation");
    for id in 0..30 {
        store
            .transact(proposal(&format!("task-{id}"), id, vec![]))
            .unwrap();
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(store.clone());
    control.snapshot = Some(store.snapshot().unwrap());
    control.visible = true;
    assert_eq!(control.selected_task().unwrap().id, 1);
    control.select_task(false);
    assert!(control.selected_task().is_none()); // The Manual group is not a task target.
    assert!(control.command(&runtime, "/approve", "fixture").is_err());
    control.select_task(false);
    assert_eq!(control.selected_task().unwrap().id, 30);
    control.command(&runtime, "/approve", "fixture").unwrap();
    control.select_task(true); // A pending request keeps its target visible.
    assert_eq!(control.selected_task().unwrap().id, 30);
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        control.selected_task().unwrap().status,
        TaskStatus::Approved
    );
    assert_eq!(
        store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Proposed
    );
    store
        .transact(proposal("background-new-task", 31, vec![]))
        .unwrap();
    control.refresh(&runtime);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(control.selected_task().unwrap().id, 30);
    let app = App::new("fixture".into());
    for (width, height) in [(140, 40), (60, 25)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &app, &control))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("#30"));
        assert!(text.contains("Approved"));
        assert!(!text.contains("#1 · Needs approval"));
    }
    control.evidence = Some(
        alfredo_tui::review::View::from_verified(
            30,
            &serde_json::json!({
                "run": "task-30-run-1", "baseline": "a".repeat(40),
                "status": TaskStatus::Failed, "detail": "Fixture interrupted",
                "patch": "", "check": null
            })
            .to_string(),
        )
        .unwrap(),
    );
    control.scroll = 100;
    control.select_task(true);
    assert_eq!(control.selected_task().unwrap().id, 31);
    assert!(control.evidence.is_none());
    assert_eq!(control.scroll, 0);
}

#[test]
fn activity_is_ordered_receipt_truth_and_navigation_never_adds_events() {
    use alfredo_tui::{activity, model::App, task_control::TaskControl, ui};
    use ratatui::{backend::TestBackend, Terminal};
    use std::time::{Duration, Instant};
    let fixture = Fixture::new();
    let store = fixture.store("activity");
    let original = proposal("first-proposal", 0, vec![]);
    store.transact(original.clone()).unwrap();
    store
        .transact(Request {
            correlation: "approval-one".into(),
            expected_revision: 1,
            action: Action::Approve { task: 1 },
        })
        .unwrap();
    for revision in 2..11 {
        store
            .transact(proposal(&format!("proposal-{revision}"), revision, vec![]))
            .unwrap();
    }
    store.transact(original).unwrap();
    let snapshot = store.snapshot().unwrap();
    let entries = activity::entries(&snapshot, "");
    assert_eq!(entries.len(), 11);
    assert_eq!(entries[0].revision, 11);
    assert_eq!(entries.last().unwrap().revision, 1);
    let selected = activity::entries(&snapshot, "#1");
    assert_eq!(selected.len(), 2);
    assert!(selected.iter().all(|entry| entry.task == 1));
    assert_eq!(activity::entries(&snapshot, "APPROVAL-ONE").len(), 1);
    assert_eq!(activity::entries(&snapshot, "approved").len(), 1);
    assert!(store
        .transact(Request {
            correlation: "invalid".into(),
            expected_revision: 11,
            action: Action::Approve { task: 999 }
        })
        .is_err());
    let before = fs::read(fixture.file()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(store.clone());
    control
        .command(&runtime, "/activity #1", "fixture")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let app = App::new("fixture".into());
    for (width, height) in [(140, 40), (60, 25)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &app, &control))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Saved task activity"));
        assert!(text.contains("Task approved"));
        assert!(text.contains("approval-one"));
    }
    let mut terminal = Terminal::new(TestBackend::new(32, 10)).unwrap();
    let mut saw_receipt = false;
    for _ in 0..40 {
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &app, &control))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Prompt"));
        saw_receipt |= text.contains("approval-one");
        control.scroll_rows(1);
    }
    assert!(
        saw_receipt,
        "minimum terminal cannot inspect activity receipt"
    );
    assert_eq!(fs::read(fixture.file()).unwrap(), before);
    assert_eq!(activity::entries(&store.snapshot().unwrap(), "#1").len(), 2);
}

fn capacity_seed(fixture: &Fixture) -> (TaskStore, alfredo_tui::tasks::Snapshot) {
    use alfredo_tui::tasks::WorkPolicy;
    let store = fixture.store("capacity");
    let mut snapshot = store.transact(proposal("seed", 0, vec![])).unwrap().0;
    for action in [
        Action::Permit {
            task: 1,
            policy: WorkPolicy {
                files: vec!["file.txt".into()],
                check: vec!["true".into()],
            },
        },
        Action::Approve { task: 1 },
        Action::Start {
            task: 1,
            baseline: "0".repeat(40),
            inputs: vec![],
        },
        Action::Propose {
            title: "filler".into(),
            model: "fixture".into(),
            dependencies: vec![],
        },
    ] {
        snapshot = store
            .transact(Request {
                correlation: format!("seed-{}", snapshot.revision),
                expected_revision: snapshot.revision,
                action,
            })
            .unwrap()
            .0;
    }
    (store, snapshot)
}

// Construct a replay-valid journal directly so boundary tests avoid thousands of disk writes.
fn capacity_permit(snapshot: &mut alfredo_tui::tasks::Snapshot, quotes: usize) {
    use alfredo_tui::tasks::{Receipt, WorkPolicy};
    let policy = WorkPolicy {
        files: vec!["file.txt".into()],
        check: if quotes == 0 {
            vec!["true".into()]
        } else {
            (0..quotes)
                .step_by(2048)
                .map(|offset| "\"".repeat((quotes - offset).min(2048)))
                .collect()
        },
    };
    policy.validate().unwrap();
    snapshot.tasks[1].policy = Some(policy.clone());
    let request = Request {
        correlation: format!("fill-{}", snapshot.revision),
        expected_revision: snapshot.revision,
        action: Action::Permit { task: 2, policy },
    };
    snapshot.revision += 1;
    snapshot.receipts.push(Receipt {
        request,
        revision: snapshot.revision,
        task: 2,
    });
}

fn capacity_finish(store: &TaskStore, snapshot: &alfredo_tui::tasks::Snapshot) -> Request {
    use alfredo_tui::worker::Evidence;
    use sha2::{Digest, Sha256};
    let run = snapshot.tasks[0].run.as_ref().unwrap();
    let detail = "\"".repeat(1024);
    let bytes = serde_json::to_vec(&Evidence {
        agent: None,
        run: run.id.clone(),
        baseline: run.baseline.clone(),
        status: TaskStatus::Failed,
        detail: detail.clone(),
        patch: String::new(),
        candidate_commit: None,
        model_metrics: None,
        generation: None,
        check: None,
    })
    .unwrap();
    let directory = store.run_directory(&run.id).unwrap();
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("evidence.json"), &bytes).unwrap();
    Request {
        correlation: "\"".repeat(160),
        expected_revision: snapshot.revision,
        action: Action::Finish {
            task: 1,
            run: run.id.clone(),
            status: TaskStatus::Failed,
            evidence_sha256: format!("{:x}", Sha256::digest(bytes)),
            detail,
        },
    }
}

#[test]
fn receipt_capacity_keeps_the_last_slot_for_a_running_result() {
    let fixture = Fixture::new();
    let (store, mut snapshot) = capacity_seed(&fixture);
    while snapshot.receipts.len() < 4095 {
        capacity_permit(&mut snapshot, 0);
    }
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    fs::write(fixture.file(), &bytes).unwrap();
    assert_eq!(
        serde_json::to_vec(&store.snapshot().unwrap()).unwrap(),
        bytes
    );
    let finish = capacity_finish(&store, &snapshot);
    let error = store
        .transact(proposal("unrelated", snapshot.revision, vec![]))
        .unwrap_err();
    assert!(error.contains("reserved"), "{error}");
    assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
    let done = store.transact(finish.clone()).unwrap().0;
    assert_eq!(done.receipts.len(), 4096);
    assert_eq!(done.tasks[0].status, TaskStatus::Failed);
    assert_eq!(store.transact(finish).unwrap().0.revision, done.revision);
}

#[test]
fn byte_capacity_keeps_space_for_escaped_worker_results() {
    let fixture = Fixture::new();
    let (store, mut snapshot) = capacity_seed(&fixture);
    const TARGET: usize = 4 * 1024 * 1024 - 6000;
    loop {
        let mut trial = snapshot.clone();
        capacity_permit(&mut trial, 65536);
        if serde_json::to_vec(&trial).unwrap().len() > TARGET {
            break;
        }
        snapshot = trial;
    }
    let (mut low, mut high) = (1usize, 65536usize);
    while low < high {
        let mid = (low + high).div_ceil(2);
        let mut trial = snapshot.clone();
        capacity_permit(&mut trial, mid);
        if serde_json::to_vec(&trial).unwrap().len() <= TARGET {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    capacity_permit(&mut snapshot, low);
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    assert!(
        (TARGET - 5..=TARGET).contains(&bytes.len()),
        "{}",
        bytes.len()
    );
    fs::write(fixture.file(), &bytes).unwrap();
    assert_eq!(
        serde_json::to_vec(&store.snapshot().unwrap()).unwrap(),
        bytes
    );
    let finish = capacity_finish(&store, &snapshot);
    let request = Request {
        correlation: "unrelated".into(),
        expected_revision: snapshot.revision,
        action: Action::Propose {
            title: "x".repeat(1000),
            model: "fixture".into(),
            dependencies: vec![],
        },
    };
    let error = store.transact(request).unwrap_err();
    assert!(error.contains("reserved"), "{error}");
    assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
    assert_eq!(
        store.transact(finish).unwrap().0.tasks[0].status,
        TaskStatus::Failed
    );
}

#[test]
fn near_full_journal_refuses_a_claim_without_room_for_both_workers() {
    let fixture = Fixture::new();
    let (store, mut snapshot) = capacity_seed(&fixture);
    while snapshot.receipts.len() < 4093 {
        capacity_permit(&mut snapshot, 0);
    }
    fs::write(fixture.file(), serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let snapshot = store
        .transact(Request {
            correlation: "approve-second".into(),
            expected_revision: snapshot.revision,
            action: Action::Approve { task: 2 },
        })
        .unwrap()
        .0;
    let bytes = fs::read(fixture.file()).unwrap();
    let error = store
        .transact(Request {
            correlation: "start-second".into(),
            expected_revision: snapshot.revision,
            action: Action::Start {
                task: 2,
                baseline: "0".repeat(40),
                inputs: vec![],
            },
        })
        .unwrap_err();
    assert!(error.contains("reserved"), "{error}");
    assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
    assert!(store.snapshot().unwrap().tasks[1].run.is_none());
    store.transact(capacity_finish(&store, &snapshot)).unwrap();
}

#[test]
fn dispatch_selects_approved_work_only_after_accepted_dependencies_and_once_per_approval() {
    use alfredo_tui::{dispatch::Dispatch, tasks::WorkPolicy};
    let fixture = Fixture::new();
    let store = fixture.store("dispatch");
    let mut snapshot = store.transact(proposal("first", 0, vec![])).unwrap().0;
    for action in [
        Action::Propose {
            title: "child".into(),
            model: "fixture".into(),
            dependencies: vec![1],
        },
        Action::Permit {
            task: 1,
            policy: WorkPolicy {
                files: vec!["a".into()],
                check: vec!["true".into()],
            },
        },
        Action::Permit {
            task: 2,
            policy: WorkPolicy {
                files: vec!["b".into()],
                check: vec!["true".into()],
            },
        },
        Action::Approve { task: 1 },
        Action::Approve { task: 2 },
    ] {
        snapshot = store
            .transact(Request {
                correlation: format!("r{}", snapshot.revision),
                expected_revision: snapshot.revision,
                action,
            })
            .unwrap()
            .0;
    }
    let mut dispatch = Dispatch::default();
    assert_eq!(dispatch.next(&snapshot, &Default::default()), None);
    dispatch.enabled = true;
    assert_eq!(dispatch.next(&snapshot, &Default::default()), Some(1));
    dispatch
        .attempts
        .insert(1, alfredo_tui::dispatch::approval(&snapshot, 1).unwrap());
    assert_eq!(dispatch.next(&snapshot, &Default::default()), None);
    // These are scheduling projections only; TaskStore still validates actual run claims.
    snapshot.tasks[0].status = TaskStatus::ReviewReady;
    assert_eq!(dispatch.next(&snapshot, &Default::default()), None);
    snapshot.tasks[0].status = TaskStatus::Accepted;
    assert_eq!(dispatch.next(&snapshot, &Default::default()), Some(2));
    assert_eq!(dispatch.next(&snapshot, &[2].into_iter().collect()), None);
    snapshot.tasks[1].policy = None;
    assert_eq!(dispatch.next(&snapshot, &Default::default()), None);
}

#[test]
fn failed_manual_start_is_not_automatically_retried_and_dispatch_defaults_off_after_reopen() {
    use alfredo_tui::{provider::Ollama, task_control::TaskControl, tasks::WorkPolicy};
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let fixture = Fixture::new();
    let store = fixture.store("dispatch-failure");
    store.transact(proposal("first", 0, vec![])).unwrap();
    store
        .transact(Request {
            correlation: "permit".into(),
            expected_revision: 1,
            action: Action::Permit {
                task: 1,
                policy: WorkPolicy {
                    files: vec!["a".into()],
                    check: vec!["true".into()],
                },
            },
        })
        .unwrap();
    store
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: 2,
            action: Action::Approve { task: 1 },
        })
        .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(store.clone());
    control.set_provider(Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap());
    control.refresh(&runtime);
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    control.command(&runtime, "/run 1", "fixture").unwrap();
    while !control.workers.is_empty() {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(control.dispatch.failures.contains_key(&1));
    assert!(store.snapshot().unwrap().tasks[0].run.is_none());
    for command in ["/dispatch on", "/dispatch off", "/dispatch on"] {
        let intent = control
            .prepare_command(command, "fixture")
            .unwrap()
            .unwrap();
        control.dispatch_prepared(&runtime, &intent).unwrap();
        while control.intent_pending(&intent) {
            control.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(control.dispatch.enabled, command.ends_with("on"));
        assert!(control.prepare_dispatch().unwrap().is_none());
    }
    assert_eq!(store.snapshot().unwrap().revision, 3);
    control.cancel_all();
    assert!(!control.dispatch.enabled);
    let reopened = TaskControl::new(store);
    assert!(!reopened.dispatch.enabled);
}

#[test]
fn background_dispatch_refresh_does_not_block_commands_or_clear_pending_write_state() {
    use alfredo_tui::task_control::TaskControl;
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let fixture = Fixture::new();
    let store = fixture.store("background-refresh");
    store.transact(proposal("first", 0, vec![])).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(store.clone());
    control.refresh(&runtime);
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    control.refresh_background(&runtime);
    assert!(!control.pending);
    control.command(&runtime, "/approve 1", "fixture").unwrap();
    assert!(control.writing);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Approved
    );
    for _ in 0..10 {
        control.poll();
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(control.snapshot.as_ref().unwrap().revision, 2);
    assert!(control.notice.contains("revision 2"));
}

#[test]
fn task_search_limits_navigation_and_shorthand_actions_to_visible_rows() {
    use alfredo_tui::{model::App, task_control::TaskControl, ui};
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let fixture = Fixture::new();
    let store = fixture.store("search");
    for (index, title) in ["Alpha", "Beta implementation", "Beta tests"]
        .iter()
        .enumerate()
    {
        store
            .transact(Request {
                correlation: format!("task-{index}"),
                expected_revision: index as u64,
                action: Action::Propose {
                    title: (*title).into(),
                    model: "fixture".into(),
                    dependencies: if index == 1 { vec![1] } else { vec![] },
                },
            })
            .unwrap();
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(store.clone());
    control.snapshot = Some(store.snapshot().unwrap());
    let bytes = fs::read(fixture.file()).unwrap();
    control.command(&runtime, "/tasks #2", "fixture").unwrap();
    control.command(&runtime, "/tasks BETA", "fixture").unwrap();
    assert_eq!(
        control
            .visible_tasks()
            .iter()
            .map(|task| task.id)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(control.selected_task().unwrap().id, 2);
    control.select_task(false);
    assert!(control.selected_task().is_none());
    control.select_task(false);
    assert_eq!(control.selected_task().unwrap().id, 3);
    control.select_task(true);
    assert!(control.selected_task().is_none());
    control.select_task(true);
    assert_eq!(control.selected_task().unwrap().id, 2);
    assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
    control.command(&runtime, "/tasks #2", "fixture").unwrap();
    control.command(&runtime, "/approve", "fixture").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let saved = store.snapshot().unwrap();
    assert_eq!(saved.tasks[0].status, TaskStatus::Proposed);
    assert_eq!(saved.tasks[1].status, TaskStatus::Approved);
    assert_eq!(saved.tasks[2].status, TaskStatus::Proposed);
    control
        .command(&runtime, "/tasks blocked", "fixture")
        .unwrap();
    assert_eq!(control.selected_task().unwrap().id, 2);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 32)).unwrap();
    terminal
        .draw(|frame| ui::draw_with_tasks(frame, &App::new("fixture".into()), &control))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("blocked by #1"), "{text}");
    assert!(text.contains("filter blocked   1/3 tasks"), "{text}");
    control
        .command(&runtime, "/tasks no-such-task", "fixture")
        .unwrap();
    assert!(control.selected_task().is_none());
    assert!(control.command(&runtime, "/approve", "fixture").is_err());
    assert_eq!(store.snapshot().unwrap().revision, saved.revision);
    terminal
        .draw(|frame| ui::draw_with_tasks(frame, &App::new("fixture".into()), &control))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("No matching tasks"));
    assert!(control
        .command(&runtime, &format!("/tasks {}", "x".repeat(201)), "fixture")
        .is_err());
}

#[test]
fn task_readiness_distinguishes_policy_approval_and_accepted_dependency_gates() {
    use alfredo_tui::{task_view::readiness, tasks::WorkPolicy};
    let fixture = Fixture::new();
    let store = fixture.store("readiness");
    store.transact(proposal("parent", 0, vec![])).unwrap();
    let mut snapshot = store.transact(proposal("child", 1, vec![1])).unwrap().0;
    assert!(readiness(&snapshot, &snapshot.tasks[1]).contains("Needs exact file/check policy"));
    snapshot.tasks[1].policy = Some(WorkPolicy {
        files: vec!["a".into()],
        check: vec!["true".into()],
    });
    assert!(readiness(&snapshot, &snapshot.tasks[1]).starts_with("Needs approval"));
    snapshot.tasks[1].status = TaskStatus::Approved;
    snapshot.tasks[0].status = TaskStatus::ReviewReady;
    assert!(readiness(&snapshot, &snapshot.tasks[1]).contains("blocked by #1 (ReviewReady)"));
    snapshot.tasks[0].status = TaskStatus::Accepted;
    assert_eq!(
        readiness(&snapshot, &snapshot.tasks[1]),
        "Approved for run validation"
    );
}

#[test]
fn resolution_command_requires_one_explicit_source_id() {
    assert_eq!(
        parse("/resolve-repair 2", "worker").unwrap(),
        Action::ResolveRepair { task: 2 }
    );
    for input in [
        "/resolve-repair",
        "/resolve-repair x",
        "/resolve-repair 2 3",
    ] {
        assert!(parse(input, "worker").is_err());
    }
}

#[test]
fn busy_scope_lock_is_a_transient_refusal_and_writes_nothing() {
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    store.transact(proposal("first", 0, vec![])).unwrap();
    store
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: 1,
            action: Action::Approve { task: 1 },
        })
        .unwrap();
    let before = fs::read(fixture.file()).unwrap();
    let scope = store.understanding();
    let held = scope.lock().unwrap();
    let refusal = store
        .transact_checked(proposal("second", 2, vec![]))
        .unwrap_err();
    assert!(matches!(refusal, Refusal::Busy), "{refusal:?}");
    assert!(refusal.transient());
    let claim = store.claim_worker(1).err().unwrap();
    assert!(matches!(claim, Refusal::Busy), "{claim:?}");
    drop(held);
    assert_eq!(fs::read(fixture.file()).unwrap(), before);
    store.transact(proposal("second", 2, vec![])).unwrap();
    drop(store.claim_worker(1).unwrap());
}
