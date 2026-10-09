use alfredo_tui::{
    tasks::{Action, Request, TaskStatus, TaskStore, WorkPolicy},
    worker::Evidence,
};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
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
            "alfredo-recovery-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("workspace")).unwrap();
        let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "test").unwrap();
        for (revision, action) in [
            Action::Propose {
                title: "recover".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
            Action::Permit {
                task: 1,
                policy: WorkPolicy {
                    files: vec!["file".into()],
                    check: vec!["/bin/true".into()],
                },
            },
            Action::Approve { task: 1 },
        ]
        .into_iter()
        .enumerate()
        {
            store
                .transact(Request {
                    correlation: format!("setup-{revision}"),
                    expected_revision: revision as u64,
                    action,
                })
                .unwrap();
        }
        Self { root, store }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn claim(store: &TaskStore) -> String {
    let (snapshot, _) = store
        .transact(Request {
            correlation: "start".into(),
            expected_revision: 3,
            action: Action::Start {
                inputs: vec![],
                task: 1,
                baseline: "a".repeat(40),
            },
        })
        .unwrap();
    snapshot.tasks[0].run.as_ref().unwrap().id.clone()
}
fn save(store: &TaskStore, run: &str, status: TaskStatus) {
    let directory = store.run_directory(run).unwrap();
    fs::create_dir_all(&directory).unwrap();
    let evidence = Evidence {
        agent: None,
        candidate_commit: None,
        model_metrics: None,
        failure_code: None,
        generation: None,
        run: run.into(),
        baseline: "a".repeat(40),
        status,
        detail: "Model failed before applying any files".into(),
        patch: String::new(),
        check: None,
    };
    fs::write(
        directory.join("evidence.json"),
        serde_json::to_vec(&evidence).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "subprocess ownership fixture"]
fn owner_process_fixture() {
    let root = PathBuf::from(std::env::var_os("ALFREDO_RECOVERY_FIXTURE").unwrap());
    let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "test").unwrap();
    let _owner = store.claim_worker(1).unwrap();
    let run = claim(&store);
    save(&store, &run, TaskStatus::Failed);
    fs::write(root.join("ready"), b"ready").unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn dead_process_releases_owner_and_saved_result_recovers_exactly_once() {
    let fixture = Fixture::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "owner_process_fixture"])
        .env("ALFREDO_RECOVERY_FIXTURE", &fixture.root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !fixture.root.join("ready").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let ready = fixture.root.join("ready").exists();
    let active = ready.then(|| fixture.store.recover(1));
    let observation = ready.then(|| {
        fixture
            .store
            .run_observations(&fixture.store.snapshot().unwrap())
    });
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(ready, "owner fixture did not start");
    assert!(active.unwrap().unwrap_err().contains("still active"));
    assert!(observation.unwrap()[&1].contains("owner active"));
    let pending = fixture.store.snapshot().unwrap();
    assert!(fixture.store.run_observations(&pending)[&1].contains("saved result available"));
    let (recovered, _) = fixture.store.recover(1).unwrap();
    assert_eq!(recovered.tasks[0].status, TaskStatus::Failed);
    assert_eq!(recovered.revision, pending.revision + 1);
    let (replayed, _) = fixture.store.recover(1).unwrap();
    assert_eq!(replayed.revision, recovered.revision);
    assert_eq!(replayed.receipts.len(), recovered.receipts.len());
}

#[test]
fn missing_truncated_or_false_success_evidence_never_changes_claim() {
    let fixture = Fixture::new();
    let owner = fixture.store.claim_worker(1).unwrap();
    let run = claim(&fixture.store);
    drop(owner);
    let before = serde_json::to_vec(&fixture.store.snapshot().unwrap()).unwrap();
    let error = fixture.store.recover(1).unwrap_err();
    assert!(error.contains("Outcome unknown"), "{error}");
    save(&fixture.store, &run, TaskStatus::ReviewReady);
    assert!(fixture
        .store
        .recover(1)
        .unwrap_err()
        .contains("successful bounded check"));
    let artifact = fixture
        .store
        .run_directory(&run)
        .unwrap()
        .join("evidence.json");
    fs::write(&artifact, b"{truncated").unwrap();
    assert!(fixture.store.recover(1).is_err());
    assert_eq!(fs::read(artifact).unwrap(), b"{truncated");
    assert_eq!(
        serde_json::to_vec(&fixture.store.snapshot().unwrap()).unwrap(),
        before
    );
    assert!(fixture
        .store
        .run_observations(&fixture.store.snapshot().unwrap())[&1]
        .contains("outcome unknown"));
}

#[test]
fn legacy_claim_without_owner_marker_cannot_be_adopted_by_a_new_launch() {
    let fixture = Fixture::new();
    let run = claim(&fixture.store);
    save(&fixture.store, &run, TaskStatus::Failed);
    assert!(fixture.store.claim_worker(1).is_err());
    assert!(fixture.store.recover(1).unwrap_err().contains("legacy run"));
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Running
    );
}

#[test]
fn reopened_terminal_offers_recovery_and_renders_acknowledged_result() {
    use alfredo_tui::{model::App, task_control::TaskControl, ui};
    use ratatui::{backend::TestBackend, Terminal};
    let fixture = Fixture::new();
    let owner = fixture.store.claim_worker(1).unwrap();
    let run = claim(&fixture.store);
    save(&fixture.store, &run, TaskStatus::Failed);
    drop(owner);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(fixture.store.clone());
    control.command(&runtime, "/tasks", "fixture").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let app = App::new("fixture".into());
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
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
    assert!(text.contains("saved result available"));
    assert!(text.contains("/recover ID"));
    control.command(&runtime, "/recover 1", "fixture").unwrap();
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        control.snapshot.unwrap().tasks[0].status,
        TaskStatus::Failed
    );
    assert!(control.notice.contains("no effects replayed"));
}

#[test]
fn brief_storage_contention_waits_for_release_without_losing_the_request() {
    let fixture = Fixture::new();
    let run_path = fixture.store.run_directory("task-1-run-4").unwrap();
    let namespace = run_path.parent().unwrap().parent().unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(namespace.join("tasks.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        drop(lock);
    });
    let result = fixture.store.snapshot();
    release.join().unwrap();
    assert_eq!(result.unwrap().revision, 3);
}

#[cfg(unix)]
#[test]
fn fork_inherited_descriptor_does_not_extend_released_worker_ownership() {
    let fixture = Fixture::new();
    let owner = fixture.store.claim_worker(1).unwrap();
    let _run = claim(&fixture.store);
    let mut pipe = [-1; 2];
    // Child uses only async-signal-safe libc calls after fork; no Rust runtime work.
    assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0);
    if pid == 0 {
        unsafe {
            libc::close(pipe[1]);
            let mut byte = 0u8;
            libc::read(pipe[0], (&mut byte as *mut u8).cast(), 1);
            libc::_exit(0);
        }
    }
    unsafe {
        libc::close(pipe[0]);
    }
    struct Child {
        pid: libc::pid_t,
        release: libc::c_int,
    }
    impl Drop for Child {
        fn drop(&mut self) {
            unsafe {
                let byte = 1u8;
                libc::write(self.release, (&byte as *const u8).cast(), 1);
                libc::close(self.release);
                while libc::waitpid(self.pid, std::ptr::null_mut(), 0) < 0 {
                    if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                        break;
                    }
                }
            }
        }
    }
    let child = Child {
        pid,
        release: pipe[1],
    };
    assert!(fixture
        .store
        .recover(1)
        .unwrap_err()
        .contains("still active"));
    drop(owner);
    let error = fixture.store.recover(1).unwrap_err();
    // Keep the child's inherited descriptor open until AFTER the observation.
    drop(child);
    assert!(
        error.contains("Outcome unknown"),
        "Released owner still blocks recovery: {error}"
    );
}

fn boundary_run(fixture: &Fixture) -> (std::path::PathBuf, alfredo_tui::tasks::WorkerOwner) {
    let owner = fixture.store.claim_worker(1).unwrap();
    let run = claim(&fixture.store);
    let directory = fixture.store.run_directory(&run).unwrap();
    fs::create_dir_all(&directory).unwrap();
    alfredo_tui::run_boundary::record_start(
        &directory,
        &fixture.store.snapshot().unwrap().tasks[0],
    )
    .unwrap();
    (directory, owner)
}

#[test]
fn interrupted_before_check_recovers_once_and_repair_still_requires_approval() {
    let fixture = Fixture::new();
    let (directory, owner) = boundary_run(&fixture);
    fs::write(directory.join("partial.txt"), "retained partial work").unwrap();
    assert!(fixture
        .store
        .recover(1)
        .unwrap_err()
        .contains("still active"));
    assert!(!directory.join("evidence.json").exists());
    drop(owner);
    let before = fixture.store.snapshot().unwrap();
    assert!(fixture.store.run_observations(&before)[&1].contains("before check launch"));
    assert!(
        !directory.join("evidence.json").exists(),
        "observation must remain read-only"
    );
    let (recovered, detail) = fixture.store.recover(1).unwrap();
    assert!(detail.contains("before check launch"));
    assert_eq!(recovered.tasks[0].status, TaskStatus::Failed);
    assert_eq!(recovered.revision, before.revision + 1);
    let evidence: Evidence = serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    assert!(evidence.check.is_none());
    assert!(evidence.candidate_commit.is_none());
    assert!(evidence.patch.is_empty());
    assert_eq!(
        fs::read_to_string(directory.join("partial.txt")).unwrap(),
        "retained partial work"
    );
    let (again, _) = fixture.store.recover(1).unwrap();
    assert_eq!(
        serde_json::to_vec(&again).unwrap(),
        serde_json::to_vec(&recovered).unwrap()
    );
    let (repaired, _) = fixture
        .store
        .transact(Request {
            correlation: "repair-after-interruption".into(),
            expected_revision: recovered.revision,
            action: Action::Repair {
                task: 1,
                reason: "Retry from baseline after interrupted planning".into(),
            },
        })
        .unwrap();
    assert_eq!(repaired.tasks[1].status, TaskStatus::Proposed);
    assert_eq!(repaired.tasks[1].repair_of, Some(1));
    assert_eq!(repaired.tasks[1].policy, repaired.tasks[0].policy);
    assert!(repaired.tasks[1].run.is_none());
}

#[test]
fn possible_launch_bad_boundary_and_bad_evidence_never_authorize_reconstruction() {
    for case in [
        "intent",
        "partial-intent",
        "wrong-run",
        "wrong-baseline",
        "wrong-task",
        "future-version",
        "truncated",
        "bad-evidence",
    ] {
        let fixture = Fixture::new();
        let (directory, owner) = boundary_run(&fixture);
        let task = fixture.store.snapshot().unwrap().tasks[0].clone();
        let boundary = directory.join("execution-boundary.json");
        assert!(alfredo_tui::run_boundary::record_start(&directory, &task).is_err());
        match case {
            "intent" => alfredo_tui::run_boundary::record_check_intent(&directory, &task).unwrap(),
            "partial-intent" => fs::write(directory.join("check-launch-intent.json"), b"").unwrap(),
            "wrong-run" | "wrong-baseline" | "wrong-task" | "future-version" => {
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&boundary).unwrap()).unwrap();
                if case == "wrong-run" {
                    value["run"] = "task-other".into();
                } else if case == "wrong-baseline" {
                    value["baseline"] = "b".repeat(40).into();
                } else if case == "wrong-task" {
                    value["task"] = 2.into();
                } else {
                    value["schema_version"] = 2.into();
                }
                fs::write(&boundary, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "truncated" => fs::write(&boundary, b"{").unwrap(),
            "bad-evidence" => fs::write(directory.join("evidence.json"), b"{").unwrap(),
            _ => unreachable!(),
        }
        drop(owner);
        let before = fixture.store.snapshot().unwrap();
        assert!(fixture.store.recover(1).is_err(), "{case}");
        assert_eq!(
            serde_json::to_vec(&fixture.store.snapshot().unwrap()).unwrap(),
            serde_json::to_vec(&before).unwrap(),
            "{case}"
        );
        assert!(fixture.store.run_observations(&before)[&1].contains("unknown"));
        if case == "bad-evidence" {
            assert_eq!(fs::read(directory.join("evidence.json")).unwrap(), b"{");
        } else {
            assert!(!directory.join("evidence.json").exists(), "{case}");
        }
    }
}
