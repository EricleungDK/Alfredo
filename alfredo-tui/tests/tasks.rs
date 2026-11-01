mod common;
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
fn permit_request(correlation: &str, revision: u64, task: u64) -> Request {
    Request {
        correlation: correlation.into(),
        expected_revision: revision,
        action: Action::Permit {
            task,
            policy: alfredo_tui::tasks::WorkPolicy {
                files: vec!["calc.py".into()],
                check: vec!["/usr/bin/python3".into(), "-c".into(), "pass".into()],
            },
        },
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
    fixture
        .store("mission")
        .transact(permit_request("permit", 1, 1))
        .unwrap();
    let approval = Request {
        correlation: "approve".into(),
        expected_revision: 2,
        action: Action::Approve { task: 1 },
    };
    fixture.store("mission").transact(approval).unwrap();
    let (restored, replay) = fixture.store("mission").transact(request).unwrap();
    assert_eq!(replay, receipt);
    assert_eq!(restored.tasks.len(), 1);
    assert_eq!(restored.tasks[0].status, TaskStatus::Approved);
    assert_eq!(restored.revision, 3);
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
    store.transact(permit_request("permit-30", 30, 30)).unwrap();
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
        .transact(proposal("background-new-task", 32, vec![]))
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
    store.transact(permit_request("permit-one", 1, 1)).unwrap();
    store
        .transact(Request {
            correlation: "approval-one".into(),
            expected_revision: 2,
            action: Action::Approve { task: 1 },
        })
        .unwrap();
    for revision in 3..12 {
        store
            .transact(proposal(&format!("proposal-{revision}"), revision, vec![]))
            .unwrap();
    }
    store.transact(original).unwrap();
    let snapshot = store.snapshot().unwrap();
    let entries = activity::entries(&snapshot, "");
    assert_eq!(entries.len(), 12);
    assert_eq!(entries[0].revision, 12);
    assert_eq!(entries.last().unwrap().revision, 1);
    let selected = activity::entries(&snapshot, "#1");
    assert_eq!(selected.len(), 3);
    assert!(selected.iter().all(|entry| entry.task == 1));
    assert_eq!(activity::entries(&snapshot, "APPROVAL-ONE").len(), 1);
    assert_eq!(activity::entries(&snapshot, "approved").len(), 1);
    assert!(store
        .transact(Request {
            correlation: "invalid".into(),
            expected_revision: 12,
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
    assert_eq!(activity::entries(&store.snapshot().unwrap(), "#1").len(), 3);
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
        failure_code: None,
        generation: None,
        project_notes: None,
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

/// Drive `/run` the way the terminal does and return the refusal text plus the
/// task inspector rendered with that task selected.
fn run_then_render(
    store: &TaskStore,
    commands: &[&str],
) -> (alfredo_tui::task_control::TaskControl, Vec<String>) {
    use alfredo_tui::task_control::TaskControl;
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(store.clone());
    control.set_provider(
        alfredo_tui::provider::Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
    );
    control.refresh(&runtime);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut messages = vec![];
    for command in commands {
        while control.pending {
            control.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        if let Err(error) = control.command(&runtime, command, "fixture") {
            messages.push(error);
        }
        while control.pending || !control.workers.is_empty() {
            control.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        messages.push(control.notice.clone());
    }
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    (control, messages)
}

fn inspector_text(control: &alfredo_tui::task_control::TaskControl) -> String {
    use alfredo_tui::{model::App, ui};
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 40)).unwrap();
    terminal
        .draw(|frame| ui::draw_with_tasks(frame, &App::new("fixture".into()), control))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn run_on_a_proposed_task_names_approve_not_the_internal_worker_owner_rule() {
    let fixture = Fixture::new();
    let store = fixture.store("run-proposed");
    store.transact(proposal("task", 0, vec![])).unwrap();
    store.transact(permit_request("permit", 1, 1)).unwrap();
    let before = fs::read(fixture.file()).unwrap();
    let (control, messages) = run_then_render(&store, &["/run 1"]);
    let joined = messages.join(" | ");
    assert!(
        joined.contains("Task #1 is proposed; /approve 1 first"),
        "{joined}"
    );
    assert!(!joined.contains("acquire a new worker owner"), "{joined}");
    assert!(!inspector_text(&control).contains("start paused"));
    assert_eq!(fs::read(fixture.file()).unwrap(), before);
    assert!(store.snapshot().unwrap().tasks[0].run.is_none());
}

/// Statuses reached through `task_in_status`; a Cancelled task there never ran.
const RUN_REFUSALS: [(TaskStatus, &str); 6] = [
    (
        TaskStatus::Accepted,
        "Task #1 is accepted; nothing is left to run",
    ),
    (
        TaskStatus::Failed,
        "Task #1 is failed; /repair 1 <hint> to try again",
    ),
    (
        TaskStatus::Rejected,
        "Task #1 is rejected; /repair 1 <hint> to try again",
    ),
    (
        TaskStatus::ReviewReady,
        "Task #1 is review-ready; /accept 1 or /review 1 JSON",
    ),
    (
        TaskStatus::Cancelled,
        "Task #1 was cancelled before it ran; /task describes new work to do instead",
    ),
    (
        TaskStatus::NeedsHumanReview,
        "Task #1 is needs-human-review; /review 1 JSON",
    ),
];

const CANCELLED_DURING_RUN_REFUSAL: &str = "Task #1 is cancelled; /requeue 1 to approve it again";

#[test]
fn run_on_a_non_approved_task_names_its_status_and_the_next_command() {
    for (status, expected) in RUN_REFUSALS {
        let fixture = Fixture::new();
        let (store, _) = task_in_status(&fixture, status.clone(), true);
        let before = fs::read(fixture.file()).unwrap();
        let (mut control, messages) = run_then_render(&store, &["/run 1"]);
        let joined = messages.join(" | ");
        assert!(
            messages.iter().any(|message| message == expected),
            "{status:?}: {joined}"
        );
        assert!(!joined.contains("acquire a new worker owner"), "{joined}");
        assert!(control.workers.is_empty());
        assert!(!control.dispatch.failures.contains_key(&1), "{status:?}");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        control.command(&runtime, "/tasks #1", "fixture").unwrap();
        let text = inspector_text(&control);
        assert!(text.contains("Task #1"), "{status:?}: {text}");
        assert!(!text.contains("start paused"), "{status:?}: {text}");
        assert_eq!(fs::read(fixture.file()).unwrap(), before, "{status:?}");
        assert_eq!(store.snapshot().unwrap().tasks[0].status, status);
    }
}

#[test]
fn cancelled_during_run_task_run_hint_requeue_is_accepted_by_the_store() {
    let fixture = Fixture::new();
    let (store, revision) = cancelled_after_run(&fixture);
    let task = store.snapshot().unwrap().tasks[0].clone();
    assert!(task.run.is_some() && task.policy.is_some());
    assert!(task.can_requeue());
    let (control, messages) = run_then_render(&store, &["/run 1"]);
    assert!(
        messages
            .iter()
            .any(|message| message == CANCELLED_DURING_RUN_REFUSAL),
        "{messages:?}"
    );
    assert!(!control.dispatch.failures.contains_key(&1));
    assert_eq!(store.snapshot().unwrap().revision, revision);
    let (control, _) = run_then_render(&store, &["/run 1", "/requeue 1"]);
    assert_eq!(
        store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Approved
    );
    assert!(!control.dispatch.failures.contains_key(&1));
}

#[test]
fn only_approved_and_running_tasks_have_no_pre_start_run_refusal() {
    use alfredo_tui::tasks::run_refusal;
    let task_with = |status: TaskStatus| {
        let fixture = Fixture::new();
        let (store, _) = task_in_status(&fixture, status, true);
        store.snapshot().unwrap().tasks[0].clone()
    };
    assert_eq!(run_refusal(&task_with(TaskStatus::Approved)), None);
    let mut proposed = task_with(TaskStatus::Proposed);
    proposed.id = 7;
    assert_eq!(
        run_refusal(&proposed).as_deref(),
        Some("Task #7 is proposed; /approve 7 first")
    );
    let running = task_with(TaskStatus::Running);
    assert_eq!(
        run_refusal(&running).as_deref(),
        Some(
            "Task #1 is running; wait for it to finish, /stop it, or /recover 1 if its worker died"
        )
    );
    for (status, expected) in RUN_REFUSALS {
        assert_eq!(run_refusal(&task_with(status)).as_deref(), Some(expected));
    }
    let fixture = Fixture::new();
    let (store, _) = cancelled_after_run(&fixture);
    let mut cancelled = store.snapshot().unwrap().tasks[0].clone();
    assert_eq!(
        run_refusal(&cancelled).as_deref(),
        Some(CANCELLED_DURING_RUN_REFUSAL)
    );
    // A cancelled run without a policy cannot be requeued, so the hint must not say so.
    cancelled.policy = None;
    assert!(!cancelled.can_requeue());
    assert_eq!(
        run_refusal(&cancelled).as_deref(),
        Some("Task #1 is cancelled; /tasks shows what can still be reviewed")
    );
}

#[test]
fn bare_run_on_a_selected_non_approved_task_names_its_status_and_the_next_command() {
    for (status, expected) in RUN_REFUSALS {
        let fixture = Fixture::new();
        let (store, _) = task_in_status(&fixture, status.clone(), true);
        let before = fs::read(fixture.file()).unwrap();
        let (control, messages) = run_then_render(&store, &["/tasks #1", "/run"]);
        let joined = messages.join(" | ");
        assert!(
            messages.iter().any(|message| message == expected),
            "{status:?}: {joined}"
        );
        assert!(!joined.contains("acquire a new worker owner"), "{joined}");
        assert!(!control.dispatch.failures.contains_key(&1), "{status:?}");
        assert!(!inspector_text(&control).contains("start paused"));
        assert_eq!(fs::read(fixture.file()).unwrap(), before, "{status:?}");
    }
}

#[test]
fn run_on_a_running_task_keeps_its_own_refusal_not_the_status_hint() {
    let fixture = Fixture::new();
    let (store, _) = task_in_status(&fixture, TaskStatus::Running, true);
    let (_, messages) = run_then_render(&store, &["/run 1"]);
    let joined = messages.join(" | ");
    assert!(!joined.contains("nothing is left to run"), "{joined}");
    assert!(!joined.contains("/repair 1"), "{joined}");
}

#[test]
fn cancelled_never_ran_task_run_hint_is_a_command_the_store_accepts() {
    let fixture = Fixture::new();
    let (store, revision) = task_in_status(&fixture, TaskStatus::Cancelled, true);
    assert!(store.snapshot().unwrap().tasks[0].run.is_none());
    // The store refuses what the old hint recommended.
    let error = refusal(&fixture, &store, revision, Action::Requeue { task: 1 });
    assert!(error.contains("Requeue requires"), "{error}");
    let (control, messages) = run_then_render(&store, &["/run 1", "/task Replacement work"]);
    let joined = messages.join(" | ");
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Task #1 was cancelled before it ran")),
        "{joined}"
    );
    assert!(joined.contains("/task"), "{joined}");
    assert!(!joined.contains("/requeue"), "{joined}");
    assert!(!joined.contains("/run 1 first"), "{joined}");
    assert!(!control.dispatch.failures.contains_key(&1));
    // The command it names is accepted: it adds new work and leaves #1 cancelled.
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.tasks.len(), 2, "{joined}");
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Cancelled);
}

#[test]
fn not_run_view_for_cancelled_task_names_task_command() {
    let fixture = Fixture::new();
    let (store, _) = task_in_status(&fixture, TaskStatus::Cancelled, true);
    let before = fs::read(fixture.file()).unwrap();
    for command in ["/evidence 1", "/accept 1"] {
        let (_, messages) = run_then_render(&store, &[command]);
        let joined = messages.join(" | ");
        assert!(!joined.contains("/run 1"), "{command}: {joined}");
        assert!(!joined.contains("/requeue"), "{command}: {joined}");
        if command != "/accept 1" {
            assert!(
                joined.contains("Task #1 was cancelled before it ran; /task"),
                "{command}: {joined}"
            );
        }
    }
    assert_eq!(fs::read(fixture.file()).unwrap(), before);
}

#[test]
fn branch_record_for_a_task_cancelled_before_it_ran_names_task_not_run() {
    let fixture = Fixture::new();
    let (store, revision) = task_in_status(&fixture, TaskStatus::Cancelled, true);
    let error = refusal(
        &fixture,
        &store,
        revision,
        Action::Branch {
            task: 1,
            name: "alfredo/task-1".into(),
            commit: "a".repeat(40),
        },
    );
    assert_eq!(
        error,
        "Task #1 was cancelled before it ran; /task describes new work to do instead"
    );
}

#[test]
fn run_on_running_task_without_worker_names_status_not_internal_rule() {
    let fixture = Fixture::new();
    let (store, _) = task_in_status(&fixture, TaskStatus::Running, true);
    let before = fs::read(fixture.file()).unwrap();
    let (mut control, messages) = run_then_render(&store, &["/run 1"]);
    let joined = messages.join(" | ");
    assert!(
        messages
            .iter()
            .any(|message| message.starts_with("Task #1 is running")),
        "{joined}"
    );
    assert!(joined.contains("/recover 1"), "{joined}");
    assert!(!joined.contains("acquire a new worker owner"), "{joined}");
    assert!(control.workers.is_empty());
    assert!(!control.dispatch.failures.contains_key(&1), "{joined}");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    control.command(&runtime, "/tasks #1", "fixture").unwrap();
    let text = inspector_text(&control);
    assert!(text.contains("Task #1"), "{text}");
    assert!(!text.contains("start paused"), "{text}");
    assert_eq!(fs::read(fixture.file()).unwrap(), before);
    assert_eq!(
        store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Running
    );
}

#[test]
fn accept_cancel_task_and_repair_on_running_task_without_worker_name_recover() {
    let fixture = Fixture::new();
    let (store, _) = task_in_status(&fixture, TaskStatus::Running, true);
    let before = fs::read(fixture.file()).unwrap();
    for command in ["/accept 1", "/cancel-task 1", "/repair 1 try again"] {
        let (_, messages) = run_then_render(&store, &[command]);
        let joined = messages.join(" | ");
        assert!(joined.contains("Task #1 is running"), "{command}: {joined}");
        assert!(joined.contains("/recover 1"), "{command}: {joined}");
        assert!(
            !joined.contains("no evidence receipt"),
            "{command}: {joined}"
        );
        assert!(
            !joined.contains("needs a proposed or approved"),
            "{command}: {joined}"
        );
    }
    assert_eq!(fs::read(fixture.file()).unwrap(), before);
}

#[test]
fn refused_run_leaves_no_start_paused_row_once_the_task_is_approved() {
    let fixture = Fixture::new();
    let store = fixture.store("run-then-approve");
    store.transact(proposal("task", 0, vec![])).unwrap();
    store.transact(permit_request("permit", 1, 1)).unwrap();
    let (control, _) = run_then_render(&store, &["/run 1", "/approve 1"]);
    assert_eq!(
        store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Approved
    );
    let text = inspector_text(&control);
    assert!(text.contains("Approved for run validation"), "{text}");
    assert!(!text.contains("start paused"), "{text}");
}

#[test]
fn a_start_notice_recorded_while_proposed_is_dropped_when_the_task_is_approved() {
    let fixture = Fixture::new();
    let store = fixture.store("stale-notice");
    store.transact(proposal("task", 0, vec![])).unwrap();
    store.transact(permit_request("permit", 1, 1)).unwrap();
    let (mut control, _) = run_then_render(&store, &[]);
    control.dispatch.failures.insert(1, "start refused".into());
    control.dispatch.failures.insert(2, "other task".into());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    control.command(&runtime, "/tasks #1", "fixture").unwrap();
    assert!(inspector_text(&control).contains("start paused: start refused"));
    control.command(&runtime, "/approve 1", "fixture").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!control.dispatch.failures.contains_key(&1));
    assert!(control.dispatch.failures.contains_key(&2));
}

#[test]
fn run_on_an_approved_task_with_an_unaccepted_dependency_still_names_the_dependency() {
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
        vec!["add", "-A"],
        vec![
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "fixture",
            "--allow-empty",
        ],
    ] {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    let store = fixture.store("run-dependency");
    store.transact(proposal("parent", 0, vec![])).unwrap();
    store.transact(proposal("child", 1, vec![1])).unwrap();
    store.transact(permit_request("permit", 2, 2)).unwrap();
    store
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: 3,
            action: Action::Approve { task: 2 },
        })
        .unwrap();
    let (mut control, messages) = run_then_render(&store, &["/run 2"]);
    let joined = messages.join(" | ");
    assert!(joined.contains("Dependency #1 is not accepted"), "{joined}");
    assert!(!joined.contains("acquire a new worker owner"), "{joined}");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    control.command(&runtime, "/tasks #2", "fixture").unwrap();
    let text = inspector_text(&control);
    assert!(text.contains("Task #2"), "{text}");
    assert!(!text.contains("start paused"), "{text}");
    assert!(store.snapshot().unwrap().tasks[1].run.is_none());
}

#[test]
fn run_after_the_dependency_was_just_accepted_is_not_refused_by_a_stale_snapshot() {
    use alfredo_tui::task_control::TaskControl;
    use std::time::Duration;
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
        vec!["add", "-A"],
        vec![
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "fixture",
            "--allow-empty",
        ],
    ] {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    // Task 1 is already accepted in the store; task 2 depends on it and is approved.
    let (store, revision) = task_in_status(&fixture, TaskStatus::Accepted, false);
    store
        .transact(proposal("child", revision, vec![1]))
        .unwrap();
    store
        .transact(permit_request("permit", revision + 1, 2))
        .unwrap();
    store
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: revision + 2,
            action: Action::Approve { task: 2 },
        })
        .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(store.clone());
    control.set_provider(
        alfredo_tui::provider::Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
    );
    control.refresh(&runtime);
    while control.pending {
        control.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    // The terminal still holds the view from before the acceptance landed.
    control.snapshot.as_mut().unwrap().tasks[0].status = TaskStatus::ReviewReady;
    let result = control.command(&runtime, "/run 2", "fixture");
    assert!(
        result.is_ok(),
        "stale snapshot refused a start the store accepts: {result:?}"
    );
}

#[test]
fn run_on_a_repair_task_is_not_refused_for_an_unaccepted_dependency_like_the_store() {
    use alfredo_tui::task_control::TaskControl;
    use std::time::{Duration, Instant};
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
        vec!["add", "-A"],
        vec![
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "fixture",
            "--allow-empty",
        ],
    ] {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    let store = fixture.store("run-repair-dependency");
    store.transact(proposal("parent", 0, vec![])).unwrap();
    store.transact(proposal("child", 1, vec![1])).unwrap();
    store.transact(permit_request("permit", 2, 2)).unwrap();
    store
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: 3,
            action: Action::Approve { task: 2 },
        })
        .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(store.clone());
    control.set_provider(
        alfredo_tui::provider::Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
    );
    control.refresh(&runtime);
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    // The store skips the dependency rule for a repair, so the precheck must too.
    control.snapshot.as_mut().unwrap().tasks[1].repair_of = Some(1);
    let result = control.command(&runtime, "/run 2", "fixture");
    if let Err(error) = &result {
        assert!(!error.contains("Dependency"), "{error}");
    }
    while control.pending || !control.workers.is_empty() {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
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
    store.transact(permit_request("permit", 1, 1)).unwrap();
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
    assert_eq!(control.snapshot.as_ref().unwrap().revision, 3);
    assert!(control.notice.contains("revision 3"));
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
    store.transact(permit_request("permit-2", 3, 2)).unwrap();
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
    store.transact(permit_request("permit", 1, 1)).unwrap();
    store
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: 2,
            action: Action::Approve { task: 1 },
        })
        .unwrap();
    let before = fs::read(fixture.file()).unwrap();
    let scope = store.understanding();
    let held = scope.lock().unwrap();
    let refusal = store
        .transact_checked(proposal("second", 3, vec![]))
        .unwrap_err();
    assert!(matches!(refusal, Refusal::Busy), "{refusal:?}");
    assert!(refusal.transient());
    let claim = store.claim_worker(1).err().unwrap();
    assert!(matches!(claim, Refusal::Busy), "{claim:?}");
    drop(held);
    assert_eq!(fs::read(fixture.file()).unwrap(), before);
    store.transact(proposal("second", 3, vec![])).unwrap();
    drop(store.claim_worker(1).unwrap());
}

fn policy() -> alfredo_tui::tasks::WorkPolicy {
    alfredo_tui::tasks::WorkPolicy {
        files: vec!["calc.py".into()],
        check: vec!["/bin/true".into()],
    }
}

/// Task 1 permitted, approved, started and finished with `status`; returns the store.
fn finished(fixture: &Fixture, status: TaskStatus) -> TaskStore {
    let store = fixture.store("mission");
    let steps = [
        Action::Propose {
            title: "Requeue me".into(),
            model: "fixture".into(),
            dependencies: vec![],
        },
        Action::Permit {
            task: 1,
            policy: policy(),
        },
        Action::Approve { task: 1 },
        Action::Start {
            task: 1,
            baseline: "a".repeat(40),
            inputs: vec![],
        },
    ];
    for (index, action) in steps.into_iter().enumerate() {
        store
            .transact(Request {
                correlation: format!("step-{index}"),
                expected_revision: index as u64,
                action,
            })
            .unwrap();
    }
    // Synthetic worker digest: the Finish receipt is written directly, as the
    // activity tests do, and the store's full replay validator accepts it.
    let mut snapshot = store.snapshot().unwrap();
    let run = snapshot.tasks[0].run.clone().unwrap().id;
    snapshot.receipts.push(alfredo_tui::tasks::Receipt {
        revision: 5,
        task: 1,
        request: Request {
            correlation: "finish".into(),
            expected_revision: 4,
            action: Action::Finish {
                task: 1,
                run,
                status: status.clone(),
                evidence_sha256: "b".repeat(64),
                detail: "Worker cancelled during inference".into(),
            },
        },
    });
    snapshot.revision = 5;
    snapshot.tasks[0].status = status;
    let bound = snapshot.tasks[0].run.as_mut().unwrap();
    bound.evidence_sha256 = Some("b".repeat(64));
    bound.detail = "Worker cancelled during inference".into();
    fs::write(
        fixture.file(),
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .unwrap();
    store
}

#[test]
fn requeue_returns_a_cancelled_run_to_approved_and_the_next_start_gets_a_fresh_run() {
    let fixture = Fixture::new();
    let store = finished(&fixture, TaskStatus::Cancelled);
    let first = store.snapshot().unwrap().tasks[0].run.clone().unwrap().id;
    let (queued, _) = store
        .transact(Request {
            correlation: "requeue".into(),
            expected_revision: 5,
            action: Action::Requeue { task: 1 },
        })
        .unwrap();
    assert_eq!(queued.tasks.len(), 1);
    assert_eq!(queued.tasks[0].status, TaskStatus::Approved);
    assert_eq!(queued.tasks[0].policy, Some(policy()));
    assert!(queued.tasks[0].repair_of.is_none());
    assert!(queued.receipts.iter().any(|r| matches!(
        &r.request.action,
        Action::Finish {
            status: TaskStatus::Cancelled,
            ..
        }
    )));
    let (started, _) = store
        .transact(Request {
            correlation: "start-again".into(),
            expected_revision: 6,
            action: Action::Start {
                task: 1,
                baseline: "a".repeat(40),
                inputs: vec![],
            },
        })
        .unwrap();
    assert_eq!(started.tasks[0].status, TaskStatus::Running);
    assert_ne!(started.tasks[0].run.as_ref().unwrap().id, first);
    assert_eq!(
        parse("/requeue 1", "m").unwrap(),
        Action::Requeue { task: 1 }
    );
    assert!(parse("/requeue x", "m").is_err());
}

#[test]
fn requeue_is_refused_unless_the_task_was_cancelled_after_a_run() {
    for status in [TaskStatus::Failed, TaskStatus::ReviewReady] {
        let fixture = Fixture::new();
        let store = finished(&fixture, status);
        let before = fs::read(fixture.file()).unwrap();
        assert!(store
            .transact(Request {
                correlation: "requeue".into(),
                expected_revision: 5,
                action: Action::Requeue { task: 1 },
            })
            .is_err());
        assert_eq!(fs::read(fixture.file()).unwrap(), before);
    }
    // Cancelled before any run, unknown, and already-approved tasks.
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    for (index, action) in [
        Action::Propose {
            title: "Never ran".into(),
            model: "fixture".into(),
            dependencies: vec![],
        },
        Action::Cancel { task: 1 },
    ]
    .into_iter()
    .enumerate()
    {
        store
            .transact(Request {
                correlation: format!("s{index}"),
                expected_revision: index as u64,
                action,
            })
            .unwrap();
    }
    for (name, task) in [("unstarted", 1), ("unknown", 9)] {
        assert!(store
            .transact(Request {
                correlation: name.into(),
                expected_revision: 2,
                action: Action::Requeue { task },
            })
            .is_err());
    }
}

#[test]
fn a_v16_store_cannot_replay_a_requeue_receipt_but_upgrades_on_write() {
    let fixture = Fixture::new();
    let store = finished(&fixture, TaskStatus::Cancelled);
    store
        .transact(Request {
            correlation: "requeue".into(),
            expected_revision: 5,
            action: Action::Requeue { task: 1 },
        })
        .unwrap();
    assert_eq!(store.snapshot().unwrap().schema_version, 17);
    let path = fixture.file();
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["schema_version"] = 16.into();
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let error = store.snapshot().unwrap_err();
    assert!(error.contains("v17"), "{error}");

    // A genuine v16 store without Requeue receipts still opens and upgrades.
    let other = Fixture::new();
    let store = finished(&other, TaskStatus::Cancelled);
    let path = other.file();
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["schema_version"] = 16.into();
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    assert_eq!(store.snapshot().unwrap().schema_version, 16);
    let (upgraded, _) = store
        .transact(Request {
            correlation: "requeue".into(),
            expected_revision: 5,
            action: Action::Requeue { task: 1 },
        })
        .unwrap();
    assert_eq!(upgraded.schema_version, 17);
}

#[test]
fn state_dir_with_dotdot_resolving_outside_the_workspace_is_accepted() {
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    // Existing sibling, and a not-yet-existing sibling, both reached through `..`.
    fs::create_dir(fixture.root.join("sibling")).unwrap();
    for name in ["sibling", "missing/deeper"] {
        let state = workspace.join("..").join(name);
        TaskStore::new(&state, &workspace, "mission")
            .unwrap_or_else(|e| panic!("{name} should be accepted: {e}"));
    }
}

#[test]
fn state_dir_with_dotdot_resolving_inside_the_workspace_is_rejected_with_both_paths() {
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    fs::create_dir(workspace.join("sub")).unwrap();
    for state in [
        workspace.join("sub/../state"),
        workspace.join("./state"),
        workspace.join("missing/../state"),
        fixture.root.join("workspace/sub/../../workspace/state"),
    ] {
        let message = TaskStore::new(&state, &workspace, "mission")
            .err()
            .unwrap_or_else(|| panic!("{} should be rejected", state.display()));
        let resolved = workspace.canonicalize().unwrap();
        assert!(
            message.contains("outside the coding workspace"),
            "{message}"
        );
        assert!(
            message.contains(&resolved.join("state").display().to_string()),
            "{message}"
        );
        assert!(
            message.contains(&resolved.display().to_string()),
            "{message}"
        );
    }
    assert!(!workspace.join("state").exists());
}

#[test]
fn state_dir_through_a_symlink_into_the_workspace_is_rejected() {
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    std::os::unix::fs::symlink(&workspace, fixture.root.join("link")).unwrap();
    // `link/..` is the OS parent of the symlink target, not of `link`'s parent.
    assert!(TaskStore::new(&fixture.root.join("link/state"), &workspace, "m").is_err());
    fs::create_dir(workspace.join("sub")).unwrap();
    std::os::unix::fs::symlink(workspace.join("sub"), fixture.root.join("sublink")).unwrap();
    // Lexically `root/sublink/../state` is outside; the OS resolves it to workspace/state.
    assert!(TaskStore::new(&fixture.root.join("sublink/../state"), &workspace, "m").is_err());
}

#[test]
fn approve_without_policy_is_refused_with_the_permit_next_step() {
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    store.transact(proposal("first", 0, vec![])).unwrap();
    let before = fs::read(fixture.file()).unwrap();
    let error = store
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: 1,
            action: Action::Approve { task: 1 },
        })
        .unwrap_err();
    assert!(error.contains("Task #1 has no files/check yet"), "{error}");
    assert!(error.contains("/permit 1 {\"files\":["), "{error}");
    assert_eq!(
        store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Proposed
    );
    assert_eq!(fs::read(fixture.file()).unwrap(), before);
}

/// Task 1 (from a one-step plan) in `status`, with two acceptance criteria when `criteria` is set.
fn task_in_status(fixture: &Fixture, status: TaskStatus, criteria: bool) -> (TaskStore, u64) {
    task_in_state(fixture, status, criteria, false)
}

/// Task 1 cancelled by a stopped run (it started, so it has a run and a policy).
fn cancelled_after_run(fixture: &Fixture) -> (TaskStore, u64) {
    task_in_state(fixture, TaskStatus::Cancelled, true, true)
}

/// `task_in_status`, where `started` lets a Cancelled task have run first.
fn task_in_state(
    fixture: &Fixture,
    status: TaskStatus,
    criteria: bool,
    started: bool,
) -> (TaskStore, u64) {
    let store = fixture.store("mission");
    let plan = alfredo_tui::planner::Plan {
        prompt: "goal".into(),
        planner: "fixture".into(),
        tasks: vec![alfredo_tui::planner::Step {
            title: "Make calc".into(),
            acceptance: if criteria {
                vec!["one".into(), "two".into()]
            } else {
                vec![]
            },
            model: "fixture".into(),
            dependencies: vec![],
            policy: policy(),
        }],
        context: None,
        scope: None,
        architecture: None,
        usage: None,
    };
    store
        .transact(Request {
            correlation: "plan".into(),
            expected_revision: 0,
            action: Action::Plan { plan },
        })
        .unwrap();
    // Reach `status` through receipts the replay validator accepts; Finish and
    // Review receipts are written directly, as `finished` does.
    let mut actions = Vec::new();
    if status != TaskStatus::Proposed {
        actions.push(Action::Approve { task: 1 });
    }
    if status == TaskStatus::Cancelled && !started {
        actions.push(Action::Cancel { task: 1 });
    }
    let ran = started
        || !matches!(
            status,
            TaskStatus::Proposed | TaskStatus::Approved | TaskStatus::Cancelled
        );
    if ran {
        actions.push(Action::Start {
            task: 1,
            baseline: "a".repeat(40),
            inputs: vec![],
        });
    }
    let mut snapshot = store.snapshot().unwrap();
    for action in actions {
        let (next, _) = store
            .transact(Request {
                correlation: format!("step-{}", snapshot.revision),
                expected_revision: snapshot.revision,
                action,
            })
            .unwrap();
        snapshot = next;
    }
    if !matches!(status, TaskStatus::Running) && ran {
        let run = snapshot.tasks[0].run.clone().unwrap().id;
        // Review reads the run's saved evidence; its recorded status does not matter here.
        let evidence = serde_json::to_vec(&serde_json::json!({
            "run": run,
            "baseline": "a".repeat(40),
            "status": "failed",
            "detail": "check passed",
            "patch": "",
        }))
        .unwrap();
        let digest = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(&evidence))
        };
        let directory = store.run_directory(&run).unwrap();
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("evidence.json"), &evidence).unwrap();
        let finish = match status {
            TaskStatus::Accepted | TaskStatus::Rejected | TaskStatus::NeedsHumanReview => {
                TaskStatus::ReviewReady
            }
            _ => status.clone(),
        };
        let mut finals = vec![Action::Finish {
            task: 1,
            run,
            status: finish,
            evidence_sha256: digest.clone(),
            detail: "check passed".into(),
        }];
        if matches!(status, TaskStatus::Accepted | TaskStatus::Rejected) {
            finals.push(Action::Review {
                task: 1,
                accept: status == TaskStatus::Accepted,
            });
        }
        if status == TaskStatus::NeedsHumanReview {
            use alfredo_tui::assessment::{Criterion, Decision, Outcome};
            finals.push(Action::Decide {
                task: 1,
                decision: Decision {
                    failure: None,
                    risk: None,
                    outcome: Outcome::NeedsHumanReview,
                    reason: "needs a person".into(),
                    criteria: (1..=if criteria { 2 } else { 0 })
                        .map(|criterion| Criterion {
                            criterion,
                            met: true,
                            note: "ok".into(),
                        })
                        .collect(),
                    limitations: vec![],
                },
            });
        }
        for action in finals {
            let revision = snapshot.revision;
            snapshot.receipts.push(alfredo_tui::tasks::Receipt {
                revision: revision + 1,
                task: 1,
                request: Request {
                    correlation: format!("final-{revision}"),
                    expected_revision: revision,
                    action,
                },
            });
            snapshot.revision = revision + 1;
        }
        snapshot.tasks[0].status = status;
        let bound = snapshot.tasks[0].run.as_mut().unwrap();
        bound.evidence_sha256 = Some(digest);
        bound.detail = "check passed".into();
        fs::write(
            fixture.file(),
            serde_json::to_vec_pretty(&snapshot).unwrap(),
        )
        .unwrap();
    }
    let revision = store.snapshot().unwrap().revision;
    (store, revision)
}

fn refusal(fixture: &Fixture, store: &TaskStore, revision: u64, action: Action) -> String {
    let file = fixture.file();
    let before = fs::read(&file).unwrap();
    let error = store
        .transact(Request {
            correlation: "refuse".into(),
            expected_revision: revision,
            action,
        })
        .unwrap_err()
        .to_string();
    assert_eq!(fs::read(&file).unwrap(), before);
    error
}

#[test]
fn accept_and_reject_name_the_status_and_the_next_command_for_every_unreviewable_status() {
    let cases = [
        (TaskStatus::Accepted, "Task #1 is already accepted"),
        (TaskStatus::Rejected, "Task #1 is already rejected"),
        (TaskStatus::Failed, "Task #1 failed; /repair 1 <hint>"),
        (TaskStatus::Running, "Task #1 is running; wait"),
        (TaskStatus::Proposed, "Task #1 is proposed; /approve 1"),
        (
            TaskStatus::Approved,
            "Task #1 has not run; /run 1 first (it is approved)",
        ),
        (TaskStatus::Cancelled, "Task #1 is cancelled"),
        (TaskStatus::NeedsHumanReview, "Task #1 needs human review"),
    ];
    for (status, expected) in cases {
        for criteria in [false, true] {
            for accept in [true, false] {
                let fixture = Fixture::new();
                let (store, revision) = task_in_status(&fixture, status.clone(), criteria);
                let error = refusal(
                    &fixture,
                    &store,
                    revision,
                    Action::Review { task: 1, accept },
                );
                assert!(error.contains(expected), "{status:?} {accept}: {error}");
                assert!(!error.contains("Use /review"), "{status:?}: {error}");
                assert!(!error.contains("check evidence"), "{status:?}: {error}");
            }
        }
    }
}

#[test]
fn review_template_hint_is_only_for_a_review_ready_task() {
    let fixture = Fixture::new();
    let (store, revision) = task_in_status(&fixture, TaskStatus::ReviewReady, true);
    let error = refusal(
        &fixture,
        &store,
        revision,
        Action::Review {
            task: 1,
            accept: true,
        },
    );
    assert!(error.starts_with("Use /review 1 {"), "{error}");
}

#[test]
fn approve_and_cancel_refusals_name_the_status_and_allowed_transitions_are_unchanged() {
    let all = [
        TaskStatus::Proposed,
        TaskStatus::Approved,
        TaskStatus::Cancelled,
        TaskStatus::Running,
        TaskStatus::ReviewReady,
        TaskStatus::NeedsHumanReview,
        TaskStatus::Accepted,
        TaskStatus::Failed,
        TaskStatus::Rejected,
    ];
    for status in all {
        let word = alfredo_tui::tasks::status_label(&status);
        let fixture = Fixture::new();
        let (store, revision) = task_in_status(&fixture, status.clone(), false);
        let approve = store.transact(Request {
            correlation: "approve".into(),
            expected_revision: revision,
            action: Action::Approve { task: 1 },
        });
        assert_eq!(
            approve.is_ok(),
            status == TaskStatus::Proposed,
            "{status:?}"
        );
        if let Err(error) = approve {
            assert!(
                error.to_string().contains(&format!("Task #1 is {word};")),
                "{error}"
            );
        }
        let fixture = Fixture::new();
        let (store, revision) = task_in_status(&fixture, status.clone(), false);
        let cancel = store.transact(Request {
            correlation: "cancel".into(),
            expected_revision: revision,
            action: Action::Cancel { task: 1 },
        });
        assert_eq!(
            cancel.is_ok(),
            matches!(status, TaskStatus::Proposed | TaskStatus::Approved),
            "{status:?}"
        );
        if let Err(error) = cancel {
            assert!(
                error.to_string().contains(&format!("Task #1 is {word};")),
                "{error}"
            );
        }
    }
}

#[test]
fn approve_after_permit_is_accepted_and_the_run_guard_passes() {
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    store.transact(proposal("first", 0, vec![])).unwrap();
    store.transact(permit_request("permit", 1, 1)).unwrap();
    let (snapshot, _) = store
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: 2,
            action: Action::Approve { task: 1 },
        })
        .unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Approved);
    let (snapshot, _) = store
        .transact(Request {
            correlation: "run".into(),
            expected_revision: 3,
            action: Action::Start {
                inputs: vec![],
                task: 1,
                baseline: "a".repeat(40),
            },
        })
        .unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Running);
}

#[test]
fn permit_error_shows_a_json_example() {
    let error = parse("/permit 1 {}", "worker").unwrap_err();
    assert!(error.contains("{\"files\":[\"calc.py\"]"), "{error}");
    assert!(error.contains("\"check\":["), "{error}");
}

#[test]
fn a_saved_approval_without_policy_still_loads_but_never_reads_approved() {
    let fixture = Fixture::new();
    let store = fixture.store("mission");
    store.transact(proposal("first", 0, vec![])).unwrap();
    store.transact(permit_request("permit", 1, 1)).unwrap();
    store
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: 2,
            action: Action::Approve { task: 1 },
        })
        .unwrap();
    common::forge_legacy_approval(&store);
    let snapshot = fixture.store("mission").snapshot().unwrap();
    let task = &snapshot.tasks[0];
    assert_eq!(task.status, TaskStatus::Approved);
    assert_eq!(alfredo_tui::dashboard::state_word(task), "needs policy");
    let text = alfredo_tui::task_view::readiness(&snapshot, task);
    assert!(text.contains("Needs exact file/check policy"), "{text}");
    // The rendered panel agrees: never "approved" while the task needs a policy.
    let mut control = alfredo_tui::task_control::TaskControl::new(store.clone());
    control.snapshot = Some(snapshot);
    control.visible = true;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 40)).unwrap();
    terminal
        .draw(|frame| {
            alfredo_tui::ui::draw_with_tasks(
                frame,
                &alfredo_tui::model::App::new("fixture".into()),
                &control,
            )
        })
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("Needs files/check policy"), "{screen}");
    assert!(screen.contains("needs policy"), "{screen}");
    assert!(!screen.contains("Task #1 · approved"), "{screen}");
}

#[test]
fn task_receipt_notice_names_permit_before_approve() {
    use alfredo_tui::task_control::TaskControl;
    use std::time::{Duration, Instant};
    let fixture = Fixture::new();
    let store = fixture.store("notice");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(store.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    control.refresh(&runtime);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    control
        .command(&runtime, "/task Add calc", "fixture")
        .unwrap();
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        control.notice.contains("next /permit 1"),
        "{}",
        control.notice
    );
    assert!(
        control.notice.contains("then /approve 1"),
        "{}",
        control.notice
    );
}
