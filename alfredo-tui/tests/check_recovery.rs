//! Process-death evidence uses the same provider/checkpoint helper as the worker.
//! All crash controls live in this integration-test executable.
use alfredo_tui::{
    execution::{ExecutionCallbacks, ExecutionReceipt},
    run_boundary,
    tasks::{Action, Request, Snapshot, Task, TaskStatus, TaskStore},
    worker::{self, Evidence},
};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Barrier,
    },
    thread,
    time::Duration,
};

static ID: AtomicU64 = AtomicU64::new(0);
const MISSION: &str = "check-recovery";
const READY: &str = "CHECK_RECOVERY_CUT_REACHED";

struct Fixture {
    root: PathBuf,
    store: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        Self::with_waiting_check(false)
    }
    fn with_waiting_check(wait: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-check-recovery-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("workspace")).unwrap();
        let store = open_store(&root);
        let fixture = Self { root, store };
        fixture.action(Action::Propose {
            title: "Recover an approved counted check".into(),
            model: "fixture".into(),
            dependencies: vec![],
        });
        fixture.action(Action::Permit {
            task: 1,
            policy: alfredo_tui::tasks::WorkPolicy {
                files: vec!["count".into()],
                check: vec![
                    "/usr/bin/python3".into(),
                    "-B".into(),
                    "-c".into(),
                    format!(
                        "from pathlib import Path; p=Path('count'); p.write_text(str(int(p.read_text())+1)); print('COUNTED_CHECK_RESULT', flush=True){}",
                        if wait { "; import signal; signal.pause()" } else { "" }
                    ),
                ],
            },
        });
        fixture.action(Action::Approve { task: 1 });
        fixture
    }
    fn action(&self, action: Action) -> Snapshot {
        transact(&self.store, action)
    }
    fn run(&self) -> (Task, PathBuf) {
        prepare_run(&self.store)
    }
    fn completed(&self) -> (Task, PathBuf, ExecutionReceipt) {
        let owner = self.store.claim_worker(1).unwrap();
        let (task, directory) = self.run();
        let receipt = execute(&directory, &task);
        drop(owner);
        (task, directory, receipt)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn open_store(root: &Path) -> TaskStore {
    TaskStore::new(&root.join("state"), &root.join("workspace"), MISSION).unwrap()
}
fn transact(store: &TaskStore, action: Action) -> Snapshot {
    let revision = store.snapshot().unwrap().revision;
    store
        .transact(Request {
            correlation: format!("fixture-{revision}"),
            expected_revision: revision,
            action,
        })
        .unwrap()
        .0
}
fn prepare_run(store: &TaskStore) -> (Task, PathBuf) {
    let snapshot = transact(
        store,
        Action::Start {
            task: 1,
            baseline: "a".repeat(40),
            inputs: vec![],
        },
    );
    let task = snapshot.tasks[0].clone();
    let directory = store.run_directory(&task.run.as_ref().unwrap().id).unwrap();
    fs::create_dir_all(directory.join("worktree")).unwrap();
    // The provider requires a read-only .git binding. This fixture exercises no Git.
    fs::write(directory.join("worktree/.git"), "fixture git metadata\n").unwrap();
    fs::write(directory.join("worktree/count"), "0").unwrap();
    run_boundary::record_start(&directory, &task).unwrap();
    (task, directory)
}
fn execute(directory: &Path, task: &Task) -> ExecutionReceipt {
    let request = worker::check_request(&directory.join("worktree"), task, MISSION).unwrap();
    let receipt = run_boundary::execute_check(
        directory,
        task,
        MISSION,
        &request,
        &mut ExecutionCallbacks::none(),
    )
    .unwrap();
    assert_eq!(receipt.status, "completed", "{receipt:?}");
    assert_eq!(receipt.exit_code, Some(0));
    receipt
}
fn finish_count(snapshot: &Snapshot) -> usize {
    snapshot
        .receipts
        .iter()
        .filter(|receipt| matches!(receipt.request.action, Action::Finish { task: 1, .. }))
        .count()
}
fn assert_dependent_cannot_start(store: &TaskStore) {
    let before = store.snapshot().unwrap();
    let run = before.tasks[0].run.as_ref().unwrap();
    assert!(store
        .transact(Request {
            correlation: format!("blocked-dependent-{}", before.revision),
            expected_revision: before.revision,
            action: Action::Start {
                task: 2,
                baseline: "a".repeat(40),
                inputs: vec![alfredo_tui::tasks::DependencyInput {
                    task: 1,
                    source_task: None,
                    run: run.id.clone(),
                    evidence_sha256: run.evidence_sha256.clone().unwrap(),
                    candidate: "b".repeat(40),
                }],
            },
        })
        .is_err());
    assert_eq!(store.snapshot().unwrap().revision, before.revision);
    assert!(store.snapshot().unwrap().tasks[1].run.is_none());
}
fn save_final(directory: &Path, task: &Task, receipt: ExecutionReceipt) {
    let run = task.run.as_ref().unwrap();
    fs::write(
        directory.join("evidence.json"),
        serde_json::to_vec(&Evidence {
            agent: None,
            run: run.id.clone(),
            baseline: run.baseline.clone(),
            status: TaskStatus::Failed,
            detail: "Fixture finalization failed after the completed check".into(),
            patch: String::new(),
            candidate_commit: None,
            model_metrics: None,
            generation: None,
            check: Some(receipt),
        })
        .unwrap(),
    )
    .unwrap();
}
fn signal_and_park() -> ! {
    println!("{READY}");
    std::io::stdout().flush().unwrap();
    loop {
        thread::park();
    }
}

#[test]
#[ignore = "test-only subprocess crash fixture"]
fn check_process_fixture() {
    let root = PathBuf::from(std::env::var_os("ALFREDO_CHECK_CRASH_ROOT").unwrap());
    let cut = std::env::var("ALFREDO_CHECK_CRASH_CUT").unwrap();
    let store = open_store(&root);
    let _owner = store.claim_worker(1).unwrap();
    let (task, directory) = prepare_run(&store);
    if cut == "before-launch" {
        signal_and_park();
    }
    let request = worker::check_request(&directory.join("worktree"), &task, MISSION).unwrap();
    let mut seen_output = Vec::new();
    let mut output = |_, bytes: &[u8]| {
        seen_output.extend(bytes.iter().copied().take(1024 - seen_output.len()));
        if cut == "in-flight"
            && String::from_utf8_lossy(&seen_output).contains("COUNTED_CHECK_RESULT")
        {
            signal_and_park();
        }
    };
    let receipt = run_boundary::execute_check(
        &directory,
        &task,
        MISSION,
        &request,
        &mut ExecutionCallbacks {
            output: Some(&mut output),
            process_started: None,
            poll: None,
        },
    )
    .unwrap();
    assert_eq!(receipt.status, "completed", "{receipt:?}");
    assert_eq!(receipt.exit_code, Some(0));
    if cut == "after-evidence" || cut == "after-finish" {
        save_final(&directory, &task, receipt);
    }
    if cut == "after-finish" {
        drop(_owner);
        store.recover(1).unwrap();
    }
    signal_and_park();
}

struct CrashedChild(Child);
impl CrashedChild {
    fn spawn(fixture: &Fixture, cut: &str) -> Self {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "check_process_fixture",
                "--nocapture",
            ])
            .env("ALFREDO_CHECK_CRASH_ROOT", &fixture.root)
            .env("ALFREDO_CHECK_CRASH_CUT", cut)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut child = Self(child);
        let stdout = child.0.stdout.take().unwrap();
        let (ready, wait) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.is_ok_and(|line| line.contains(READY)) {
                    let _ = ready.send(());
                    break;
                }
            }
        });
        wait.recv_timeout(Duration::from_secs(15))
            .expect("test child did not reach its deterministic crash cut");
        child
    }
    fn kill(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}
impl Drop for CrashedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn killed_after_durable_check_recovers_once_without_rerunning_successful_effect() {
    let fixture = Fixture::new();
    let mut child = CrashedChild::spawn(&fixture, "after-check");
    let running = fixture.store.snapshot().unwrap();
    let task = &running.tasks[0];
    let directory = fixture
        .store
        .run_directory(&task.run.as_ref().unwrap().id)
        .unwrap();
    let intent = fs::read(directory.join("check-launch-intent.json")).unwrap();
    let checkpoint = fs::read(directory.join("check-result.json")).unwrap();
    assert!(!directory.join("evidence.json").exists());
    assert!(fixture
        .store
        .recover(1)
        .unwrap_err()
        .contains("still active"));
    assert_eq!(finish_count(&fixture.store.snapshot().unwrap()), 0);
    child.kill();

    let reopened = open_store(&fixture.root);
    assert!(reopened.run_observations(&running)[&1].contains("stopped after check"));
    let (recovered, message) = reopened.recover(1).unwrap();
    assert!(message.contains("no effects replayed"), "{message}");
    assert_eq!(recovered.tasks[0].status, TaskStatus::Failed);
    assert_eq!(recovered.revision, running.revision + 1);
    assert_eq!(finish_count(&recovered), 1);
    assert_eq!(
        recovered.receipts.last().unwrap().request.correlation,
        format!("finish:{}", task.run.as_ref().unwrap().id)
    );
    let evidence: Evidence = serde_json::from_str(&reopened.evidence(1).unwrap()).unwrap();
    assert!(evidence.detail.contains("candidate not finalized"));
    assert!(evidence.patch.is_empty());
    assert!(evidence.candidate_commit.is_none());
    let check = evidence.check.unwrap();
    assert_eq!(check.status, "completed");
    assert_eq!(check.exit_code, Some(0));
    assert!(check.stdout.contains("COUNTED_CHECK_RESULT"));
    assert_eq!(
        fs::read_to_string(directory.join("worktree/count")).unwrap(),
        "1"
    );
    assert_eq!(
        fs::read(directory.join("check-launch-intent.json")).unwrap(),
        intent
    );
    assert_eq!(
        fs::read(directory.join("check-result.json")).unwrap(),
        checkpoint
    );
    let (replayed, _) = open_store(&fixture.root).recover(1).unwrap();
    assert_eq!(
        serde_json::to_vec(&replayed).unwrap(),
        serde_json::to_vec(&recovered).unwrap()
    );
    assert_eq!(
        fs::read_to_string(directory.join("worktree/count")).unwrap(),
        "1"
    );
}

fn assert_refused_unchanged(fixture: &Fixture, directory: &Path) {
    let before = serde_json::to_vec(&fixture.store.snapshot().unwrap()).unwrap();
    let artifacts: Vec<_> = [
        "execution-boundary.json",
        "check-launch-intent.json",
        "check-result.json",
        "evidence.json",
        "worktree/count",
    ]
    .into_iter()
    .map(|name| (name, fs::read(directory.join(name)).ok()))
    .collect();
    let reopened = open_store(&fixture.root);
    assert!(reopened.recover(1).is_err());
    assert_eq!(
        serde_json::to_vec(&reopened.snapshot().unwrap()).unwrap(),
        before
    );
    for (name, bytes) in artifacts {
        assert_eq!(
            fs::read(directory.join(name)).ok(),
            bytes,
            "{name} changed on refusal"
        );
    }
    assert!(
        reopened.run_observations(&reopened.snapshot().unwrap())[&1].contains("outcome unknown")
    );
}

#[test]
fn death_before_launch_is_recoverable_but_an_in_flight_check_remains_unknown() {
    for cut in ["before-launch", "in-flight"] {
        let fixture = Fixture::with_waiting_check(cut == "in-flight");
        let mut child = CrashedChild::spawn(&fixture, cut);
        let running = fixture.store.snapshot().unwrap();
        let directory = fixture
            .store
            .run_directory(&running.tasks[0].run.as_ref().unwrap().id)
            .unwrap();
        child.kill();
        assert!(!directory.join("check-result.json").exists());
        if cut == "before-launch" {
            assert!(!directory.join("check-launch-intent.json").exists());
            let (recovered, _) = open_store(&fixture.root).recover(1).unwrap();
            assert_eq!(recovered.tasks[0].status, TaskStatus::Failed);
            let evidence: Evidence =
                serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
            assert!(evidence.check.is_none());
            assert_eq!(
                fs::read_to_string(directory.join("worktree/count")).unwrap(),
                "0"
            );
        } else {
            assert!(directory.join("check-launch-intent.json").exists());
            assert_eq!(
                fs::read_to_string(directory.join("worktree/count")).unwrap(),
                "1"
            );
            assert_refused_unchanged(&fixture, &directory);
        }
    }
}

#[test]
fn death_after_final_evidence_and_after_finish_preserves_the_final_result() {
    for cut in ["after-evidence", "after-finish"] {
        let fixture = Fixture::new();
        let mut child = CrashedChild::spawn(&fixture, cut);
        let before = fixture.store.snapshot().unwrap();
        let directory = fixture
            .store
            .run_directory(&before.tasks[0].run.as_ref().unwrap().id)
            .unwrap();
        let evidence = fs::read(directory.join("evidence.json")).unwrap();
        child.kill();
        // Intact final evidence remains authoritative even if an older checkpoint
        // is subsequently damaged; recovery must not replace the final result.
        fs::write(directory.join("check-result.json"), b"{damaged-checkpoint").unwrap();
        let (recovered, _) = open_store(&fixture.root).recover(1).unwrap();
        assert_eq!(recovered.tasks[0].status, TaskStatus::Failed);
        assert_eq!(finish_count(&recovered), 1);
        assert_eq!(
            recovered.revision,
            before.revision + u64::from(cut == "after-evidence")
        );
        assert_eq!(fs::read(directory.join("evidence.json")).unwrap(), evidence);
        assert_eq!(
            fs::read(directory.join("check-result.json")).unwrap(),
            b"{damaged-checkpoint"
        );
        assert!(fixture
            .store
            .evidence(1)
            .unwrap()
            .contains("Fixture finalization failed"));
        assert_eq!(
            fs::read_to_string(directory.join("worktree/count")).unwrap(),
            "1"
        );
    }
}

#[test]
fn checkpoint_publication_failure_never_returns_success_or_overwrites_partial_bytes() {
    let fixture = Fixture::new();
    let owner = fixture.store.claim_worker(1).unwrap();
    let (task, directory) = fixture.run();
    let request = worker::check_request(&directory.join("worktree"), &task, MISSION).unwrap();
    let mut started = |_| {
        // Deterministically collide after intent publication and real process launch.
        fs::write(directory.join("check-result.json"), b"{partial-checkpoint").unwrap();
        Ok(())
    };
    let result = run_boundary::execute_check(
        &directory,
        &task,
        MISSION,
        &request,
        &mut ExecutionCallbacks {
            output: None,
            process_started: Some(&mut started),
            poll: None,
        },
    );
    assert!(
        result.is_err(),
        "a checkpoint publication failure returned a success receipt"
    );
    assert_eq!(
        fs::read_to_string(directory.join("worktree/count")).unwrap(),
        "1"
    );
    assert_eq!(
        fs::read(directory.join("check-result.json")).unwrap(),
        b"{partial-checkpoint"
    );
    assert!(!directory.join("evidence.json").exists());
    drop(owner);
    assert_refused_unchanged(&fixture, &directory);
}

#[test]
fn concurrent_recovery_and_repair_keep_the_original_failed_and_dependents_blocked() {
    let fixture = Fixture::new();
    fixture.action(Action::Propose {
        title: "Needs the original accepted result".into(),
        model: "fixture".into(),
        dependencies: vec![1],
    });
    fixture.action(Action::Permit {
        task: 2,
        policy: fixture.store.snapshot().unwrap().tasks[0]
            .policy
            .clone()
            .unwrap(),
    });
    fixture.action(Action::Approve { task: 2 });
    let (_, directory, _) = fixture.completed();
    let running = fixture.store.snapshot().unwrap();
    let barrier = Arc::new(Barrier::new(9));
    let jobs: Vec<_> = (0..8)
        .map(|_| {
            let store = open_store(&fixture.root);
            let ready = barrier.clone();
            thread::spawn(move || {
                ready.wait();
                store.recover(1)
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = jobs.into_iter().map(|job| job.join().unwrap()).collect();
    assert!(results.iter().any(Result::is_ok));
    for result in results {
        if let Err(error) = result {
            assert!(error.contains("still active"), "{error}");
        }
    }
    let recovered = fixture.store.snapshot().unwrap();
    assert_eq!(recovered.revision, running.revision + 1);
    assert_eq!(finish_count(&recovered), 1);
    assert_eq!(recovered.tasks[0].status, TaskStatus::Failed);
    assert!(recovered.dependency_source(1).is_err());
    assert_dependent_cannot_start(&fixture.store);

    let repaired = fixture.action(Action::Repair {
        task: 1,
        reason: "Inspect interrupted finalization".into(),
    });
    assert_eq!(repaired.tasks[2].status, TaskStatus::Proposed);
    assert_eq!(repaired.tasks[2].repair_of, Some(1));
    assert!(repaired.tasks[2].run.is_none());
    assert!(fixture.store.claim_worker(3).is_err());
    assert_eq!(
        fs::read_to_string(directory.join("worktree/count")).unwrap(),
        "1"
    );
    fixture.action(Action::Approve { task: 3 });
    let repair_owner = fixture.store.claim_worker(3).unwrap();
    drop(repair_owner);
    let after_approval = fixture.store.snapshot().unwrap();
    assert_eq!(after_approval.tasks[0].status, TaskStatus::Failed);
    assert_eq!(after_approval.tasks[1].status, TaskStatus::Approved);
    assert_eq!(after_approval.tasks[2].status, TaskStatus::Approved);
    assert!(after_approval.dependency_source(1).is_err());
    assert_dependent_cannot_start(&fixture.store);
    assert_eq!(finish_count(&after_approval), 1);
}

#[test]
fn absent_legacy_partial_oversized_or_foreign_artifacts_do_not_grant_recovery() {
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new();
    let (_, directory, _) = fixture.completed();
    let intent_path = directory.join("check-launch-intent.json");
    let result_path = directory.join("check-result.json");
    let original_intent = fs::read(&intent_path).unwrap();
    let original_result = fs::read(&result_path).unwrap();

    fs::remove_file(&result_path).unwrap();
    assert_refused_unchanged(&fixture, &directory);
    fs::write(&result_path, &original_result).unwrap();
    fs::remove_file(&intent_path).unwrap();
    assert_refused_unchanged(&fixture, &directory);
    fs::write(&intent_path, &original_intent).unwrap();

    for path in [&intent_path, &result_path] {
        fs::write(path, b"{truncated").unwrap();
        assert_refused_unchanged(&fixture, &directory);
        fs::write(&intent_path, &original_intent).unwrap();
        fs::write(&result_path, &original_result).unwrap();
    }
    fs::write(&result_path, vec![b' '; 1024 * 1024]).unwrap();
    assert_refused_unchanged(&fixture, &directory);
    fs::write(&result_path, &original_result).unwrap();
    fs::write(
        &intent_path,
        fs::read(directory.join("execution-boundary.json")).unwrap(),
    )
    .unwrap();
    assert_refused_unchanged(&fixture, &directory);
    fs::write(&intent_path, &original_intent).unwrap();

    // Update the outer hash too: refusal must verify canonical identities, not
    // merely notice a stale digest left by this test's corruption.
    for field in [
        "task",
        "run",
        "baseline",
        "mission",
        "schema_version",
        "contract_version",
        "request",
    ] {
        let mut intent: serde_json::Value = serde_json::from_slice(&original_intent).unwrap();
        match field {
            "task" => intent[field] = 99.into(),
            "schema_version" | "contract_version" => intent[field] = 99.into(),
            "baseline" => intent[field] = "b".repeat(40).into(),
            "request" => intent[field]["working_directory"] = "/tmp/foreign-worktree".into(),
            _ => intent[field] = format!("foreign-{field}").into(),
        }
        let bytes = serde_json::to_vec(&intent).unwrap();
        let mut result: serde_json::Value = serde_json::from_slice(&original_result).unwrap();
        result["intent_sha256"] = format!("{:x}", Sha256::digest(&bytes)).into();
        fs::write(&intent_path, bytes).unwrap();
        fs::write(&result_path, serde_json::to_vec(&result).unwrap()).unwrap();
        assert_refused_unchanged(&fixture, &directory);
    }
    fs::write(&intent_path, &original_intent).unwrap();
    fs::write(&result_path, &original_result).unwrap();
    assert_eq!(
        fixture.store.recover(1).unwrap().0.tasks[0].status,
        TaskStatus::Failed
    );
}

#[test]
fn malformed_final_evidence_is_never_replaced_by_a_valid_checkpoint() {
    let fixture = Fixture::new();
    let (_, directory, _) = fixture.completed();
    fs::write(directory.join("evidence.json"), b"{damaged-final-evidence").unwrap();
    assert_refused_unchanged(&fixture, &directory);
}

#[cfg(unix)]
#[test]
fn symlink_and_special_result_artifacts_are_refused_without_touching_their_targets() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let (_, directory, _) = fixture.completed();
    let result_path = directory.join("check-result.json");
    let target = fixture.root.join("retained-check-result.json");
    fs::rename(&result_path, &target).unwrap();
    let bytes = fs::read(&target).unwrap();
    symlink(&target, &result_path).unwrap();
    assert_refused_unchanged(&fixture, &directory);
    assert!(fs::symlink_metadata(&result_path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(&target).unwrap(), bytes);
    fs::remove_file(&result_path).unwrap();
    fs::create_dir(&result_path).unwrap();
    assert_refused_unchanged(&fixture, &directory);
    assert!(result_path.is_dir());
    assert_eq!(fs::read(&target).unwrap(), bytes);
}
