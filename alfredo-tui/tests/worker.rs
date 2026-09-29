use alfredo_tui::{
    provider::Ollama,
    tasks::{Action, Request, TaskStatus, TaskStore, WorkPolicy},
    worker::{self, FileEdit, FilePlan},
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    store: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        Self::with_model("fixture")
    }
    fn with_model(model: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-worker-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let workspace = root.join("workspace");
        fs::create_dir(&workspace).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
        ] {
            assert!(Command::new("git")
                .arg("-C")
                .arg(&workspace)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        fs::write(workspace.join(".gitattributes"), "* text eol=lf\n").unwrap();
        fs::write(workspace.join("calc.py"), "def answer():\n    return 0\n").unwrap();
        assert!(Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(["add", "."])
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(["-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture"])
            .status()
            .unwrap()
            .success());
        let store = TaskStore::new(&root.join("state"), &workspace, "mission").unwrap();
        let fixture = Self {
            root,
            workspace,
            store,
        };
        fixture.action(Action::Propose {
            title: "Make answer return 42 and add notes.txt".into(),
            model: model.into(),
            dependencies: vec![],
        });
        fixture
    }
    fn action(&self, action: Action) {
        let revision = self.store.snapshot().unwrap().revision;
        self.store
            .transact(Request {
                correlation: format!("action-{revision}"),
                expected_revision: revision,
                action,
            })
            .unwrap();
    }
    fn permit(&self) {
        self.action(Action::Permit {
            task: 1,
            policy: WorkPolicy {
                files: vec!["calc.py".into(), "notes.txt".into()],
                check: vec![
                    "/usr/bin/python3".into(),
                    "-B".into(),
                    "-c".into(),
                    "from calc import answer; from pathlib import Path; assert answer() == 42; assert Path('notes.txt').read_text().strip(); print('CHECK_OK')".into(),
                ],
            },
        });
        self.action(Action::Approve { task: 1 });
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn server(
    plan: FilePlan,
    delay: Duration,
) -> (
    Ollama,
    thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<()>,
) {
    server_with_source(plan, delay, "return 0")
}

fn server_with_source(
    plan: FilePlan,
    delay: Duration,
    expected: &str,
) -> (
    Ollama,
    thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<()>,
) {
    server_with_capture(plan, delay, expected, None)
}
fn server_with_capture(
    plan: FilePlan,
    delay: Duration,
    expected: &str,
    capture: Option<std::sync::mpsc::Sender<serde_json::Value>>,
) -> (
    Ollama,
    thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<()>,
) {
    server_with_completion(plan, delay, expected, capture, true)
}

fn server_with_completion(
    plan: FilePlan,
    delay: Duration,
    expected: &str,
    capture: Option<std::sync::mpsc::Sender<serde_json::Value>>,
    complete: bool,
) -> (
    Ollama,
    thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<()>,
) {
    server_with_reason(plan, delay, expected, capture, complete, None)
}

fn server_with_reason(
    plan: FilePlan,
    delay: Duration,
    expected: &str,
    capture: Option<std::sync::mpsc::Sender<serde_json::Value>>,
    complete: bool,
    done_reason: Option<&'static str>,
) -> (
    Ollama,
    thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<()>,
) {
    serve_reply(
        serde_json::to_string(&plan).unwrap(),
        delay,
        expected,
        capture,
        complete,
        done_reason,
    )
}

/// One-request worker fixture replying with `content` verbatim.
fn serve_reply(
    content: String,
    delay: Duration,
    expected: &str,
    capture: Option<std::sync::mpsc::Sender<serde_json::Value>>,
    complete: bool,
    done_reason: Option<&'static str>,
) -> (
    Ollama,
    thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<()>,
) {
    let expected = expected.to_string();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (notify, received) = std::sync::mpsc::channel();
    let job = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(e) => panic!("Fixture server did not receive worker request: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut header = Vec::new();
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        let header = String::from_utf8(header).unwrap();
        let length: usize = header
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_string)
            })
            .unwrap()
            .parse()
            .unwrap();
        let mut request = vec![0; length];
        stream.read_exact(&mut request).unwrap();
        let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
        // Default worker requests ask for FILE blocks without a schema; the
        // legacy JSON request is explicit.
        match request.get("format") {
            Some(format) => assert_eq!(format["required"], serde_json::json!(["files"])),
            None => assert!(
                request["messages"].as_array().unwrap().last().unwrap()["content"]
                    .as_str()
                    .unwrap()
                    .contains("=== END FILE ===")
            ),
        }
        // Repairs raise sampling temperature within the bounded schedule.
        assert!(request["options"]["temperature"]
            .as_f64()
            .is_some_and(|t| (0.0..=0.8).contains(&t)));
        for expected in expected.split('\0') {
            assert!(
                request["messages"].as_array().unwrap().last().unwrap()["content"]
                    .as_str()
                    .unwrap()
                    .contains(expected)
            );
        }
        let prompt = request["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap();
        if prompt.starts_with("Implement this task: Repair #1:") {
            assert!(
                prompt.contains("REPAIR CONTEXT")
                    && prompt.contains("Prior task #1")
                    && (prompt.contains("Prior model exchange did not complete")
                        || prompt.contains("CHECK_OK")
                        || prompt.contains("Worker stream ended before completion")
                        || prompt.contains("AssertionError")
                        || prompt.contains("-token limit")
                        || prompt.contains("(truncated)"))
            );
        }
        if let Some(capture) = capture {
            capture.send(request.clone()).unwrap();
        }
        let _ = notify.send(());
        thread::sleep(delay);
        let mut frame = serde_json::json!({"message":{"content":content},"done":complete,"load_duration":500000000,"eval_duration":2000000000,"eval_count":40});
        if let Some(reason) = done_reason {
            frame["done_reason"] = reason.into();
        }
        let body = serde_json::to_string(&frame).unwrap();
        let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
    });
    (
        Ollama::new(&endpoint, Duration::from_secs(5)).unwrap(),
        job,
        received,
    )
}

fn good_plan() -> FilePlan {
    FilePlan {
        files: vec![
            FileEdit {
                path: "calc.py".into(),
                content: "def answer():\n    return 42\n".into(),
            },
            FileEdit {
                path: "notes.txt".into(),
                content: "Worker created this file.\n".into(),
            },
        ],
    }
}

#[tokio::test]
async fn real_model_transport_edits_isolated_worktree_checks_and_restores_review_evidence() {
    let fixture = Fixture::new();
    for (key, value) in [
        ("commit.gpgSign", "true"),
        ("gpg.program", "/nonexistent-signing-program"),
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&fixture.workspace)
            .args(["config", key, value])
            .status()
            .unwrap()
            .success());
    }

    fixture.permit();
    let (provider, server, _received) = server(good_plan(), Duration::ZERO);
    let provider = provider.with_structured_thinking(Some(true));
    let (observer, progress) = worker::Observer::channel();
    let (snapshot, detail) = worker::start_observed(
        fixture.store.clone(),
        1,
        "run-one".into(),
        3,
        provider.clone(),
        Arc::new(AtomicBool::new(false)),
        observer,
    )
    .await
    .unwrap();
    assert_eq!(progress.borrow().stage, "Saving evidence and receipt");
    assert!(progress.borrow().received_bytes > 0);
    assert!(progress.borrow().first_content.is_some());
    server.join().unwrap();
    assert_eq!(
        snapshot.tasks[0].status,
        TaskStatus::ReviewReady,
        "{detail}"
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("calc.py")).unwrap(),
        "def answer():\n    return 0\n"
    );
    assert!(!fixture.workspace.join("notes.txt").exists());
    let evidence = fixture.store.evidence(1).unwrap();
    let requested: worker::Evidence = serde_json::from_str(&evidence).unwrap();
    let mut expected = provider.structured_generation();
    expected.answer_format = Some(worker::WorkerFormat::Blocks);
    assert_eq!(requested.generation.as_ref().unwrap(), &expected);

    assert!(evidence.contains("CHECK_OK"));
    assert!(evidence.contains("notes.txt"));
    assert!(evidence.contains("return 42"));
    let mut retained: worker::Evidence = serde_json::from_str(&evidence).unwrap();
    let directory = fixture.store.run_directory(&retained.run).unwrap();
    let boundary: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("execution-boundary.json")).unwrap())
            .unwrap();
    let intent_bytes = fs::read(directory.join("check-launch-intent.json")).unwrap();
    let intent: serde_json::Value = serde_json::from_slice(&intent_bytes).unwrap();
    let checkpoint: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("check-result.json")).unwrap()).unwrap();
    assert_eq!(boundary["schema_version"], 1);
    assert_eq!(intent["schema_version"], 2);
    assert_eq!(intent["contract_version"], 1);
    assert_eq!(checkpoint["schema_version"], 1);
    assert_eq!(intent["mission"], "mission");
    assert_eq!(
        checkpoint["receipt"],
        serde_json::to_value(retained.check.as_ref().unwrap()).unwrap()
    );
    assert_eq!(checkpoint["request_digest"], intent["request_digest"]);
    assert_eq!(
        checkpoint["receipt"]["request_digest"],
        intent["request_digest"]
    );
    {
        use sha2::{Digest, Sha256};
        assert_eq!(
            checkpoint["intent_sha256"],
            format!("{:x}", Sha256::digest(&intent_bytes))
        );
    }
    assert_eq!(intent["run"], retained.run);
    assert_eq!(intent["baseline"], retained.baseline);
    assert_eq!(intent["task"], 1);

    assert_eq!(
        retained.model_metrics.as_ref().unwrap().load_duration,
        Some(500_000_000)
    );
    assert!(retained
        .model_metrics
        .as_ref()
        .unwrap()
        .summary()
        .contains("20.0 generated tokens/s"));
    let candidate = worker::verify_candidate(&fixture.workspace, &retained)
        .await
        .unwrap();
    assert_eq!(candidate.len(), 40);
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&fixture.workspace)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    assert_eq!(git(&["rev-parse", "HEAD"]).trim(), retained.baseline);
    assert_eq!(
        git(&["rev-parse", &format!("refs/alfredo/candidates/{candidate}")]).trim(),
        candidate
    );
    assert!(git(&["status", "--porcelain"]).is_empty());
    assert_eq!(
        git(&["show", &format!("{candidate}:notes.txt")]),
        "Worker created this file.\n"
    );
    let worktree = fixture
        .store
        .run_directory(&retained.run)
        .unwrap()
        .join("worktree");
    fs::write(worktree.join("calc.py"), "changed after evidence").unwrap();
    assert_eq!(
        worker::verify_candidate(&fixture.workspace, &retained)
            .await
            .unwrap(),
        candidate
    );
    retained.patch.push_str("fabricated patch");
    assert!(worker::verify_candidate(&fixture.workspace, &retained)
        .await
        .is_err());
    retained.patch = serde_json::from_str::<worker::Evidence>(&evidence)
        .unwrap()
        .patch;
    retained.baseline = "b".repeat(40);
    assert!(worker::verify_candidate(&fixture.workspace, &retained)
        .await
        .is_err());

    let (replay, _) = worker::start(
        fixture.store.clone(),
        1,
        "run-one".into(),
        3,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    assert_eq!(replay.revision, snapshot.revision);
    let artifact = fixture
        .store
        .run_directory(&snapshot.tasks[0].run.as_ref().unwrap().id)
        .unwrap()
        .join("evidence.json");
    let original = fs::read(&artifact).unwrap();
    let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    changed["generation"]["thinking"] = "off".into();
    fs::write(&artifact, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(fixture
        .store
        .evidence(1)
        .unwrap_err()
        .contains("digest mismatch"));
    fs::write(&artifact, &original).unwrap();
    fs::write(&artifact, b"tampered").unwrap();
    assert!(fixture.store.evidence(1).is_err());
    assert!(fixture
        .store
        .transact(Request {
            correlation: "tampered-review".into(),
            expected_revision: snapshot.revision,
            action: Action::Review {
                task: 1,
                accept: true
            }
        })
        .is_err());
    fs::write(artifact, original).unwrap();
    fixture.action(Action::Review {
        task: 1,
        accept: true,
    });
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Accepted
    );
}

#[tokio::test]
async fn unapproved_model_path_is_rejected_before_any_file_changes() {
    let fixture = Fixture::new();
    fixture.permit();
    let plan = FilePlan {
        files: vec![FileEdit {
            path: "../escape".into(),
            content: "no".into(),
        }],
    };
    let (provider, server, _) = server(plan, Duration::ZERO);
    let (snapshot, _) = worker::start(
        fixture.store.clone(),
        1,
        "bad-plan".into(),
        3,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Failed);
    let directory = fixture
        .store
        .run_directory(&snapshot.tasks[0].run.as_ref().unwrap().id)
        .unwrap();
    assert!(!directory.join("escape").exists());
    assert_eq!(
        fs::read_to_string(directory.join("worktree/calc.py")).unwrap(),
        "def answer():\n    return 0\n"
    );
}

#[tokio::test]
async fn cancellation_during_inference_produces_terminal_receipt_without_edits() {
    let fixture = Fixture::new();
    fixture.permit();
    let (provider, server, received) = server(good_plan(), Duration::from_millis(300));
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let store = fixture.store.clone();
    let (observer, progress) = worker::Observer::channel();
    let worker = tokio::spawn(async move {
        worker::start_observed(store, 1, "cancel".into(), 3, provider, flag, observer).await
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    while received.try_recv().is_err() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(progress.borrow().stage, "Waiting for model server");
    assert_eq!(progress.borrow().received_bytes, 0);
    assert!(progress.borrow().first_content.is_none());
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Running
    );
    cancel.store(true, Ordering::SeqCst);
    let (snapshot, _) = tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    server.join().unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Cancelled);
}

#[tokio::test]
async fn bare_legacy_approval_cannot_start_a_worker() {
    let fixture = Fixture::new();
    fixture.action(Action::Approve { task: 1 });
    let provider = Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap();
    assert!(worker::start(
        fixture.store.clone(),
        1,
        "no-policy".into(),
        2,
        provider,
        Arc::new(AtomicBool::new(false))
    )
    .await
    .unwrap_err()
    .contains("explicit"));
    assert!(fixture.store.snapshot().unwrap().tasks[0].run.is_none());
}

#[test]
fn file_policy_rejects_metadata_traversal_duplicates_and_binary_edits() {
    for file in [
        "../out",
        ".git",
        ".git/config",
        "x/.git",
        "a/../b",
        "/tmp/out",
        "a//b",
        "a\\b",
    ] {
        assert!(WorkPolicy {
            files: vec![file.into()],
            check: vec!["true".into()]
        }
        .validate()
        .is_err());
    }
    let policy = WorkPolicy {
        files: vec!["x".into()],
        check: vec!["true".into()],
    };
    let twice = FilePlan {
        files: vec![
            FileEdit {
                path: "x".into(),
                content: "a".into(),
            },
            FileEdit {
                path: "x".into(),
                content: "b".into(),
            },
        ],
    };
    assert_eq!(
        worker::validate_plan(&twice, &policy).unwrap_err(),
        "Returned x more than once; return each file once with its complete content"
    );
    assert!(worker::validate_plan(
        &FilePlan {
            files: vec![FileEdit {
                path: "x".into(),
                content: "\0".into()
            }]
        },
        &policy
    )
    .is_err());
}

#[tokio::test]
#[ignore = "Requires installed local qwen2.5-coder:14b and Bubblewrap"]
async fn live_local_model_produces_real_edits_and_passing_check() {
    let model = std::env::var("ALFREDO_SMOKE_MODEL").unwrap_or_else(|_| "qwen2.5-coder:14b".into());
    let fixture = Fixture::with_model(&model);
    fixture.permit();
    let provider = Ollama::new("http://127.0.0.1:11434", Duration::from_secs(60)).unwrap();
    let started = Instant::now();
    let (snapshot, detail) = worker::start(
        fixture.store.clone(),
        1,
        "live-worker".into(),
        3,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    println!(
        "Live coding worker: {:?}, {:?}: {detail}",
        started.elapsed(),
        snapshot.tasks[0].status
    );
    if snapshot.tasks[0].status != TaskStatus::ReviewReady {
        let directory = fixture
            .store
            .run_directory(&snapshot.tasks[0].run.as_ref().unwrap().id)
            .unwrap();
        println!(
            "Synthetic fixture response diagnostic: {:?}",
            fs::read_to_string(directory.join("model-response.txt"))
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect::<String>()
        );
    }
    assert_eq!(
        snapshot.tasks[0].status,
        TaskStatus::ReviewReady,
        "{}",
        fixture.store.evidence(1).unwrap()
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("calc.py")).unwrap(),
        "def answer():\n    return 0\n"
    );
}

#[tokio::test]
async fn approved_check_cannot_reach_host_files_network_or_rewrite_git_identity() {
    let fixture = Fixture::new();
    fixture.permit();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let marker = fixture.root.join("host-marker");
    fs::write(&marker, "private").unwrap();
    let code = format!("import os,socket; assert not os.path.exists({:?}); assert not os.access('.git',os.W_OK); s=socket.socket(); s.settimeout(.2); assert s.connect_ex(('127.0.0.1',{port})) != 0", marker.to_str().unwrap());
    fixture.action(Action::Permit {
        task: 1,
        policy: WorkPolicy {
            files: vec!["calc.py".into(), "notes.txt".into()],
            check: vec!["/usr/bin/python3".into(), "-B".into(), "-c".into(), code],
        },
    });
    fixture.action(Action::Approve { task: 1 });
    let (provider, server, _) = server(good_plan(), Duration::ZERO);
    let (snapshot, detail) = worker::start(
        fixture.store.clone(),
        1,
        "sandbox-proof".into(),
        5,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(
        snapshot.tasks[0].status,
        TaskStatus::ReviewReady,
        "{detail}"
    );
    assert_eq!(fs::read_to_string(marker).unwrap(), "private");
}

#[test]
fn terminal_shows_waiting_worker_and_cancellation_until_durable_result() {
    use alfredo_tui::{model::App, task_control::TaskControl, ui};
    use ratatui::{backend::TestBackend, Terminal};
    let fixture = Fixture::new();
    fixture.permit();
    let (provider, server, _received) = server(good_plan(), Duration::from_millis(800));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(fixture.store.clone());
    control.refresh(&runtime);
    let refresh_deadline = Instant::now() + Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(Instant::now() < refresh_deadline);
        thread::sleep(Duration::from_millis(2));
    }
    control.set_provider(provider);
    control.command(&runtime, "/run 1", "fixture").unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !control
        .worker_progress(1)
        .is_some_and(|p| p.starts_with("Waiting for model"))
    {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
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
            .map(|c| c.symbol())
            .collect();
        assert!(
            text.contains("Waiting for model"),
            "{width}x{height}: {text}"
        );
        // Header row 1 names the mission and repository directory.
        assert!(text.contains("ALFREDO  mission · workspace"), "{text}");
        assert!(!text.contains("Check passed"));
    }
    control
        .command(&runtime, "/cancel-task 1", "fixture")
        .unwrap();
    assert!(control
        .worker_progress(1)
        .unwrap()
        .starts_with("Cancellation requested"));
    while !control.workers.is_empty() {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(control.worker_progress(1).is_none());
    assert_eq!(
        control.snapshot.unwrap().tasks[0].status,
        TaskStatus::Cancelled
    );
    server.join().unwrap();
}

#[tokio::test]
async fn repair_preserves_parent_requires_approval_and_uses_verified_context() {
    let fixture = Fixture::new();
    fixture.permit();
    let (provider, job, _) = server(good_plan(), Duration::ZERO);
    let (original, _) = worker::start(
        fixture.store.clone(),
        1,
        "original".into(),
        3,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    job.join().unwrap();
    fixture.action(Action::Review {
        task: 1,
        accept: false,
    });
    let parent = fixture.store.snapshot().unwrap().tasks[0].clone();
    let original_evidence = fixture.store.evidence(1).unwrap();
    let request = Request {
        correlation: "repair-proposal".into(),
        expected_revision: 6,
        action: Action::Repair {
            task: 1,
            reason: "Explain the result more clearly in notes.txt".into(),
        },
    };
    let (snapshot, receipt) = fixture.store.transact(request.clone()).unwrap();
    assert_eq!(receipt.task, 2);
    assert_eq!(snapshot.tasks[0], parent);
    let child = &snapshot.tasks[1];
    assert_eq!(child.repair_of, Some(1));
    assert_eq!(child.status, TaskStatus::Proposed);
    assert_eq!(child.policy, parent.policy);
    assert_eq!(child.model, parent.model);
    assert_eq!(
        fixture.store.transact(request).unwrap().0.revision,
        snapshot.revision
    );
    assert!(fixture
        .store
        .transact(Request {
            correlation: "duplicate-child".into(),
            expected_revision: 7,
            action: Action::Repair {
                task: 1,
                reason: "another repair".into()
            }
        })
        .is_err());
    assert!(fixture
        .store
        .transact(Request {
            correlation: "unapproved".into(),
            expected_revision: 7,
            action: Action::Start {
                inputs: vec![],
                task: 2,
                baseline: original.tasks[0].run.as_ref().unwrap().baseline.clone()
            }
        })
        .is_err());
    let (baseline, context) = fixture.store.repair_context(child).unwrap().unwrap();
    assert!(
        context.contains("CHECK_OK")
            && context.contains("return 42")
            && context.contains("Prior task #1")
    );
    assert_eq!(baseline, parent.run.as_ref().unwrap().baseline);
    fixture.action(Action::Approve { task: 2 });
    assert!(fixture
        .store
        .transact(Request {
            correlation: "wrong-baseline".into(),
            expected_revision: 8,
            action: Action::Start {
                inputs: vec![],
                task: 2,
                baseline: "b".repeat(40)
            }
        })
        .is_err());
    // A subsequent workspace commit must not retarget a repair's baseline.
    fs::write(
        fixture.workspace.join("calc.py"),
        "def answer():\n    return 99\n",
    )
    .unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&fixture.workspace)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qam",
            "later baseline"
        ])
        .status()
        .unwrap()
        .success());
    let (provider, job, _) = server(good_plan(), Duration::ZERO);
    let (finished, _) = worker::start(
        fixture.store.clone(),
        2,
        "repair-run".into(),
        8,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    job.join().unwrap();
    assert_eq!(finished.tasks[1].status, TaskStatus::ReviewReady);
    assert_eq!(finished.tasks[1].run.as_ref().unwrap().baseline, baseline);
    assert_eq!(finished.tasks[0], parent);
    assert_eq!(fixture.store.evidence(1).unwrap(), original_evidence);
    assert!(fs::read_to_string(fixture.workspace.join("calc.py"))
        .unwrap()
        .contains("return 99"));
    let artifact = fixture
        .store
        .run_directory(&parent.run.as_ref().unwrap().id)
        .unwrap()
        .join("evidence.json");
    fs::write(artifact, "tampered").unwrap();
    assert!(fixture.store.repair_context(&finished.tasks[1]).is_err());
}

#[test]
fn live_check_output_renders_before_completion_and_cancel_still_records_receipt() {
    use alfredo_tui::{model::App, task_control::TaskControl, ui};
    use ratatui::{backend::TestBackend, Terminal};
    let fixture = Fixture::new();
    fixture.action(Action::Permit { task: 1, policy: WorkPolicy {
        files: vec!["calc.py".into(), "notes.txt".into()],
        check: vec!["/usr/bin/python3".into(), "-B".into(), "-c".into(),
            "import sys,time; print('LIVE_STDOUT', flush=True); print('LIVE_STDERR', file=sys.stderr, flush=True); time.sleep(30)".into()],
    }});
    fixture.action(Action::Approve { task: 1 });
    let (provider, server, _) = server(good_plan(), Duration::ZERO);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(fixture.store.clone());
    control.refresh(&runtime);
    let refresh_deadline = Instant::now() + Duration::from_secs(5);
    while control.pending {
        control.poll();
        assert!(Instant::now() < refresh_deadline);
        thread::sleep(Duration::from_millis(2));
    }
    control.set_provider(provider);
    control.command(&runtime, "/run 1", "fixture").unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        control.poll();
        if control
            .worker_output(1)
            .is_some_and(|(out, err)| out.contains("LIVE_STDOUT") && err.contains("LIVE_STDERR"))
        {
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", control.notice);
        thread::sleep(Duration::from_millis(10));
    }
    let running = fixture.store.snapshot().unwrap();
    assert_eq!(running.tasks[0].status, TaskStatus::Running);
    assert!(running.tasks[0]
        .run
        .as_ref()
        .unwrap()
        .evidence_sha256
        .is_none());
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
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("LIVE_STDOUT") && text.contains("LIVE_STDERR"));
    assert!(!text.contains("Check passed"));
    control
        .command(&runtime, "/cancel-task 1", "fixture")
        .unwrap();
    while !control.workers.is_empty() {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(control.worker_output(1).is_none());
    let finished = fixture.store.snapshot().unwrap();
    assert_eq!(finished.tasks[0].status, TaskStatus::Cancelled);
    let evidence = fixture.store.evidence(1).unwrap();
    assert!(evidence.contains("LIVE_STDOUT") && evidence.contains("LIVE_STDERR"));
    server.join().unwrap();
}

#[tokio::test]
async fn legacy_evidence_does_not_acquire_a_candidate_commit() {
    let old = serde_json::json!({"run":"task-1-run-1","baseline":"a".repeat(40),"status":"failed","detail":"old result","patch":"","check":null});
    let evidence: worker::Evidence = serde_json::from_value(old).unwrap();
    assert!(evidence.candidate_commit.is_none());
    let error = worker::verify_candidate(std::path::Path::new("/nonexistent"), &evidence)
        .await
        .unwrap_err();
    assert!(error.contains("no candidate commit"));
}

async fn run_dependency_fixture(
    fixture: &Fixture,
    id: u64,
    plan: FilePlan,
    expected: &str,
) -> alfredo_tui::tasks::Snapshot {
    let (provider, server, _) = server_with_source(plan, Duration::ZERO, expected);
    let revision = fixture.store.snapshot().unwrap().revision;
    let (snapshot, detail) = worker::start(
        fixture.store.clone(),
        id,
        format!("run-{id}-{revision}"),
        revision,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(
        snapshot.tasks[id as usize - 1].status,
        TaskStatus::ReviewReady,
        "{detail}"
    );
    snapshot
}
fn propose_dependency(fixture: &Fixture, deps: Vec<u64>, file: &str) -> u64 {
    let id = fixture.store.snapshot().unwrap().tasks.len() as u64 + 1;
    fixture.action(Action::Propose {
        title: format!("Write {file}"),
        model: "fixture".into(),
        dependencies: deps,
    });
    fixture.action(Action::Permit {
        task: id,
        policy: WorkPolicy {
            files: vec![file.into()],
            check: vec!["/bin/true".into()],
        },
    });
    fixture.action(Action::Approve { task: id });
    id
}
fn one_file(path: &str, content: &str) -> FilePlan {
    FilePlan {
        files: vec![FileEdit {
            path: path.into(),
            content: content.into(),
        }],
    }
}

#[tokio::test]
async fn accepted_dependencies_compose_and_diamond_ancestry_is_not_applied_twice() {
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 1,
        accept: true,
    });
    let namespace = fixture.store.conversation_directory().unwrap();
    let state_path = namespace.join("tasks.json");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    legacy["schema_version"] = 3.into();
    let legacy_bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&state_path, &legacy_bytes).unwrap();
    assert_eq!(fixture.store.snapshot().unwrap().schema_version, 3);
    let second = propose_dependency(&fixture, vec![], "extra.txt");
    run_dependency_fixture(
        &fixture,
        second,
        one_file("extra.txt", "SECOND_PARENT"),
        "(new file)",
    )
    .await;
    fixture.action(Action::Review {
        task: second,
        accept: true,
    });
    let child = propose_dependency(&fixture, vec![1, second], "result.txt");
    fixture.action(Action::Permit { task: child, policy: WorkPolicy {
        files: vec!["calc.py".into(), "extra.txt".into(), "result.txt".into()],
        check: vec!["/usr/bin/python3".into(), "-B".into(), "-c".into(), "from calc import answer; from pathlib import Path; assert answer()==42; assert Path('extra.txt').read_text()=='SECOND_PARENT'; assert Path('result.txt').read_text()=='CHILD'".into()],
    }});
    fixture.action(Action::Approve { task: child });
    let finished = run_dependency_fixture(
        &fixture,
        child,
        one_file("result.txt", "CHILD"),
        "SECOND_PARENT",
    )
    .await;
    let run = finished.tasks[child as usize - 1].run.as_ref().unwrap();
    assert_eq!(
        run.inputs
            .iter()
            .map(|input| input.task)
            .collect::<Vec<_>>(),
        vec![1, second]
    );
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(child).unwrap()).unwrap();
    assert!(
        evidence.patch.contains("result.txt") && !evidence.patch.contains("diff --git a/calc.py")
    );
    worker::verify_candidate(&fixture.workspace, &evidence)
        .await
        .unwrap();
    fixture.action(Action::Review {
        task: child,
        accept: true,
    });
    let diamond = propose_dependency(&fixture, vec![1, child], "final.txt");
    let last = run_dependency_fixture(
        &fixture,
        diamond,
        one_file("final.txt", "DONE"),
        "(new file)",
    )
    .await;
    assert_eq!(
        last.tasks[diamond as usize - 1]
            .run
            .as_ref()
            .unwrap()
            .baseline,
        evidence.candidate_commit.unwrap()
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("calc.py")).unwrap(),
        "def answer():\n    return 0\n"
    );
    assert!(!fixture.workspace.join("extra.txt").exists());
    assert!(!fixture.workspace.join("result.txt").exists());
    let status = Command::new("git")
        .arg("-C")
        .arg(&fixture.workspace)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(status.stdout.is_empty());
    assert_eq!(
        fs::read(namespace.join("tasks-v3-backup.json")).unwrap(),
        legacy_bytes
    );
    let head = Command::new("git")
        .arg("-C")
        .arg(&fixture.workspace)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(head.stdout).unwrap().trim(),
        finished.tasks[0].run.as_ref().unwrap().baseline
    );
}

#[tokio::test]
async fn unaccepted_conflicting_and_tampered_dependencies_never_start_the_child() {
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 1,
        accept: true,
    });
    let second = propose_dependency(&fixture, vec![], "calc.py");
    run_dependency_fixture(
        &fixture,
        second,
        one_file("calc.py", "def answer():\n    return 43\n"),
        "return 0",
    )
    .await;
    let child = propose_dependency(&fixture, vec![1, second], "child.txt");
    let attempt = |revision| {
        worker::start(
            fixture.store.clone(),
            child,
            format!("blocked-{revision}"),
            revision,
            Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
            Arc::new(AtomicBool::new(false)),
        )
    };
    let before = fixture.store.snapshot().unwrap();
    assert!(attempt(before.revision)
        .await
        .unwrap_err()
        .contains("not accepted"));
    assert_eq!(fixture.store.snapshot().unwrap().revision, before.revision);
    fixture.action(Action::Review {
        task: second,
        accept: true,
    });
    let before = fixture.store.snapshot().unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&fixture.workspace)
        .args(["config", "merge.evil.driver", "touch SHOULD_NOT_RUN"])
        .status()
        .unwrap()
        .success());
    assert!(attempt(before.revision)
        .await
        .unwrap_err()
        .contains("Custom merge configuration"));
    assert!(!fixture.workspace.join("SHOULD_NOT_RUN").exists());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&fixture.workspace)
        .args(["config", "--remove-section", "merge.evil"])
        .status()
        .unwrap()
        .success());
    let error = attempt(before.revision).await.unwrap_err();
    assert!(
        error.contains("compose cleanly") && error.contains("calc.py"),
        "{error}"
    );
    assert_eq!(fixture.store.snapshot().unwrap().revision, before.revision);
    let run = before.tasks[0].run.as_ref().unwrap();
    fs::write(
        fixture
            .store
            .run_directory(&run.id)
            .unwrap()
            .join("evidence.json"),
        "tampered",
    )
    .unwrap();
    assert!(attempt(before.revision)
        .await
        .unwrap_err()
        .contains("digest mismatch"));
    let after = fixture.store.snapshot().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.tasks[child as usize - 1].status, TaskStatus::Approved);
    assert!(after.tasks[child as usize - 1].run.is_none());
}

#[tokio::test]
async fn accepted_branch_handoff_preserves_dirty_workspace_and_replays_without_new_receipts() {
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    let before = fixture.store.snapshot().unwrap();
    assert!(alfredo_tui::branch::publish(
        fixture.store.clone(),
        1,
        before.revision,
        "too-early".into()
    )
    .await
    .unwrap_err()
    .contains("accept"));
    fixture.action(Action::Review {
        task: 1,
        accept: true,
    });
    let before = fixture.store.snapshot().unwrap();
    let original_head = before.tasks[0].run.as_ref().unwrap().baseline.clone();
    fs::write(fixture.workspace.join("calc.py"), "user's uncommitted edit").unwrap();
    fs::write(fixture.workspace.join("private.txt"), "user's new file").unwrap();
    let (first, duplicate) = tokio::join!(
        alfredo_tui::branch::publish(fixture.store.clone(), 1, before.revision, "handoff".into()),
        alfredo_tui::branch::publish(fixture.store.clone(), 1, before.revision, "handoff".into()),
    );
    let (saved, notice) = first.unwrap();
    assert_eq!(duplicate.unwrap().0.revision, saved.revision);
    assert!(notice.contains("git switch"));
    let Action::Branch { name, commit, .. } = &saved.receipts.last().unwrap().request.action else {
        panic!("Missing branch receipt")
    };
    assert_eq!(saved.revision, before.revision + 1);
    assert_eq!(saved.tasks, before.tasks);
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&fixture.workspace)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    assert_eq!(git(&["rev-parse", "HEAD"]).trim(), original_head);
    assert_eq!(git(&["rev-parse", name]).trim(), commit);
    assert_eq!(
        git(&["show", &format!("{name}:calc.py")]),
        "def answer():\n    return 42\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("calc.py")).unwrap(),
        "user's uncommitted edit"
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("private.txt")).unwrap(),
        "user's new file"
    );
    for (revision, correlation) in [(before.revision, "handoff"), (saved.revision, "repeat")] {
        let (again, _) =
            alfredo_tui::branch::publish(fixture.store.clone(), 1, revision, correlation.into())
                .await
                .unwrap();
        assert_eq!(again.revision, saved.revision);
    }
}

#[tokio::test]
async fn branch_conflicts_are_preserved_and_existing_exact_ref_reconciles_v4_state() {
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 1,
        accept: true,
    });
    let before = fixture.store.snapshot().unwrap();
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    let commit = evidence.candidate_commit.unwrap();
    let name = alfredo_tui::branch::name(1, &commit).unwrap();
    let reference = format!("refs/heads/{name}");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&fixture.workspace)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["update-ref", &reference, &evidence.baseline]);
    assert!(alfredo_tui::branch::publish(
        fixture.store.clone(),
        1,
        before.revision,
        "collision".into()
    )
    .await
    .unwrap_err()
    .contains("points elsewhere"));
    assert_eq!(git(&["rev-parse", &reference]).trim(), evidence.baseline);
    git(&["update-ref", "-d", &reference]);
    git(&["symbolic-ref", &reference, "refs/heads/missing-target"]);
    assert!(alfredo_tui::branch::publish(
        fixture.store.clone(),
        1,
        before.revision,
        "symbolic".into()
    )
    .await
    .unwrap_err()
    .contains("symbolic ref"));
    git(&["symbolic-ref", "--delete", &reference]);
    // Simulate a successful Git operation whose task receipt was not saved.
    git(&["update-ref", &reference, &commit]);
    let namespace = fixture.store.conversation_directory().unwrap();
    let state_path = namespace.join("tasks.json");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    legacy["schema_version"] = 4.into();
    let bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&state_path, &bytes).unwrap();
    let (saved, _) = alfredo_tui::branch::publish(
        fixture.store.clone(),
        1,
        before.revision,
        "reconcile".into(),
    )
    .await
    .unwrap();
    assert_eq!(saved.schema_version, alfredo_tui::tasks::SCHEMA_VERSION);
    assert_eq!(saved.revision, before.revision + 1);
    assert_eq!(
        fs::read(namespace.join("tasks-v4-backup.json")).unwrap(),
        bytes
    );
    assert_eq!(git(&["rev-parse", &reference]).trim(), commit);
    assert_eq!(git(&["rev-parse", "HEAD"]).trim(), evidence.baseline);
}

#[tokio::test]
async fn branch_refuses_invalid_identity_before_creating_a_ref() {
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 1,
        accept: true,
    });
    let snapshot = fixture.store.snapshot().unwrap();
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    let name = alfredo_tui::branch::name(1, evidence.candidate_commit.as_ref().unwrap()).unwrap();
    let error =
        alfredo_tui::branch::publish(fixture.store.clone(), 1, snapshot.revision, "\n".into())
            .await
            .unwrap_err();
    assert!(error.contains("correlation"), "{error}");
    assert!(!Command::new("git")
        .arg("-C")
        .arg(&fixture.workspace)
        .args(["show-ref", "--verify", &format!("refs/heads/{name}")])
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(
        fixture.store.snapshot().unwrap().revision,
        snapshot.revision
    );
}

#[tokio::test]
async fn branch_checks_journal_capacity_before_effects_and_replays_at_capacity() {
    use alfredo_tui::tasks::Receipt;
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 1,
        accept: true,
    });
    fixture.action(Action::Propose {
        title: "filler".into(),
        model: "fixture".into(),
        dependencies: vec![],
    });
    let mut snapshot = fixture.store.snapshot().unwrap();
    let policy = WorkPolicy {
        files: vec!["file.txt".into()],
        check: vec!["true".into()],
    };
    snapshot.tasks[1].policy = Some(policy.clone());
    while snapshot.receipts.len() < 4096 {
        let request = Request {
            correlation: format!("fill-{}", snapshot.revision),
            expected_revision: snapshot.revision,
            action: Action::Permit {
                task: 2,
                policy: policy.clone(),
            },
        };
        snapshot.revision += 1;
        snapshot.receipts.push(Receipt {
            request,
            revision: snapshot.revision,
            task: 2,
        });
    }
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert_eq!(
        fixture.store.snapshot().unwrap().revision,
        snapshot.revision
    );
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    let name = alfredo_tui::branch::name(1, evidence.candidate_commit.as_ref().unwrap()).unwrap();
    let reference = format!("refs/heads/{name}");
    let error =
        alfredo_tui::branch::publish(fixture.store.clone(), 1, snapshot.revision, "full".into())
            .await
            .unwrap_err();
    assert!(error.contains("capacity"), "{error}");
    assert!(!Command::new("git")
        .arg("-C")
        .arg(&fixture.workspace)
        .args(["show-ref", "--verify", &reference])
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    // Restore the exact previous valid revision, then use its final slot.
    snapshot.receipts.pop();
    snapshot.revision -= 1;
    fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let (saved, _) = alfredo_tui::branch::publish(
        fixture.store.clone(),
        1,
        snapshot.revision,
        "last-slot".into(),
    )
    .await
    .unwrap();
    assert_eq!(saved.receipts.len(), 4096);
    let (replayed, _) = alfredo_tui::branch::publish(
        fixture.store.clone(),
        1,
        saved.revision,
        "repeat-full".into(),
    )
    .await
    .unwrap();
    assert_eq!(replayed.revision, saved.revision);
}

#[tokio::test]
async fn branch_checks_serialized_byte_capacity_before_creating_a_ref() {
    use alfredo_tui::tasks::{Receipt, Snapshot};
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 1,
        accept: true,
    });
    fixture.action(Action::Propose {
        title: "filler".into(),
        model: "fixture".into(),
        dependencies: vec![],
    });
    let mut snapshot = fixture.store.snapshot().unwrap();
    let append = |snapshot: &mut Snapshot, quotes: usize| {
        let policy = WorkPolicy {
            files: vec!["file.txt".into()],
            check: (0..quotes)
                .step_by(2048)
                .map(|offset| "\"".repeat((quotes - offset).min(2048)))
                .collect(),
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
    };
    const TARGET: usize = 4 * 1024 * 1024 - 64;
    loop {
        let mut trial = snapshot.clone();
        append(&mut trial, 65536);
        if serde_json::to_vec(&trial).unwrap().len() > TARGET {
            break;
        }
        snapshot = trial;
    }
    let (mut low, mut high) = (1usize, 65536usize);
    while low < high {
        let mid = (low + high).div_ceil(2);
        let mut trial = snapshot.clone();
        append(&mut trial, mid);
        if serde_json::to_vec(&trial).unwrap().len() <= TARGET {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    append(&mut snapshot, low);
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    assert!((TARGET - 5..=TARGET).contains(&bytes.len()));
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    fs::write(&path, &bytes).unwrap();
    assert_eq!(
        fixture.store.snapshot().unwrap().revision,
        snapshot.revision
    );
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    let name = alfredo_tui::branch::name(1, evidence.candidate_commit.as_ref().unwrap()).unwrap();
    let error = alfredo_tui::branch::publish(
        fixture.store.clone(),
        1,
        snapshot.revision,
        "byte-full".into(),
    )
    .await
    .unwrap_err();
    assert!(error.contains("capacity"), "{error}");
    assert!(!Command::new("git")
        .arg("-C")
        .arg(&fixture.workspace)
        .args(["show-ref", "--verify", &format!("refs/heads/{name}")])
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[tokio::test]
async fn planned_worker_receives_the_saved_scope_brief_with_its_existing_policy() {
    use alfredo_tui::{
        planner::{Plan, Step},
        understanding::{Action as ScopeAction, Brief, Request as ScopeRequest},
    };
    let fixture = Fixture::new();
    fixture.permit();
    let scope = fixture.store.understanding();
    scope
        .transact(ScopeRequest {
            correlation: "brief".into(),
            expected_revision: 0,
            action: ScopeAction::Draft {
                brief: Brief {
                    destination: "Reliable arithmetic".into(),
                    scope: "Calculation and notes".into(),
                    constraints: "Keep the SCOPE_CONSTRAINT_SENTINEL API unchanged".into(),
                    uncertainty: "Runtime performance is unmeasured".into(),
                },
            },
        })
        .unwrap();
    let binding = scope
        .transact(ScopeRequest {
            correlation: "confirm".into(),
            expected_revision: 1,
            action: ScopeAction::Confirm { draft_revision: 1 },
        })
        .unwrap()
        .binding();
    let policy = fixture.store.snapshot().unwrap().tasks[0]
        .policy
        .clone()
        .unwrap();
    fixture.action(Action::Plan {
        plan: Plan {
            architecture: None,
            prompt: "Implement calculation".into(),
            planner: "fixture".into(),
            context: None,
            scope: Some(Box::new(binding.clone())),
            tasks: vec![Step {
                acceptance: vec![
                    "Acceptance contract sentinel: preserve expected calculation behavior".into(),
                ],
                title: "Make answer return 42 and add notes".into(),
                model: "fixture".into(),
                dependencies: vec![],
                policy: policy.clone(),
            }],
        },
    });
    fixture.action(Action::Approve { task: 2 });
    let expected = format!(
        "{}\0Acceptance contract sentinel: preserve expected calculation behavior",
        serde_json::to_string(&binding).unwrap()
    );
    let (provider, server, _) = server_with_source(good_plan(), Duration::ZERO, &expected);
    let (snapshot, detail) = worker::start(
        fixture.store.clone(),
        2,
        "scope-worker".into(),
        5,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(
        snapshot.tasks[1].status,
        TaskStatus::ReviewReady,
        "{detail}"
    );
    assert_eq!(snapshot.tasks[1].policy.as_ref(), Some(&policy));
    assert_eq!(
        snapshot.plan_for_task(2).unwrap().scope.as_deref(),
        Some(&binding)
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("calc.py")).unwrap(),
        "def answer():\n    return 0\n"
    );
    use alfredo_tui::assessment::{Assessment, Criterion};
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let mut legacy = serde_json::to_value(&snapshot).unwrap();
    legacy["schema_version"] = 10.into();
    let legacy_bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    fs::write(&path, &legacy_bytes).unwrap();
    let revision = snapshot.revision;
    let review = Assessment {
        accept: false,
        reason: "REVIEW_REASON_SENTINEL: inspect compatibility".into(),
        criteria: vec![Criterion {
            criterion: 1,
            met: false,
            note: "Compatibility behavior remains unverified".into(),
        }],
    };
    let request = Request {
        correlation: "contract-review".into(),
        expected_revision: revision,
        action: Action::Assess {
            task: 2,
            assessment: review.clone(),
        },
    };
    assert!(fixture
        .store
        .transact(Request {
            correlation: "bare-accept".into(),
            expected_revision: revision,
            action: Action::Review {
                task: 2,
                accept: true
            }
        })
        .unwrap_err()
        .contains("acceptance criteria"));
    let mut missing = request.clone();
    missing.correlation = "missing".into();
    if let Action::Assess { assessment, .. } = &mut missing.action {
        assessment.criteria.clear();
    }
    assert!(fixture.store.transact(missing).is_err());
    let mut stale = request.clone();
    stale.expected_revision = 0;
    assert!(fixture.store.transact(stale).is_err());
    assert_eq!(fs::read(&path).unwrap(), legacy_bytes);
    let evidence_path = fixture
        .store
        .run_directory(&snapshot.tasks[1].run.as_ref().unwrap().id)
        .unwrap()
        .join("evidence.json");
    let original_evidence = fs::read(&evidence_path).unwrap();
    let mut changed = original_evidence.clone();
    changed.push(b' ');
    fs::write(&evidence_path, changed).unwrap();
    assert!(fixture.store.transact(request.clone()).is_err());
    assert_eq!(fs::read(&path).unwrap(), legacy_bytes);
    fs::write(&evidence_path, original_evidence).unwrap();
    let (reviewed, receipt) = fixture.store.transact(request.clone()).unwrap();
    assert_eq!(reviewed.tasks[1].status, TaskStatus::Rejected);
    assert_eq!(reviewed.assessment_for_task(2), Some(&review));
    assert_eq!(
        fs::read(path.parent().unwrap().join("tasks-v10-backup.json")).unwrap(),
        legacy_bytes
    );
    assert_eq!(fixture.store.transact(request.clone()).unwrap().1, receipt);
    let mut conflict = request;
    if let Action::Assess { assessment, .. } = &mut conflict.action {
        assessment.reason = "Changed reason".into();
    }
    assert!(fixture.store.transact(conflict).is_err());
    assert!(
        alfredo_tui::activity::entries(&reviewed, "REVIEW_REASON_SENTINEL")
            .iter()
            .any(|entry| entry.task == 2)
    );

    fixture.action(Action::Repair {
        task: 2,
        reason: "Complete the acceptance contract".into(),
    });
    let repaired = fixture.store.snapshot().unwrap();
    assert_eq!(repaired.tasks[2].status, TaskStatus::Proposed);
    assert_eq!(
        repaired.acceptance_for_task(3),
        repaired.acceptance_for_task(2)
    );
    let review = alfredo_tui::review::View::from_verified(2, &fixture.store.evidence(2).unwrap())
        .unwrap()
        .with_acceptance(repaired.acceptance_for_task(2))
        .with_assessment(repaired.assessment_for_task(2));
    assert!(review
        .lines()
        .iter()
        .any(|line| line.to_string().contains("Acceptance contract sentinel")));
    assert!(review
        .lines()
        .iter()
        .any(|line| line.to_string().contains("REVIEW_REASON_SENTINEL")));
    assert!(fixture
        .store
        .repair_context(&repaired.tasks[2])
        .unwrap()
        .unwrap()
        .1
        .contains("REVIEW_REASON_SENTINEL"));
    fixture.action(Action::Approve { task: 3 });
    let revision = fixture.store.snapshot().unwrap().revision;
    let (provider, server, _) = server_with_source(good_plan(), Duration::ZERO,
        "RECORDED ACCEPTANCE CRITERIA\0Acceptance contract sentinel: preserve expected calculation behavior\0REVIEW_REASON_SENTINEL");
    let (done, detail) = worker::start(
        fixture.store.clone(),
        3,
        "repair-contract".into(),
        revision,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(done.tasks[2].status, TaskStatus::ReviewReady, "{detail}");
    let assessment = Assessment {
        accept: true,
        reason: "Reviewed repair and supporting tests".into(),
        criteria: vec![Criterion {
            criterion: 1,
            met: true,
            note: "calc answer and notes verified in retained evidence".into(),
        }],
    };
    fixture.action(Action::Assess {
        task: 3,
        assessment: assessment.clone(),
    });
    let accepted = fixture.store.snapshot().unwrap();
    assert_eq!(accepted.tasks[2].status, TaskStatus::Accepted);
    assert_eq!(accepted.assessment_for_task(3), Some(&assessment));
    let committed = fs::read(&path).unwrap();
    let mut old_version = serde_json::to_value(&accepted).unwrap();
    old_version["schema_version"] = 10.into();
    let forged = serde_json::to_vec(&old_version).unwrap();
    fs::write(&path, &forged).unwrap();
    assert!(fixture.store.snapshot().unwrap_err().contains("schema v11"));
    assert_eq!(fs::read(&path).unwrap(), forged);
    // Historical v10 boolean reviews remain valid, with no invented assessments.
    for receipt in old_version["receipts"].as_array_mut().unwrap() {
        let action = &mut receipt["request"]["action"];
        if action["kind"] == "assess" {
            *action = serde_json::to_value(Action::Review {
                task: action["task"].as_u64().unwrap(),
                accept: action["assessment"]["accept"].as_bool().unwrap(),
            })
            .unwrap();
        }
    }
    fs::write(&path, serde_json::to_vec(&old_version).unwrap()).unwrap();
    let historical = fixture.store.snapshot().unwrap();
    assert!(historical.assessment_for_task(3).is_none());
    assert_eq!(historical.tasks[2].status, TaskStatus::Accepted);
    fs::write(path, committed).unwrap();
}

#[tokio::test]
#[ignore = "Requires installed local model and Bubblewrap; runs two real coding tasks"]
async fn live_parallel_workers_pass_independent_edge_case_checks() {
    let model = std::env::var("ALFREDO_SMOKE_MODEL").unwrap_or_else(|_| "qwen3:14b".into());
    let capacity = std::env::var("ALFREDO_SMOKE_PARALLEL_MODELS")
        .map(|s| s.parse::<usize>().expect("integer model capacity"))
        .unwrap_or(2);
    let endpoint = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let provider = Ollama::new(&endpoint, Duration::from_secs(60))
        .unwrap()
        .with_parallelism(capacity)
        .unwrap();
    println!("LIVE_WORKER_CAPACITY {capacity}");
    async fn sample(model: String, provider: Ollama, case: &str, title: &str, check: &str) -> bool {
        let fixture = Fixture::with_model(&model);
        fixture.action(Action::Propose {
            title: title.into(),
            model: model.clone(),
            dependencies: vec![],
        });
        fixture.action(Action::Permit {
            task: 2,
            policy: WorkPolicy {
                files: vec!["calc.py".into()],
                check: vec![
                    "/usr/bin/python3".into(),
                    "-B".into(),
                    "-c".into(),
                    format!("exec({})", serde_json::to_string(check).unwrap()),
                ],
            },
        });
        fixture.action(Action::Approve { task: 2 });
        let revision = fixture.store.snapshot().unwrap().revision;
        let (observer, progress) = worker::Observer::channel();
        let started = Instant::now();
        let (snapshot, detail) = worker::start_observed(
            fixture.store.clone(),
            2,
            format!("live-{case}"),
            revision,
            provider,
            Arc::new(AtomicBool::new(false)),
            observer,
        )
        .await
        .unwrap();
        let evidence: worker::Evidence =
            serde_json::from_str(&fixture.store.evidence(2).unwrap()).unwrap();
        println!(
            "LIVE_WORKER_SAMPLE {}",
            serde_json::json!({
                "model": model, "case": case, "elapsed_seconds": started.elapsed().as_secs_f64(),
                "first_content_seconds": progress.borrow().first_content.map(|d| d.as_secs_f64()),
                "status": snapshot.tasks[1].status, "detail": detail,
                "model_metrics": evidence.model_metrics, "candidate_commit": evidence.candidate_commit,
            })
        );
        assert_eq!(
            fs::read_to_string(fixture.workspace.join("calc.py")).unwrap(),
            "def answer():\n    return 0\n"
        );
        assert_eq!(
            fixture.store.snapshot().unwrap().tasks[0].status,
            TaskStatus::Proposed
        );
        snapshot.tasks[1].status == TaskStatus::ReviewReady && evidence.candidate_commit.is_some()
    }
    let ports = sample(model.clone(), provider.clone(), "strict_port",
        "Replace calc.py with parse_port(value). Accept only Python str containing one or more ASCII digits and whose numeric value is 1 through 65535 inclusive; leading zeros allowed. Return int. All other values, including bool/int/None, whitespace, signs, non-ASCII digits and out-of-range strings must raise ValueError. Use no third-party libraries.",
        r#"from calc import parse_port
for n in [1,2,80,443,1024,32768,65534,65535]:
 assert parse_port(str(n)) == n
 assert parse_port('000'+str(n)) == n
for value in ['', '0', '000', '65536', '99999999999999999999', '-1', '+80', ' 80', '80 ', '8.0', '１２', '١٢', '1_2', '1\n', 80, True, None, [], {}]:
 try: parse_port(value)
 except ValueError: pass
 else: raise AssertionError(repr(value))
print('PORT_EDGE_CASES_OK')"#);
    let intervals = sample(model, provider, "merge_intervals",
        "Replace calc.py with merge_intervals(intervals): input is a list of (start,end) integer pairs with start <= end. Return a sorted list of tuples merging overlapping OR touching intervals. Empty input returns []. Do not mutate the input list or its contained pairs. Handle duplicates, negative endpoints, nesting and zero-length intervals. Use no third-party libraries.",
        r#"from calc import merge_intervals
import copy, itertools
assert merge_intervals([]) == []
assert merge_intervals([(1,2),(2,3)]) == [(1,3)]
assert merge_intervals([(5,7),(-4,-2),(-3,1),(6,6)]) == [(-4,1),(5,7)]
pairs=[(a,b) for a in range(-2,3) for b in range(a,3)]
for chosen in itertools.product(pairs, repeat=3):
 original=[list(p) for p in chosen]; before=copy.deepcopy(original)
 expected=[]
 for a,b in sorted(chosen):
  if expected and a <= expected[-1][1]: expected[-1]=(expected[-1][0],max(b,expected[-1][1]))
  else: expected.append((a,b))
 assert merge_intervals(original) == expected, chosen
 assert original == before, 'mutated input'
print('INTERVAL_EDGE_CASES_OK')"#);
    let (ports_ok, intervals_ok) = match std::env::var("ALFREDO_SMOKE_CASE").as_deref() {
        Ok("strict_port") => (ports.await, true),
        Ok("merge_intervals") => (true, intervals.await),
        Err(std::env::VarError::NotPresent) => tokio::join!(ports, intervals),
        _ => panic!("ALFREDO_SMOKE_CASE must be strict_port or merge_intervals"),
    };
    assert!(
        ports_ok && intervals_ok,
        "Both independent live coding cases must pass; see sample outcomes"
    );
}

#[tokio::test]
async fn fresh_created_workspace_supports_first_isolated_worker_without_source_files() {
    let mut fixture = Fixture::new();
    let target = fixture.root.join("fresh");
    let state = fixture.root.join("fresh-state");
    fixture.workspace = alfredo_tui::selection::acknowledge(&target, &state, true)
        .await
        .unwrap();
    let context = alfredo_tui::planning_context::capture(&target, "Build calculation")
        .await
        .unwrap();
    assert!(context.sources.is_empty());
    fixture.store = TaskStore::new(&state, &target, "fresh").unwrap();
    fixture.action(Action::Propose {
        title: "Create calculation and notes".into(),
        model: "fixture".into(),
        dependencies: vec![],
    });
    fixture.permit();
    let (provider, server, _) =
        server_with_source(good_plan(), Duration::ZERO, "FILE calc.py\n(new file)");
    let (observer, _) = worker::Observer::channel();
    let (snapshot, detail) = worker::start_observed(
        fixture.store.clone(),
        1,
        "first-run".into(),
        3,
        provider,
        Arc::new(AtomicBool::new(false)),
        observer,
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(
        snapshot.tasks[0].status,
        TaskStatus::ReviewReady,
        "{detail}"
    );
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    assert_eq!(evidence.baseline, context.baseline);
    assert!(evidence.candidate_commit.is_some());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(
        alfredo_tui::planning_context::capture(&target, "Inspect")
            .await
            .unwrap(),
        context
    );
}

#[tokio::test]
#[ignore = "subprocess crash fixture"]
async fn interrupted_worker_process_fixture() {
    let root = PathBuf::from(std::env::var_os("ALFREDO_CRASH_ROOT").unwrap());
    let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
    let provider = Ollama::new(
        &std::env::var("ALFREDO_CRASH_ENDPOINT").unwrap(),
        Duration::from_secs(30),
    )
    .unwrap();
    worker::start(
        store,
        1,
        "crash-before-check".into(),
        3,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
}

#[test]
fn killed_real_worker_before_model_reply_recovers_without_launching_check() {
    let fixture = Fixture::new();
    fixture.permit();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (seen, received) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("crash fixture did not connect: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut header = vec![];
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
            assert!(header.len() < 8192);
        }
        seen.send(()).unwrap();
        let _ = wait.recv_timeout(Duration::from_secs(10));
    });
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "interrupted_worker_process_fixture"])
        .env("ALFREDO_CRASH_ROOT", &fixture.root)
        .env("ALFREDO_CRASH_ENDPOINT", endpoint)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let requested = received.recv_timeout(Duration::from_secs(10)).is_ok();
    let active_refused = requested
        && fixture
            .store
            .recover(1)
            .is_err_and(|error| error.contains("still active"));
    let _ = child.kill();
    child.wait().unwrap();
    let _ = release.send(());
    server.join().unwrap();
    assert!(requested && active_refused);
    let running = fixture.store.snapshot().unwrap();
    let directory = fixture
        .store
        .run_directory(&running.tasks[0].run.as_ref().unwrap().id)
        .unwrap();
    assert!(directory.join("execution-boundary.json").exists());
    assert!(!directory.join("check-launch-intent.json").exists());
    let (recovered, _) = fixture.store.recover(1).unwrap();
    assert_eq!(recovered.tasks[0].status, TaskStatus::Failed);
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    assert!(evidence.check.is_none());
    assert!(directory.join("worktree").is_dir());
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("calc.py")).unwrap(),
        "def answer():\n    return 0\n"
    );
    assert!(!directory.join("check-launch-intent.json").exists());
}

#[tokio::test]
async fn review_outcomes_hold_resolve_and_preserve_exact_v11_history() {
    use alfredo_tui::{
        assessment::{Criterion, Decision, Outcome},
        planner::{Plan, Step},
    };
    for outcome in [
        Outcome::Approved,
        Outcome::ApprovedWithLimitations,
        Outcome::NeedsRepair,
        Outcome::NeedsHumanReview,
        Outcome::Rejected,
    ] {
        let fixture = Fixture::new();
        fixture.permit();
        let policy = fixture.store.snapshot().unwrap().tasks[0]
            .policy
            .clone()
            .unwrap();
        fixture.action(Action::Cancel { task: 1 });
        fixture.action(Action::Plan {
            plan: Plan {
                architecture: None,
                prompt: "Implement answer".into(),
                planner: "fixture".into(),
                context: None,
                scope: None,
                tasks: vec![Step {
                    title: "Make answer return 42 and add notes".into(),
                    acceptance: vec!["Answer is 42".into()],
                    model: "fixture".into(),
                    dependencies: vec![],
                    policy: policy.clone(),
                }],
            },
        });
        fixture.action(Action::Approve { task: 2 });
        let (provider, server, _) = server(good_plan(), Duration::ZERO);
        worker::start(
            fixture.store.clone(),
            2,
            "outcome-run".into(),
            fixture.store.snapshot().unwrap().revision,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        server.join().unwrap();
        fixture.action(Action::Propose {
            title: "Dependent child".into(),
            model: "fixture".into(),
            dependencies: vec![2],
        });
        fixture.action(Action::Permit { task: 3, policy });
        fixture.action(Action::Approve { task: 3 });
        let ready = fixture.store.snapshot().unwrap();
        let path = fixture
            .store
            .conversation_directory()
            .unwrap()
            .join("tasks.json");
        let mut legacy = serde_json::to_value(&ready).unwrap();
        legacy["schema_version"] = 11.into();
        let legacy_bytes = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&path, &legacy_bytes).unwrap();
        let decision = Decision {
            failure: None,
            risk: None,
            outcome,
            reason: "Reviewed fixture outcome".into(),
            criteria: vec![Criterion {
                criterion: 1,
                met: outcome.approves(),
                note: "Inspected retained calculation evidence".into(),
            }],
            limitations: if outcome == Outcome::ApprovedWithLimitations {
                vec!["Performance beyond fixture inputs is unmeasured".into()]
            } else {
                vec![]
            },
        };
        let request = Request {
            correlation: "decision".into(),
            expected_revision: ready.revision,
            action: Action::Decide {
                task: 2,
                decision: decision.clone(),
            },
        };
        let evidence_path = fixture
            .store
            .run_directory(&ready.tasks[1].run.as_ref().unwrap().id)
            .unwrap()
            .join("evidence.json");
        let original = fs::read(&evidence_path).unwrap();
        let mut tampered = original.clone();
        tampered.push(b' ');
        fs::write(&evidence_path, tampered).unwrap();
        assert!(fixture.store.transact(request.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), legacy_bytes);
        fs::write(&evidence_path, original).unwrap();
        let (reviewed, receipt) = fixture.store.transact(request.clone()).unwrap();
        assert_eq!(reviewed.decision_for_task(2), Some(&decision));
        assert_eq!(fixture.store.transact(request.clone()).unwrap().1, receipt);
        assert_eq!(
            fs::read(path.parent().unwrap().join("tasks-v11-backup.json")).unwrap(),
            legacy_bytes
        );
        let mut conflict = request;
        if let Action::Decide { decision, .. } = &mut conflict.action {
            decision.reason = "Different reason".into();
        }
        assert!(fixture.store.transact(conflict).is_err());
        let dispatch = alfredo_tui::dispatch::Dispatch {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(
            dispatch.next(&reviewed, &std::collections::BTreeSet::new()),
            if outcome.approves() { Some(3) } else { None }
        );
        assert_eq!(
            reviewed.tasks[1].status,
            if outcome.approves() {
                TaskStatus::Accepted
            } else if outcome == Outcome::NeedsHumanReview {
                TaskStatus::NeedsHumanReview
            } else {
                TaskStatus::Rejected
            }
        );
        assert!(
            alfredo_tui::activity::entries(&reviewed, "Reviewed fixture outcome")
                .iter()
                .any(|entry| entry.task == 2)
        );
        let mut view =
            alfredo_tui::review::View::from_verified(2, &fixture.store.evidence(2).unwrap())
                .unwrap()
                .with_acceptance(reviewed.acceptance_for_task(2))
                .with_review(reviewed.review_summary_for_task(2).as_deref());
        if outcome == Outcome::NeedsHumanReview {
            let unchanged = fs::read(&path).unwrap();
            for action in [
                Action::Approve { task: 2 },
                Action::Review {
                    task: 2,
                    accept: true,
                },
                Action::Repair {
                    task: 2,
                    reason: "Must not bypass human hold".into(),
                },
                Action::Start {
                    task: 2,
                    baseline: ready.tasks[1].run.as_ref().unwrap().baseline.clone(),
                    inputs: vec![],
                },
            ] {
                assert!(fixture
                    .store
                    .transact(Request {
                        correlation: "bypass".into(),
                        expected_revision: reviewed.revision,
                        action
                    })
                    .is_err());
                assert_eq!(fs::read(&path).unwrap(), unchanged);
            }
            assert!(fixture.store.claim_worker(2).is_err());
            let resolved = Decision {
                failure: None,
                risk: None,
                outcome: Outcome::Approved,
                reason: "Human resolved the concern".into(),
                criteria: vec![Criterion {
                    criterion: 1,
                    met: true,
                    note: "Checked expected behavior and retained test".into(),
                }],
                limitations: vec![],
            };
            fixture.action(Action::Decide {
                task: 2,
                decision: resolved,
            });
            let resolved = fixture.store.snapshot().unwrap();
            assert_eq!(resolved.tasks[1].status, TaskStatus::Accepted);
            assert_eq!(
                dispatch.next(&resolved, &std::collections::BTreeSet::new()),
                Some(3)
            );
            view = view.with_review(resolved.review_summary_for_task(2).as_deref());
            let text = view
                .lines()
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(!text.contains("Needs human review"));
            assert!(text.contains("Human resolved the concern"));
            assert_eq!(text.matches("Recorded reviewer assessment").count(), 1);
        }
        let current = fixture.store.snapshot().unwrap();
        let committed = fs::read(&path).unwrap();
        let mut downgraded = serde_json::to_value(current).unwrap();
        downgraded["schema_version"] = 11.into();
        let bad = serde_json::to_vec(&downgraded).unwrap();
        fs::write(&path, &bad).unwrap();
        assert!(fixture.store.snapshot().unwrap_err().contains("schema v12"));
        assert_eq!(fs::read(&path).unwrap(), bad);
        fs::write(path, committed).unwrap();
    }
}

#[tokio::test]
async fn failed_work_can_be_held_and_sent_to_repair_but_not_approved() {
    use alfredo_tui::assessment::{Decision, Outcome};
    let fixture = Fixture::new();
    fixture.permit();
    let (provider, server, _) = server(
        FilePlan {
            files: vec![FileEdit {
                path: "unapproved.txt".into(),
                content: "refuse".into(),
            }],
        },
        Duration::ZERO,
    );
    let (failed, _) = worker::start(
        fixture.store.clone(),
        1,
        "failed-review".into(),
        3,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(failed.tasks[0].status, TaskStatus::Failed);
    let decision = |outcome| Decision {
        failure: None,
        risk: None,
        outcome,
        reason: "Inspect failed result".into(),
        criteria: vec![],
        limitations: if outcome == Outcome::ApprovedWithLimitations {
            vec!["No successful check".into()]
        } else {
            vec![]
        },
    };
    fixture.action(Action::Decide {
        task: 1,
        decision: decision(Outcome::NeedsHumanReview),
    });
    for outcome in [Outcome::Approved, Outcome::ApprovedWithLimitations] {
        let snapshot = fixture.store.snapshot().unwrap();
        assert!(fixture
            .store
            .transact(Request {
                correlation: "invalid-approval".into(),
                expected_revision: snapshot.revision,
                action: Action::Decide {
                    task: 1,
                    decision: decision(outcome)
                }
            })
            .unwrap_err()
            .contains("successful check"));
        assert_eq!(
            fixture.store.snapshot().unwrap().tasks[0].status,
            TaskStatus::NeedsHumanReview
        );
    }
    fixture.action(Action::Decide {
        task: 1,
        decision: decision(Outcome::NeedsRepair),
    });
    fixture.action(Action::Repair {
        task: 1,
        reason: "Stay within approved paths".into(),
    });
    let repaired = fixture.store.snapshot().unwrap();
    assert_eq!(repaired.tasks[1].status, TaskStatus::Proposed);
    assert!(repaired.tasks[1].run.is_none());
    assert!(fixture
        .store
        .repair_context(&repaired.tasks[1])
        .unwrap()
        .unwrap()
        .1
        .contains("Needs repair"));
}

#[tokio::test]
async fn held_repair_blocks_sibling_proposals_without_rewriting_historical_receipts() {
    use alfredo_tui::assessment::{Decision, Outcome};
    let fixture = Fixture::new();
    fixture.permit();
    for task in [1, 2] {
        if task == 2 {
            fixture.action(Action::Review {
                task: 1,
                accept: false,
            });
            fixture.action(Action::Repair {
                task: 1,
                reason: "Improve the explanation".into(),
            });
            fixture.action(Action::Approve { task: 2 });
        }
        let (provider, job, _) = server(good_plan(), Duration::ZERO);
        worker::start(
            fixture.store.clone(),
            task,
            format!("run-{task}"),
            fixture.store.snapshot().unwrap().revision,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        job.join().unwrap();
    }
    let decision = |outcome| Decision {
        failure: None,
        risk: None,
        outcome,
        reason: "Inspect retained repair evidence".into(),
        criteria: vec![],
        limitations: vec![],
    };
    fixture.action(Action::Decide {
        task: 2,
        decision: decision(Outcome::NeedsHumanReview),
    });
    let held = fixture.store.snapshot().unwrap();
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let before = fs::read(&path).unwrap();
    let request = Request {
        correlation: "sibling-during-hold".into(),
        expected_revision: held.revision,
        action: Action::Repair {
            task: 1,
            reason: "Try a second repair".into(),
        },
    };
    assert!(fixture
        .store
        .transact(request.clone())
        .is_err_and(|e| e.contains("unresolved repair")));
    assert_eq!(fs::read(&path).unwrap(), before);
    let reopened =
        TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission").unwrap();
    assert!(reopened
        .transact(request.clone())
        .is_err_and(|e| e.contains("unresolved repair")));

    // v12 previously allowed this proposal. Retain its exact history and retry,
    // while rejecting any new sibling request against that historical state.
    let mut legacy = held.clone();
    legacy.schema_version = 12;
    let mut sibling = held.tasks[1].clone();
    sibling.id = 3;
    sibling.title = "Repair #1: Try a second repair".into();
    sibling.status = TaskStatus::Proposed;
    sibling.run = None;
    legacy.tasks.push(sibling);
    legacy.revision += 1;
    legacy.receipts.push(alfredo_tui::tasks::Receipt {
        request: request.clone(),
        revision: legacy.revision,
        task: 3,
    });
    fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
    let historical = fs::read(&path).unwrap();
    assert_eq!(reopened.transact(request).unwrap().1.task, 3);
    assert_eq!(fs::read(&path).unwrap(), historical);
    assert!(reopened
        .transact(Request {
            correlation: "another-sibling".into(),
            expected_revision: legacy.revision,
            action: Action::Repair {
                task: 1,
                reason: "Another".into()
            }
        })
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), historical);

    // Resolving the hold permits a new, unapproved repair proposal as before.
    fs::write(&path, before).unwrap();
    fixture.action(Action::Decide {
        task: 2,
        decision: decision(Outcome::NeedsRepair),
    });
    fixture.action(Action::Repair {
        task: 2,
        reason: "Address the review".into(),
    });
    let resolved = fixture.store.snapshot().unwrap();
    assert_eq!(resolved.tasks[2].repair_of, Some(2));
    assert_eq!(resolved.tasks[2].status, TaskStatus::Proposed);
    assert!(resolved.tasks[2].run.is_none());
}

#[tokio::test]
async fn risk_escalation_is_durable_blocks_bypasses_and_preserves_success_requirements() {
    use alfredo_tui::assessment::{Decision, Outcome, ReviewRisk};
    for risk in [
        ReviewRisk::Critical,
        ReviewRisk::Security,
        ReviewRisk::MergeRisk,
    ] {
        for failed in [false, true] {
            let fixture = Fixture::new();
            fixture.permit();
            let mut plan = good_plan();
            if failed {
                plan.files[0].content = "def answer():\n    return 0\n".into();
            }
            let (provider, job, _) = server(plan, Duration::ZERO);
            worker::start(
                fixture.store.clone(),
                1,
                "risk-run".into(),
                fixture.store.snapshot().unwrap().revision,
                provider,
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap();
            job.join().unwrap();
            let finished = fixture.store.snapshot().unwrap();
            assert_eq!(
                finished.tasks[0].status,
                if failed {
                    TaskStatus::Failed
                } else {
                    TaskStatus::ReviewReady
                }
            );
            fixture.action(Action::Propose {
                title: "Dependent".into(),
                model: "fixture".into(),
                dependencies: vec![1],
            });
            fixture.action(Action::Permit {
                task: 2,
                policy: finished.tasks[0].policy.clone().unwrap(),
            });
            fixture.action(Action::Approve { task: 2 });
            let decision = |outcome, risk| Decision {
                outcome,
                failure: None,
                risk,
                reason: "Reviewer inspected the retained evidence".into(),
                criteria: vec![],
                limitations: vec![],
            };
            fixture.action(Action::Decide {
                task: 1,
                decision: decision(Outcome::NeedsHumanReview, None),
            });
            let namespace = fixture.store.conversation_directory().unwrap();
            let path = namespace.join("tasks.json");
            let mut legacy = serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap();
            legacy["schema_version"] = 12.into();
            let legacy_bytes = serde_json::to_vec_pretty(&legacy).unwrap();
            assert!(!String::from_utf8_lossy(&legacy_bytes).contains("\"risk\""));
            fs::write(&path, &legacy_bytes).unwrap();
            assert!(fixture
                .store
                .snapshot()
                .unwrap()
                .decision_for_task(1)
                .unwrap()
                .risk
                .is_none());
            for outcome in [
                Outcome::Rejected,
                Outcome::NeedsRepair,
                Outcome::NeedsHumanReview,
            ] {
                let request = Request {
                    correlation: format!("risk-{outcome:?}"),
                    expected_revision: fixture.store.snapshot().unwrap().revision,
                    action: Action::Decide {
                        task: 1,
                        decision: decision(outcome, Some(risk)),
                    },
                };
                let (held, receipt) = fixture.store.transact(request.clone()).unwrap();
                assert_eq!(held.schema_version, alfredo_tui::tasks::SCHEMA_VERSION);
                assert_eq!(held.tasks[0].status, TaskStatus::NeedsHumanReview);
                assert_eq!(held.task_status_label(&held.tasks[0]), "Needs human review");
                let activity = alfredo_tui::activity::entries(&held, "#1");
                assert_eq!(
                    activity[0].summary,
                    format!("Human review required: {}", risk.label())
                );
                assert!(activity[0].detail.contains(outcome.label()));

                assert!(
                    alfredo_tui::task_view::readiness(&held, &held.tasks[0]).contains(risk.label())
                );
                assert!(held
                    .review_summary_for_task(1)
                    .unwrap()
                    .contains("escalated to human review"));
                let reopen =
                    TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission")
                        .unwrap();
                assert_eq!(reopen.transact(request.clone()).unwrap().1, receipt);
                let bytes = fs::read(&path).unwrap();
                let mut conflict = request;
                conflict.action = Action::Decide {
                    task: 1,
                    decision: decision(outcome, None),
                };
                assert!(reopen.transact(conflict).is_err());
                for action in [
                    Action::Approve { task: 1 },
                    Action::Review {
                        task: 1,
                        accept: true,
                    },
                    Action::Repair {
                        task: 1,
                        reason: "Bypass escalation".into(),
                    },
                ] {
                    assert!(reopen
                        .transact(Request {
                            correlation: "bypass-risk".into(),
                            expected_revision: held.revision,
                            action
                        })
                        .is_err());
                    assert_eq!(fs::read(&path).unwrap(), bytes);
                }
                assert!(reopen.claim_worker(1).is_err());
                let dispatch = alfredo_tui::dispatch::Dispatch {
                    enabled: true,
                    ..Default::default()
                };
                assert_eq!(dispatch.next(&held, &Default::default()), None);
            }
            assert_eq!(
                fs::read(namespace.join("tasks-v12-backup.json")).unwrap(),
                legacy_bytes
            );
            let held = fixture.store.snapshot().unwrap();
            let committed = fs::read(&path).unwrap();
            let mut downgrade = serde_json::to_value(&held).unwrap();
            downgrade["schema_version"] = 12.into();
            let old = serde_json::to_vec(&downgrade).unwrap();
            fs::write(&path, &old).unwrap();
            assert!(fixture.store.snapshot().unwrap_err().contains("schema v13"));
            assert_eq!(fs::read(&path).unwrap(), old);
            fs::write(&path, &committed).unwrap();
            let approval = Request {
                correlation: "human-resolution".into(),
                expected_revision: held.revision,
                action: Action::Decide {
                    task: 1,
                    decision: decision(Outcome::Approved, None),
                },
            };
            if failed {
                assert!(fixture
                    .store
                    .transact(approval)
                    .unwrap_err()
                    .contains("successful check"));
                assert_eq!(fs::read(&path).unwrap(), committed);
                fixture.action(Action::Decide {
                    task: 1,
                    decision: decision(Outcome::NeedsRepair, None),
                });
                fixture.action(Action::Repair {
                    task: 1,
                    reason: "Human authorized a repair proposal".into(),
                });
                let repaired = fixture.store.snapshot().unwrap();
                assert_eq!(repaired.tasks[2].status, TaskStatus::Proposed);
                assert!(repaired.tasks[2].run.is_none());
            } else {
                let (accepted, _) = fixture.store.transact(approval).unwrap();
                assert_eq!(accepted.tasks[0].status, TaskStatus::Accepted);
                let dispatch = alfredo_tui::dispatch::Dispatch {
                    enabled: true,
                    ..Default::default()
                };
                assert_eq!(dispatch.next(&accepted, &Default::default()), Some(2));
                assert!(!accepted
                    .review_summary_for_task(1)
                    .unwrap()
                    .contains("Risk:"));
            }
        }
    }
}

#[tokio::test]
async fn repairs_continue_recorded_agent_then_restart_fresh_after_second_rejection() {
    let fixture = Fixture::new();
    fixture.permit();
    let mut root_agent = String::new();
    let mut prior_run = String::new();
    let mut first_request = serde_json::Value::Null;
    let mut first_answer = String::new();
    for task in 1..=3 {
        if task > 1 {
            fixture.action(Action::Review {
                task: task - 1,
                accept: false,
            });
            fixture.action(Action::Repair {
                task: task - 1,
                reason: "Clarify the explanation".into(),
            });
            fixture.action(Action::Approve { task });
        }
        let before = fixture.store.snapshot().unwrap();
        if task == 2 {
            let original: worker::Evidence =
                serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
            let path = fixture
                .store
                .run_directory(&original.run)
                .unwrap()
                .join("agent-conversation.json");
            let bytes = fs::read(&path).unwrap();
            let state_path = fixture
                .store
                .conversation_directory()
                .unwrap()
                .join("tasks.json");
            let state = fs::read(&state_path).unwrap();
            fs::write(&path, b"tampered").unwrap();
            let result = worker::start(
                fixture.store.clone(),
                task,
                "tampered-history".into(),
                before.revision,
                Ollama::new("http://127.0.0.1:9", Duration::from_millis(100)).unwrap(),
                Arc::new(AtomicBool::new(false)),
            )
            .await;
            assert!(result
                .unwrap_err()
                .contains("conversation size or digest mismatch"));
            assert_eq!(fs::read(&state_path).unwrap(), state);
            fs::remove_file(&path).unwrap();
            assert!(worker::start(
                fixture.store.clone(),
                task,
                "missing-history".into(),
                before.revision,
                Ollama::new("http://127.0.0.1:9", Duration::from_millis(100)).unwrap(),
                Arc::new(AtomicBool::new(false))
            )
            .await
            .is_err());
            assert_eq!(fs::read(&state_path).unwrap(), state);
            fs::write(&path, bytes).unwrap();
        }
        let (capture, received) = std::sync::mpsc::channel();
        let (provider, job, _) =
            server_with_capture(good_plan(), Duration::ZERO, "return 0", Some(capture));
        let reopened =
            TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission").unwrap();
        let (finished, _) = worker::start(
            reopened,
            task,
            format!("agent-{task}"),
            before.revision,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        job.join().unwrap();
        assert_eq!(
            finished.tasks[(task - 1) as usize].status,
            TaskStatus::ReviewReady
        );
        let request = received.recv().unwrap();
        let messages = request["messages"].as_array().unwrap();
        let evidence: worker::Evidence =
            serde_json::from_str(&fixture.store.evidence(task).unwrap()).unwrap();
        let agent = evidence.agent.as_ref().unwrap();
        let artifact = fixture
            .store
            .run_directory(&evidence.run)
            .unwrap()
            .join("agent-conversation.json");
        let bytes = fs::read(artifact).unwrap();
        use sha2::{Digest, Sha256};
        assert_eq!(
            agent.transcript_sha256.as_deref(),
            Some(format!("{:x}", Sha256::digest(&bytes)).as_str())
        );
        let retained: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(retained["run"], evidence.run);
        if task == 1 {
            assert_eq!(messages.len(), 1);
            assert_eq!(agent.agent, evidence.run);
            assert!(agent.continued_from.is_none());
            root_agent = agent.agent.clone();
            first_request = messages[0].clone();
            first_answer = retained["messages"][1]["content"].as_str().unwrap().into();
        } else if task == 2 {
            assert_eq!(messages.len(), 3);
            assert_eq!(agent.agent, root_agent);
            assert_eq!(agent.continued_from.as_deref(), Some(prior_run.as_str()));
            assert_eq!(messages[0], first_request);
            assert_eq!(messages[1]["role"], "assistant");
            // A retained legacy JSON answer is replayed in the requested block format.
            assert!(first_answer.starts_with('{'));
            assert_eq!(
                messages[1]["content"],
                worker::render_blocks(&worker::parse_answer(&first_answer).unwrap())
            );
            assert!(messages[2]["content"]
                .as_str()
                .unwrap()
                .contains("REPAIR CONTEXT"));
            assert!(messages[2]["content"]
                .as_str()
                .unwrap()
                .contains("current exact file/check policy"));
        } else {
            assert_eq!(messages.len(), 1);
            assert_ne!(agent.agent, root_agent);
            assert_eq!(agent.agent, evidence.run);
            assert!(agent.continued_from.is_none());
            assert!(agent.reason.contains("Second or later rejection"));
            assert!(messages[0]["content"]
                .as_str()
                .unwrap()
                .contains("Prior task #2"));
        }
        let view =
            alfredo_tui::review::View::from_verified(task, &fixture.store.evidence(task).unwrap())
                .unwrap();
        assert!(view
            .lines()
            .iter()
            .any(|line| line.to_string().contains(&agent.agent)));
        prior_run = evidence.run;
    }
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("calc.py")).unwrap(),
        "def answer():\n    return 0\n"
    );
}

#[tokio::test]
async fn continuity_typed_rejections_count_but_needs_repair_does_not() {
    use alfredo_tui::assessment::{Decision, Outcome};
    let fixture = Fixture::new();
    fixture.permit();
    let mut first_agent = String::new();
    for task in 1..=4 {
        if task > 1 {
            fixture.action(Action::Decide {
                task: task - 1,
                decision: Decision {
                    outcome: if task == 2 {
                        Outcome::NeedsRepair
                    } else {
                        Outcome::Rejected
                    },
                    reason: "Review requires another revision".into(),
                    criteria: vec![],
                    limitations: vec![],
                    failure: None,
                    risk: None,
                },
            });
            fixture.action(Action::Repair {
                task: task - 1,
                reason: "Revise explanation".into(),
            });
            fixture.action(Action::Approve { task });
        }
        let (capture, received) = std::sync::mpsc::channel();
        let (provider, job, _) =
            server_with_capture(good_plan(), Duration::ZERO, "return 0", Some(capture));
        let reopened =
            TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission").unwrap();
        let revision = reopened.snapshot().unwrap().revision;
        let (snapshot, _) = worker::start(
            reopened,
            task,
            format!("typed-continuity-{task}"),
            revision,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        job.join().unwrap();
        assert_eq!(
            snapshot.tasks[(task - 1) as usize].status,
            TaskStatus::ReviewReady
        );
        let request = received.recv().unwrap();
        let evidence: worker::Evidence =
            serde_json::from_str(&fixture.store.evidence(task).unwrap()).unwrap();
        let agent = evidence.agent.unwrap();
        if task == 1 {
            first_agent = agent.agent.clone();
        }
        if task < 4 {
            assert_eq!(agent.agent, first_agent);
            assert_eq!(
                request["messages"].as_array().unwrap().len(),
                (task * 2 - 1) as usize
            );
        } else {
            assert_ne!(agent.agent, first_agent);
            assert!(agent.continued_from.is_none());
            assert!(agent.reason.contains("Second or later rejection"));
            assert_eq!(request["messages"].as_array().unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn continuity_model_change_and_incomplete_exchange_start_fresh_after_approval() {
    for incomplete in [false, true] {
        let fixture = Fixture::new();
        fixture.permit();
        let (provider, job, _) =
            server_with_completion(good_plan(), Duration::ZERO, "return 0", None, !incomplete);
        let revision = fixture.store.snapshot().unwrap().revision;
        let (snapshot, _) = worker::start(
            fixture.store.clone(),
            1,
            "original".into(),
            revision,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        job.join().unwrap();
        assert_eq!(
            snapshot.tasks[0].status,
            if incomplete {
                TaskStatus::Failed
            } else {
                TaskStatus::ReviewReady
            }
        );
        let parent: worker::Evidence =
            serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
        assert_eq!(
            parent.agent.as_ref().unwrap().transcript_sha256.is_none(),
            incomplete
        );
        if !incomplete {
            fixture.action(Action::Review {
                task: 1,
                accept: false,
            });
        }
        fixture.action(Action::Repair {
            task: 1,
            reason: "Complete the approved work".into(),
        });
        if !incomplete {
            fixture.action(Action::Assign {
                task: 2,
                model: "different-fixture".into(),
            });
        }
        let state_path = fixture
            .store
            .conversation_directory()
            .unwrap()
            .join("tasks.json");
        let original = fs::read(&state_path).unwrap();
        let before = fixture.store.snapshot().unwrap();
        assert_eq!(before.tasks[1].policy, before.tasks[0].policy);
        assert!(worker::start(
            fixture.store.clone(),
            2,
            "without-approval".into(),
            before.revision,
            Ollama::new("http://127.0.0.1:9", Duration::from_millis(100)).unwrap(),
            Arc::new(AtomicBool::new(false))
        )
        .await
        .is_err());
        assert_eq!(fs::read(&state_path).unwrap(), original);
        fixture.action(Action::Approve { task: 2 });
        let (capture, received) = std::sync::mpsc::channel();
        let (provider, job, _) =
            server_with_capture(good_plan(), Duration::ZERO, "return 0", Some(capture));
        let reopened =
            TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission").unwrap();
        let revision = reopened.snapshot().unwrap().revision;
        let (snapshot, _) = worker::start(
            reopened,
            2,
            "fresh-repair".into(),
            revision,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        job.join().unwrap();
        assert_eq!(snapshot.tasks[1].status, TaskStatus::ReviewReady);
        let evidence: worker::Evidence =
            serde_json::from_str(&fixture.store.evidence(2).unwrap()).unwrap();
        let agent = evidence.agent.unwrap();
        assert_eq!(agent.agent, evidence.run);
        assert_ne!(agent.agent, parent.agent.unwrap().agent);
        assert!(agent.continued_from.is_none());
        assert_eq!(
            agent.reason,
            if incomplete {
                "Prior model exchange did not complete"
            } else {
                "Worker model changed"
            }
        );
        let request = received.recv().unwrap();
        assert_eq!(request["model"], snapshot.tasks[1].model);
        let messages = request["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        let prompt = messages[0]["content"].as_str().unwrap();
        assert!(prompt.contains("REPAIR CONTEXT"));
        assert!(prompt.contains("current exact file/check policy"));
        assert!(prompt.contains("calc.py") && prompt.contains("notes.txt"));
    }
}

#[tokio::test]
async fn review_and_repair_is_atomic_replayable_and_requires_fresh_approval() {
    use alfredo_tui::{
        assessment::{Criterion, Decision, Outcome, ReviewRisk},
        planner::{Plan, Step},
    };
    for outcome in [Outcome::NeedsRepair, Outcome::Rejected] {
        let fixture = Fixture::new();
        fixture.permit();
        let policy = fixture.store.snapshot().unwrap().tasks[0]
            .policy
            .clone()
            .unwrap();
        fixture.action(Action::Cancel { task: 1 });
        fixture.action(Action::Plan {
            plan: Plan {
                architecture: None,
                prompt: "Implement answer".into(),
                planner: "fixture".into(),
                context: None,
                scope: None,
                tasks: vec![Step {
                    title: "Return 42 and add notes".into(),
                    acceptance: vec!["Answer is 42".into()],
                    model: "fixture".into(),
                    dependencies: vec![],
                    policy: policy.clone(),
                }],
            },
        });
        fixture.action(Action::Approve { task: 2 });
        let (provider, job, _) = server(good_plan(), Duration::ZERO);
        worker::start(
            fixture.store.clone(),
            2,
            "review-route-original".into(),
            fixture.store.snapshot().unwrap().revision,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        job.join().unwrap();
        let ready = fixture.store.snapshot().unwrap();
        let path = fixture
            .store
            .conversation_directory()
            .unwrap()
            .join("tasks.json");
        let decision = Decision {
            outcome,
            failure: None,
            risk: None,
            reason: "Explain why the result meets the contract".into(),
            criteria: vec![Criterion {
                criterion: 1,
                met: false,
                note: "Calculation explanation needs revision".into(),
            }],
            limitations: vec![],
        };
        let request = Request {
            correlation: "review-and-repair".into(),
            expected_revision: ready.revision,
            action: Action::ReviewAndRepair {
                task: 2,
                decision: decision.clone(),
            },
        };
        let original = fs::read(&path).unwrap();
        // An invalid contract or risk-bearing decision cannot partly review the parent.
        for bad in [
            Decision {
                criteria: vec![],
                ..decision.clone()
            },
            Decision {
                failure: None,
                risk: Some(ReviewRisk::Security),
                ..decision.clone()
            },
            Decision {
                outcome: Outcome::NeedsHumanReview,
                ..decision.clone()
            },
        ] {
            assert!(fixture
                .store
                .transact(Request {
                    action: Action::ReviewAndRepair {
                        task: 2,
                        decision: bad
                    },
                    ..request.clone()
                })
                .is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
        let scope = fixture.store.understanding();
        scope
            .transact(alfredo_tui::understanding::Request {
                correlation: "pending-scope".into(),
                expected_revision: 0,
                action: alfredo_tui::understanding::Action::Draft {
                    brief: alfredo_tui::understanding::Brief {
                        destination: "Correct answer".into(),
                        scope: "Calculation".into(),
                        constraints: "Keep policy".into(),
                        uncertainty: "Explain result".into(),
                    },
                },
            })
            .unwrap();
        assert!(fixture.store.transact(request.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        scope
            .transact(alfredo_tui::understanding::Request {
                correlation: "confirm-scope".into(),
                expected_revision: 1,
                action: alfredo_tui::understanding::Action::Confirm { draft_revision: 1 },
            })
            .unwrap();
        let mut legacy = serde_json::to_value(&ready).unwrap();
        legacy["schema_version"] = 13.into();
        let legacy_bytes = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&path, &legacy_bytes).unwrap();
        let artifact = fixture
            .store
            .run_directory(&ready.tasks[1].run.as_ref().unwrap().id)
            .unwrap()
            .join("evidence.json");
        let evidence_bytes = fs::read(&artifact).unwrap();
        let mut tampered = evidence_bytes.clone();
        tampered.push(b' ');
        fs::write(&artifact, tampered).unwrap();
        assert!(fixture.store.transact(request.clone()).is_err());
        assert_eq!(fs::read(&path).unwrap(), legacy_bytes);
        fs::write(&artifact, evidence_bytes).unwrap();
        let (routed, receipt) = fixture.store.transact(request.clone()).unwrap();
        assert_eq!(routed.schema_version, alfredo_tui::tasks::SCHEMA_VERSION);
        assert_eq!(routed.revision, ready.revision + 1);
        assert_eq!(receipt.task, 3);
        for query in ["#2", "#3"] {
            let entries = alfredo_tui::activity::entries(&routed, query);
            assert!(
                entries
                    .iter()
                    .any(|entry| entry.correlation == "review-and-repair"),
                "Missing compound activity for {query}"
            );
        }
        assert_eq!(routed.tasks.len(), 3);
        assert_eq!(routed.tasks[1].status, TaskStatus::Rejected);
        let child = &routed.tasks[2];
        assert_eq!(child.status, TaskStatus::Proposed);
        assert_eq!(child.repair_of, Some(2));
        assert_eq!(child.policy, Some(policy));
        assert_eq!(child.model, routed.tasks[1].model);
        assert_eq!(child.dependencies, routed.tasks[1].dependencies);
        assert!(child.run.is_none());
        assert_eq!(routed.acceptance_for_task(3), routed.acceptance_for_task(2));
        assert_eq!(routed.decision_for_task(2), Some(&decision));
        assert!(routed
            .review_summary_for_task(2)
            .unwrap()
            .contains("Calculation explanation needs revision"));
        assert!(fixture
            .store
            .repair_context(child)
            .unwrap()
            .unwrap()
            .1
            .contains("Calculation explanation needs revision"));
        assert_eq!(
            fs::read(path.parent().unwrap().join("tasks-v13-backup.json")).unwrap(),
            legacy_bytes
        );
        let committed = fs::read(&path).unwrap();
        let reopened =
            TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission").unwrap();
        assert_eq!(reopened.transact(request.clone()).unwrap().1, receipt);
        assert_eq!(fs::read(&path).unwrap(), committed);
        let mut changed = decision.clone();
        changed.reason = "Conflicting review reason".into();
        assert!(reopened
            .transact(Request {
                action: Action::ReviewAndRepair {
                    task: 2,
                    decision: changed
                },
                ..request.clone()
            })
            .is_err());
        assert!(reopened
            .transact(Request {
                correlation: "stale-route".into(),
                ..request.clone()
            })
            .is_err());
        assert!(reopened
            .transact(Request {
                correlation: "duplicate-route".into(),
                expected_revision: routed.revision,
                ..request
            })
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), committed);
        let dispatch = alfredo_tui::dispatch::Dispatch {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(
            dispatch.next(&routed, &std::collections::BTreeSet::new()),
            None
        );
        assert!(worker::start(
            reopened.clone(),
            3,
            "unapproved-route".into(),
            routed.revision,
            Ollama::new("http://127.0.0.1:9", Duration::from_millis(100)).unwrap(),
            Arc::new(AtomicBool::new(false))
        )
        .await
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), committed);
        let mut downgraded = serde_json::to_value(&routed).unwrap();
        downgraded["schema_version"] = 13.into();
        let downgraded = serde_json::to_vec(&downgraded).unwrap();
        fs::write(&path, &downgraded).unwrap();
        assert!(reopened.snapshot().is_err());
        assert_eq!(fs::read(&path).unwrap(), downgraded);
        fs::write(&path, committed).unwrap();
        fixture.action(Action::Approve { task: 3 });
        let approved = reopened.snapshot().unwrap();
        assert_eq!(
            dispatch.next(&approved, &std::collections::BTreeSet::new()),
            Some(3)
        );
        let (capture, received) = std::sync::mpsc::channel();
        let (provider, job, _) = server_with_capture(
            good_plan(),
            Duration::ZERO,
            "Calculation explanation needs revision",
            Some(capture),
        );
        let (finished, _) = worker::start(
            reopened,
            3,
            "approved-route".into(),
            approved.revision,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        job.join().unwrap();
        assert_eq!(finished.tasks[2].status, TaskStatus::ReviewReady);
        assert_eq!(
            received.recv().unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        fixture.action(Action::ReviewAndRepair {
            task: 3,
            decision: decision.clone(),
        });
        let second_route = fixture.store.snapshot().unwrap();
        assert_eq!(second_route.tasks[3].status, TaskStatus::Proposed);
        assert_eq!(second_route.tasks[3].repair_of, Some(3));
        fixture.action(Action::Approve { task: 4 });
        let (capture, received) = std::sync::mpsc::channel();
        let (provider, job, _) = server_with_capture(
            good_plan(),
            Duration::ZERO,
            "Calculation explanation needs revision",
            Some(capture),
        );
        worker::start(
            fixture.store.clone(),
            4,
            "second-routed-repair".into(),
            fixture.store.snapshot().unwrap().revision,
            provider,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        job.join().unwrap();
        let request = received.recv().unwrap();
        let evidence: worker::Evidence =
            serde_json::from_str(&fixture.store.evidence(4).unwrap()).unwrap();
        let agent = evidence.agent.unwrap();
        if outcome == Outcome::Rejected {
            assert_eq!(request["messages"].as_array().unwrap().len(), 1);
            assert!(agent.continued_from.is_none());
            assert!(agent.reason.contains("Second or later rejection"));
        } else {
            assert_eq!(request["messages"].as_array().unwrap().len(), 5);
            assert!(agent.continued_from.is_some());
        }
    }
}

#[tokio::test]
async fn review_and_repair_capacity_refusal_preserves_parent_and_history() {
    use alfredo_tui::assessment::{Decision, Outcome};
    let fixture = Fixture::new();
    fixture.permit();
    let (provider, job, _) = server(good_plan(), Duration::ZERO);
    worker::start(
        fixture.store.clone(),
        1,
        "capacity-parent".into(),
        fixture.store.snapshot().unwrap().revision,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    job.join().unwrap();
    for index in 1..256 {
        fixture.action(Action::Propose {
            title: format!("Capacity fixture {index}"),
            model: "fixture".into(),
            dependencies: vec![],
        });
    }
    let full = fixture.store.snapshot().unwrap();
    assert_eq!(full.tasks.len(), 256);
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let original = fs::read(&path).unwrap();
    let error = fixture
        .store
        .transact(Request {
            correlation: "full-review-route".into(),
            expected_revision: full.revision,
            action: Action::ReviewAndRepair {
                task: 1,
                decision: Decision {
                    outcome: Outcome::NeedsRepair,
                    failure: None,
                    risk: None,
                    reason: "Revise result".into(),
                    criteria: vec![],
                    limitations: vec![],
                },
            },
        })
        .unwrap_err();
    assert!(error.to_lowercase().contains("capacity"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), original);
    let reopened = TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission")
        .unwrap()
        .snapshot()
        .unwrap();
    assert_eq!(reopened.tasks[0].status, TaskStatus::ReviewReady);
    assert_eq!(reopened.revision, full.revision);
    assert!(reopened.decision_for_task(1).is_none());
}

#[tokio::test]
async fn review_and_repair_cannot_bypass_held_child_of_failed_parent() {
    use alfredo_tui::assessment::{Decision, Outcome};
    let fixture = Fixture::new();
    fixture.permit();
    let (provider, job, _) =
        server_with_completion(good_plan(), Duration::ZERO, "return 0", None, false);
    let (failed, _) = worker::start(
        fixture.store.clone(),
        1,
        "failed-parent".into(),
        fixture.store.snapshot().unwrap().revision,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    job.join().unwrap();
    assert_eq!(failed.tasks[0].status, TaskStatus::Failed);
    fixture.action(Action::Repair {
        task: 1,
        reason: "Complete interrupted exchange".into(),
    });
    fixture.action(Action::Approve { task: 2 });
    let (provider, job, _) = server(good_plan(), Duration::ZERO);
    let (completed, _) = worker::start(
        fixture.store.clone(),
        2,
        "held-repair".into(),
        fixture.store.snapshot().unwrap().revision,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    job.join().unwrap();
    assert_eq!(completed.tasks[1].status, TaskStatus::ReviewReady);
    fixture.action(Action::Decide {
        task: 2,
        decision: Decision {
            outcome: Outcome::NeedsHumanReview,
            reason: "Human inspection required".into(),
            failure: None,
            risk: None,
            criteria: vec![],
            limitations: vec![],
        },
    });
    let state = fixture.store.snapshot().unwrap();
    assert_eq!(state.tasks[0].status, TaskStatus::Failed);
    assert_eq!(state.tasks[1].status, TaskStatus::NeedsHumanReview);
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let original = fs::read(&path).unwrap();
    let reopened =
        TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission").unwrap();
    for outcome in [Outcome::NeedsRepair, Outcome::Rejected] {
        let error = reopened
            .transact(Request {
                correlation: format!("bypass-held-{outcome:?}"),
                expected_revision: state.revision,
                action: Action::ReviewAndRepair {
                    task: 1,
                    decision: Decision {
                        outcome,
                        reason: "Request another repair".into(),
                        failure: None,
                        risk: None,
                        criteria: vec![],
                        limitations: vec![],
                    },
                },
            })
            .unwrap_err();
        assert!(error.contains("human review"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), original);
        let retained = reopened.snapshot().unwrap();
        assert_eq!(retained.tasks.len(), 2);
        assert_eq!(retained.revision, state.revision);
        assert_eq!(retained.tasks[0].status, TaskStatus::Failed);
        assert_eq!(retained.tasks[1].status, TaskStatus::NeedsHumanReview);
        assert!(retained.decision_for_task(1).is_none());
    }
}

#[tokio::test]
async fn explicit_repair_resolution_releases_dependents_and_freezes_both_input_identities() {
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 1,
        accept: false,
    });
    fixture.action(Action::Repair {
        task: 1,
        reason: "Clarify result".into(),
    });
    fixture.action(Action::Approve { task: 2 });
    run_dependency_fixture(&fixture, 2, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 2,
        accept: true,
    });
    let child = propose_dependency(&fixture, vec![1], "result.txt");
    fixture.action(Action::Permit { task: child, policy: WorkPolicy {
        files: vec!["calc.py".into(), "result.txt".into()],
        check: vec!["/usr/bin/python3".into(), "-B".into(), "-c".into(), "from calc import answer; from pathlib import Path; assert answer()==42; assert Path('result.txt').read_text()=='CHILD'".into()],
    }});
    fixture.action(Action::Approve { task: child });
    let pinned = propose_dependency(&fixture, vec![2], "pinned.txt");
    let pinned_finished = run_dependency_fixture(
        &fixture,
        pinned,
        one_file("pinned.txt", "PINNED"),
        "(new file)",
    )
    .await;
    let pinned_run = pinned_finished.tasks[pinned as usize - 1]
        .run
        .clone()
        .unwrap();
    assert_eq!(pinned_run.inputs[0].task, 2);
    assert_eq!(pinned_run.inputs[0].source_task, None);
    let before = fixture.store.snapshot().unwrap();
    assert!(before.dependency_source(1).is_err());
    let dispatch = alfredo_tui::dispatch::Dispatch {
        enabled: true,
        ..Default::default()
    };
    assert_eq!(
        dispatch.next(&before, &std::collections::BTreeSet::new()),
        None
    );
    let state_path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let mut old: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    old["schema_version"] = 14.into();
    let old_bytes = serde_json::to_vec(&old).unwrap();
    fs::write(&state_path, &old_bytes).unwrap();
    let request = Request {
        correlation: "resolve-repair".into(),
        expected_revision: before.revision,
        action: Action::ResolveRepair { task: 2 },
    };
    let (resolved, receipt) = fixture.store.transact(request.clone()).unwrap();
    assert_eq!(resolved.tasks[0], before.tasks[0]);
    assert_eq!(
        resolved.tasks[pinned as usize - 1].run.as_ref(),
        Some(&pinned_run)
    );
    assert_eq!(
        resolved.tasks[child as usize - 1],
        before.tasks[child as usize - 1]
    );
    assert_eq!(resolved.resolved_by(1), Some(2));
    assert_eq!(resolved.resolution_for_family(2), Some(2));
    assert_eq!(resolved.dependency_source(1).unwrap().id, 2);
    assert_eq!(
        dispatch.next(&resolved, &std::collections::BTreeSet::new()),
        Some(child)
    );
    assert_eq!(
        fs::read(state_path.with_file_name("tasks-v14-backup.json")).unwrap(),
        old_bytes
    );
    let reopened =
        TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission").unwrap();
    assert_eq!(reopened.snapshot().unwrap().resolved_by(1), Some(2));
    assert_eq!(reopened.transact(request.clone()).unwrap().1, receipt);
    let saved = fs::read(&state_path).unwrap();
    for refused in [
        Request {
            action: Action::ResolveRepair { task: 1 },
            ..request.clone()
        },
        Request {
            correlation: "stale-resolution".into(),
            ..request.clone()
        },
        Request {
            correlation: "duplicate-resolution".into(),
            expected_revision: resolved.revision,
            ..request.clone()
        },
        Request {
            correlation: "repair-after-resolution".into(),
            expected_revision: resolved.revision,
            action: Action::Repair {
                task: 1,
                reason: "Another attempt".into(),
            },
        },
    ] {
        assert!(reopened.transact(refused).is_err());
        assert_eq!(fs::read(&state_path).unwrap(), saved);
    }
    let mut downgrade: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    downgrade["schema_version"] = 14.into();
    let downgraded = serde_json::to_vec(&downgrade).unwrap();
    fs::write(&state_path, &downgraded).unwrap();
    assert!(reopened.snapshot().is_err());
    assert_eq!(fs::read(&state_path).unwrap(), downgraded);
    fs::write(&state_path, &saved).unwrap();
    let finished = run_dependency_fixture(
        &fixture,
        child,
        one_file("result.txt", "CHILD"),
        "return 42",
    )
    .await;
    let inputs = &finished.tasks[child as usize - 1]
        .run
        .as_ref()
        .unwrap()
        .inputs;
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].task, 1);
    assert_eq!(inputs[0].source_task, Some(2));
    assert_eq!(inputs[0].source_id(), 2);
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(child).unwrap()).unwrap();
    assert!(!evidence.patch.contains("diff --git a/calc.py"));
    worker::verify_candidate(&fixture.workspace, &evidence)
        .await
        .unwrap();
    assert_eq!(
        reopened.snapshot().unwrap().tasks[child as usize - 1]
            .run
            .as_ref()
            .unwrap()
            .inputs,
        *inputs
    );
    assert!(fs::read_to_string(fixture.workspace.join("calc.py"))
        .unwrap()
        .contains("return 0"));
    let parent_run = finished.tasks[child as usize - 1]
        .run
        .as_ref()
        .unwrap()
        .clone();
    fixture.action(Action::Review {
        task: child,
        accept: false,
    });
    fixture.action(Action::Repair {
        task: child,
        reason: "Retain repaired dependency inputs".into(),
    });
    let repair_id = fixture.store.snapshot().unwrap().tasks.last().unwrap().id;
    fixture.action(Action::Approve { task: repair_id });
    let repaired = run_dependency_fixture(
        &fixture,
        repair_id,
        one_file("result.txt", "CHILD"),
        "return 42",
    )
    .await;
    let repair_run = repaired.tasks[repair_id as usize - 1].run.as_ref().unwrap();
    assert_eq!(repair_run.baseline, parent_run.baseline);
    assert_eq!(repair_run.inputs, parent_run.inputs);
    assert_eq!(repair_run.inputs[0].task, 1);
    assert_eq!(repair_run.inputs[0].source_task, Some(2));
}

#[tokio::test]
async fn repair_resolution_refuses_tampered_source_and_unresolved_sibling() {
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 1,
        accept: false,
    });
    fixture.action(Action::Repair {
        task: 1,
        reason: "Revise notes".into(),
    });
    fixture.action(Action::Approve { task: 2 });
    run_dependency_fixture(&fixture, 2, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 2,
        accept: true,
    });
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(2).unwrap()).unwrap();
    let evidence_path = fixture
        .store
        .run_directory(&evidence.run)
        .unwrap()
        .join("evidence.json");
    let original = fs::read(&evidence_path).unwrap();
    let before = fs::read(&path).unwrap();
    fs::write(&evidence_path, b"tampered").unwrap();
    assert!(fixture
        .store
        .transact(Request {
            correlation: "tampered-resolution".into(),
            expected_revision: fixture.store.snapshot().unwrap().revision,
            action: Action::ResolveRepair { task: 2 }
        })
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::write(&evidence_path, original).unwrap();
    fixture.action(Action::Repair {
        task: 1,
        reason: "Alternative explanation".into(),
    });
    let revision = fixture.store.snapshot().unwrap().revision;
    let before = fs::read(&path).unwrap();
    assert!(fixture
        .store
        .transact(Request {
            correlation: "unresolved-resolution".into(),
            expected_revision: revision,
            action: Action::ResolveRepair { task: 2 }
        })
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fixture.action(Action::Approve { task: 3 });
    run_dependency_fixture(&fixture, 3, good_plan(), "return 0").await;
    fixture.action(Action::Decide {
        task: 3,
        decision: alfredo_tui::assessment::Decision {
            outcome: alfredo_tui::assessment::Outcome::NeedsHumanReview,
            reason: "Inspect alternate candidate".into(),
            criteria: vec![],
            limitations: vec![],
            failure: None,
            risk: None,
        },
    });
    let revision = fixture.store.snapshot().unwrap().revision;
    let before = fs::read(&path).unwrap();
    assert!(fixture
        .store
        .transact(Request {
            correlation: "held-resolution".into(),
            expected_revision: revision,
            action: Action::ResolveRepair { task: 2 }
        })
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fixture.action(Action::Decide {
        task: 3,
        decision: alfredo_tui::assessment::Decision {
            outcome: alfredo_tui::assessment::Outcome::Rejected,
            reason: "Select the previous candidate".into(),
            criteria: vec![],
            limitations: vec![],
            failure: None,
            risk: None,
        },
    });
    fixture.action(Action::ResolveRepair { task: 2 });
    let resolved = fixture.store.snapshot().unwrap();
    assert_eq!(resolved.resolved_by(1), Some(2));
    assert!(fixture
        .store
        .transact(Request {
            correlation: "sibling-repair-after-resolution".into(),
            expected_revision: resolved.revision,
            action: Action::Repair {
                task: 3,
                reason: "Retry alternate".into()
            }
        })
        .is_err());
}

#[tokio::test]
async fn multigeneration_resolution_composes_alias_diamond_once() {
    let fixture = Fixture::new();
    fixture.permit();
    for id in 1..=3 {
        if id > 1 {
            fixture.action(Action::Review {
                task: id - 1,
                accept: false,
            });
            fixture.action(Action::Repair {
                task: id - 1,
                reason: "Improve explanation".into(),
            });
            fixture.action(Action::Approve { task: id });
        }
        run_dependency_fixture(&fixture, id, good_plan(), "return 0").await;
    }
    fixture.action(Action::Review {
        task: 3,
        accept: true,
    });
    fixture.action(Action::ResolveRepair { task: 3 });
    let resolved = fixture.store.snapshot().unwrap();
    assert_eq!(resolved.resolved_by(1), Some(3));
    assert_eq!(resolved.resolved_by(2), Some(3));
    let left = propose_dependency(&fixture, vec![1], "left.txt");
    run_dependency_fixture(&fixture, left, one_file("left.txt", "LEFT"), "(new file)").await;
    fixture.action(Action::Review {
        task: left,
        accept: true,
    });
    let right = propose_dependency(&fixture, vec![2, 3], "right.txt");
    let right_finished = run_dependency_fixture(
        &fixture,
        right,
        one_file("right.txt", "RIGHT"),
        "(new file)",
    )
    .await;
    let right_inputs = &right_finished.tasks[right as usize - 1]
        .run
        .as_ref()
        .unwrap()
        .inputs;
    assert_eq!(
        right_inputs
            .iter()
            .map(|i| (i.task, i.source_id()))
            .collect::<Vec<_>>(),
        vec![(2, 3), (3, 3)]
    );
    fixture.action(Action::Review {
        task: right,
        accept: true,
    });
    let child = propose_dependency(&fixture, vec![1, left, right], "joined.txt");
    fixture.action(Action::Permit { task: child, policy: WorkPolicy {
        files: vec!["calc.py".into(), "left.txt".into(), "right.txt".into(), "joined.txt".into()],
        check: vec!["/usr/bin/python3".into(), "-B".into(), "-c".into(), "from calc import answer; from pathlib import Path; assert answer()==42; assert Path('left.txt').read_text()=='LEFT'; assert Path('right.txt').read_text()=='RIGHT'".into()],
    }});
    fixture.action(Action::Approve { task: child });
    let finished = run_dependency_fixture(
        &fixture,
        child,
        one_file("joined.txt", "JOINED"),
        "LEFT\0RIGHT\0return 42",
    )
    .await;
    let inputs = &finished.tasks[child as usize - 1]
        .run
        .as_ref()
        .unwrap()
        .inputs;
    assert_eq!(
        inputs
            .iter()
            .map(|i| (i.task, i.source_id()))
            .collect::<Vec<_>>(),
        vec![(1, 3), (left, left), (right, right)]
    );
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(child).unwrap()).unwrap();
    worker::verify_candidate(&fixture.workspace, &evidence)
        .await
        .unwrap();
}

#[tokio::test]
async fn contracted_repair_resolution_requires_recorded_criterion_review_even_for_historical_acceptance(
) {
    use alfredo_tui::{
        assessment::{Assessment, Criterion},
        planner::{Plan, Step},
    };
    let fixture = Fixture::new();
    fixture.permit();
    let policy = fixture.store.snapshot().unwrap().tasks[0]
        .policy
        .clone()
        .unwrap();
    fixture.action(Action::Cancel { task: 1 });
    fixture.action(Action::Plan {
        plan: Plan {
            architecture: None,
            prompt: "Implement answer".into(),
            planner: "fixture".into(),
            context: None,
            scope: None,
            tasks: vec![Step {
                title: "Return 42".into(),
                acceptance: vec!["Answer is 42".into()],
                model: "fixture".into(),
                dependencies: vec![],
                policy,
            }],
        },
    });
    fixture.action(Action::Approve { task: 2 });
    run_dependency_fixture(&fixture, 2, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 2,
        accept: false,
    });
    fixture.action(Action::Repair {
        task: 2,
        reason: "Explain the result".into(),
    });
    fixture.action(Action::Approve { task: 3 });
    run_dependency_fixture(&fixture, 3, good_plan(), "return 0").await;
    let ready = fixture.store.snapshot().unwrap();
    assert!(fixture
        .store
        .transact(Request {
            correlation: "not-yet-reviewed".into(),
            expected_revision: ready.revision,
            action: Action::ResolveRepair { task: 3 }
        })
        .is_err());
    fixture.action(Action::Assess {
        task: 3,
        assessment: Assessment {
            accept: true,
            reason: "Checked result".into(),
            criteria: vec![Criterion {
                criterion: 1,
                met: true,
                note: "Independent Python check returned CHECK_OK".into(),
            }],
        },
    });
    let accepted = fixture.store.snapshot().unwrap();
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let accepted_bytes = fs::read(&path).unwrap();
    let mut historical: serde_json::Value = serde_json::from_slice(&accepted_bytes).unwrap();
    historical["schema_version"] = 10.into();
    let receipt = historical["receipts"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap();
    receipt["request"]["action"] = serde_json::to_value(Action::Review {
        task: 3,
        accept: true,
    })
    .unwrap();
    let historical_bytes = serde_json::to_vec(&historical).unwrap();
    fs::write(&path, &historical_bytes).unwrap();
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks[2].status,
        TaskStatus::Accepted
    );
    assert!(fixture
        .store
        .transact(Request {
            correlation: "unrecorded-criteria".into(),
            expected_revision: accepted.revision,
            action: Action::ResolveRepair { task: 3 }
        })
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), historical_bytes);
    fs::write(&path, &accepted_bytes).unwrap();
    fixture.action(Action::ResolveRepair { task: 3 });
    assert_eq!(fixture.store.snapshot().unwrap().resolved_by(2), Some(3));
}

#[tokio::test]
async fn failed_ancestor_cannot_reenter_review_or_hold_after_repair_resolution() {
    use alfredo_tui::assessment::{Assessment, Decision, Outcome};
    let fixture = Fixture::new();
    fixture.permit();
    let (provider, job, _) = server(one_file("outside-policy.txt", "INVALID"), Duration::ZERO);
    let (failed, _) = worker::start(
        fixture.store.clone(),
        1,
        "failed-original".into(),
        fixture.store.snapshot().unwrap().revision,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    job.join().unwrap();
    assert_eq!(failed.tasks[0].status, TaskStatus::Failed);
    fixture.action(Action::Repair {
        task: 1,
        reason: "Stay within approved paths".into(),
    });
    fixture.action(Action::Approve { task: 2 });
    run_dependency_fixture(&fixture, 2, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 2,
        accept: true,
    });
    fixture.action(Action::ResolveRepair { task: 2 });
    let resolved = fixture.store.snapshot().unwrap();
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let before = fs::read(&path).unwrap();
    for (index, action) in [
        Action::Review {
            task: 1,
            accept: false,
        },
        Action::Assess {
            task: 1,
            assessment: Assessment {
                accept: false,
                reason: "Reject after resolution".into(),
                criteria: vec![],
            },
        },
        Action::Decide {
            task: 1,
            decision: Decision {
                outcome: Outcome::NeedsHumanReview,
                reason: "Hold after resolution".into(),
                criteria: vec![],
                limitations: vec![],
                failure: None,
                risk: None,
            },
        },
        Action::ReviewAndRepair {
            task: 1,
            decision: Decision {
                outcome: Outcome::NeedsRepair,
                reason: "Repair after resolution".into(),
                criteria: vec![],
                limitations: vec![],
                failure: None,
                risk: None,
            },
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert!(fixture
            .store
            .transact(Request {
                correlation: format!("post-resolution-review-{index}"),
                expected_revision: resolved.revision,
                action
            })
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Failed
    );
    assert_eq!(
        fixture
            .store
            .snapshot()
            .unwrap()
            .dependency_source(1)
            .unwrap()
            .id,
        2
    );
    // A matching task projection cannot legitimize an impossible receipt history.
    let mut forged = resolved.clone();
    let mut sibling = forged.tasks[1].clone();
    sibling.id = 3;
    sibling.title = "Repair #1: Forged later repair".into();
    sibling.status = TaskStatus::Proposed;
    sibling.run = None;
    forged.tasks.push(sibling);
    forged.revision += 1;
    forged.receipts.push(alfredo_tui::tasks::Receipt {
        request: Request {
            correlation: "forged-post-resolution-repair".into(),
            expected_revision: resolved.revision,
            action: Action::Repair {
                task: 1,
                reason: "Forged later repair".into(),
            },
        },
        revision: forged.revision,
        task: 3,
    });
    let forged_bytes = serde_json::to_vec(&forged).unwrap();
    fs::write(&path, &forged_bytes).unwrap();
    assert!(fixture.store.snapshot().is_err());
    assert_eq!(fs::read(&path).unwrap(), forged_bytes);
    fs::write(&path, &before).unwrap();
}

fn architecture_failure() -> alfredo_tui::assessment::Decision {
    alfredo_tui::assessment::Decision {
        outcome: alfredo_tui::assessment::Outcome::NeedsRepair,
        reason: "Architecture requires a revised calculation contract".into(),
        criteria: vec![],
        limitations: vec![],
        risk: None,
        failure: Some(alfredo_tui::assessment::FailureKind::Architecture),
    }
}

#[tokio::test]
async fn repeated_architecture_failure_requires_bound_revision_and_fresh_execution_approval() {
    use alfredo_tui::{
        assessment::{Assessment, Criterion},
        planner::{Plan, Step},
    };
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    old["schema_version"] = 15.into();
    let old_bytes = serde_json::to_vec(&old).unwrap();
    fs::write(&path, &old_bytes).unwrap();
    let first = Request {
        correlation: "first-architecture-failure".into(),
        expected_revision: fixture.store.snapshot().unwrap().revision,
        action: Action::ReviewArchitecture {
            task: 1,
            decision: architecture_failure(),
        },
    };
    let (routed, first_receipt) = fixture.store.transact(first.clone()).unwrap();
    assert_eq!(routed.tasks.len(), 2);
    assert_eq!(routed.tasks[1].repair_of, Some(1));
    assert_eq!(routed.tasks[1].status, TaskStatus::Proposed);
    assert!(!routed.architecture_required(1));
    assert_eq!(
        fs::read(path.with_file_name("tasks-v15-backup.json")).unwrap(),
        old_bytes
    );
    assert_eq!(fixture.store.transact(first).unwrap().1, first_receipt);
    fixture.action(Action::Approve { task: 2 });
    run_dependency_fixture(&fixture, 2, good_plan(), "return 0").await;
    let second = Request {
        correlation: "second-architecture-failure".into(),
        expected_revision: fixture.store.snapshot().unwrap().revision,
        action: Action::ReviewArchitecture {
            task: 2,
            decision: architecture_failure(),
        },
    };
    let (held, receipt) = fixture.store.transact(second.clone()).unwrap();
    assert_eq!(held.tasks.len(), 2);
    assert_eq!(held.tasks[1].status, TaskStatus::Rejected);
    assert!(held.architecture_required(2));
    let reopened =
        TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission").unwrap();
    assert!(reopened.snapshot().unwrap().architecture_required(2));
    assert_eq!(reopened.transact(second).unwrap().1, receipt);
    let held_bytes = fs::read(&path).unwrap();
    assert!(reopened
        .transact(Request {
            correlation: "bypass-architect".into(),
            expected_revision: held.revision,
            action: Action::Repair {
                task: 2,
                reason: "Ordinary repair bypass".into()
            }
        })
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), held_bytes);
    let context = reopened.architecture_context(2).unwrap();
    assert_eq!(context.origin.task, 2);
    assert_eq!(context.origin.review_revision, held.revision);
    assert_eq!(context.origin.run, held.tasks[1].run.as_ref().unwrap().id);
    assert_eq!(
        context.origin.evidence_sha256,
        held.tasks[1]
            .run
            .as_ref()
            .unwrap()
            .evidence_sha256
            .clone()
            .unwrap()
    );
    assert_eq!(context.revision, held.revision);
    assert!(context.reference.contains("CHECK_OK"));
    assert!(context.reference.contains("Architecture requires"));
    let revised_policy = WorkPolicy {
        files: vec!["calc.py".into(), "notes.txt".into()],
        check: vec!["/usr/bin/python3".into(), "-B".into(), "-c".into(), "from calc import answer; from pathlib import Path; assert answer()==43; assert Path('notes.txt').read_text()=='ARCHITECT_REVISED'; print('CHECK_OK_REVISED')".into()],
    };
    let plan = Plan {
        prompt: context.prompt.clone(),
        planner: "fixture-architect".into(),
        context: None,
        scope: None,
        architecture: Some(context.origin.clone()),
        tasks: vec![Step {
            title: "Implement revised answer contract".into(),
            acceptance: vec!["Answer equals 43 under revised contract".into()],
            model: "revised-worker".into(),
            dependencies: vec![],
            policy: revised_policy.clone(),
        }],
    };
    for (index, mut stale) in [plan.clone(), plan.clone(), plan.clone()]
        .into_iter()
        .enumerate()
    {
        let origin = stale.architecture.as_mut().unwrap();
        match index {
            0 => origin.review_revision += 1,
            1 => origin.run.push_str("-stale"),
            _ => origin.evidence_sha256 = "0".repeat(64),
        }
        assert!(reopened
            .transact(Request {
                correlation: "stale-architect-origin".into(),
                expected_revision: held.revision,
                action: Action::Plan { plan: stale }
            })
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), held_bytes);
    }
    let evidence_path = reopened
        .run_directory(&context.origin.run)
        .unwrap()
        .join("evidence.json");
    let evidence_bytes = fs::read(&evidence_path).unwrap();
    fs::write(&evidence_path, b"tampered").unwrap();
    assert!(reopened.architecture_context(2).is_err());
    assert!(reopened
        .transact(Request {
            correlation: "tampered-architect-adoption".into(),
            expected_revision: held.revision,
            action: Action::Plan { plan: plan.clone() }
        })
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), held_bytes);
    fs::write(&evidence_path, evidence_bytes).unwrap();
    let adopt = Request {
        correlation: "adopt-architect-revision".into(),
        expected_revision: held.revision,
        action: Action::Plan { plan: plan.clone() },
    };
    let (adopted, adopt_receipt) = reopened.transact(adopt.clone()).unwrap();
    assert_eq!(adopt_receipt.task, 3);
    assert_eq!(adopted.tasks[1], held.tasks[1]);
    assert_eq!(adopted.tasks[2].repair_of, Some(2));
    assert_eq!(adopted.tasks[2].status, TaskStatus::Proposed);
    assert_eq!(adopted.tasks[2].model, "revised-worker");
    assert_eq!(adopted.tasks[2].title, "Implement revised answer contract");
    assert_eq!(adopted.tasks[2].policy.as_ref(), Some(&revised_policy));
    assert_eq!(adopted.tasks[2].dependencies, held.tasks[1].dependencies);
    assert_eq!(adopted.acceptance_for_task(3), plan.tasks[0].acceptance);
    assert!(!adopted.architecture_required(2));
    assert_eq!(reopened.transact(adopt).unwrap().1, adopt_receipt);
    assert!(reopened
        .transact(Request {
            correlation: "duplicate-architect-adoption".into(),
            expected_revision: adopted.revision,
            action: Action::Plan { plan }
        })
        .is_err());
    assert!(reopened.claim_worker(3).is_err());
    let committed = fs::read(&path).unwrap();
    let mut downgrade: serde_json::Value = serde_json::from_slice(&committed).unwrap();
    downgrade["schema_version"] = 15.into();
    let downgrade_bytes = serde_json::to_vec(&downgrade).unwrap();
    fs::write(&path, &downgrade_bytes).unwrap();
    assert!(reopened.snapshot().is_err());
    assert_eq!(fs::read(&path).unwrap(), downgrade_bytes);
    fs::write(&path, committed).unwrap();
    fixture.action(Action::Approve { task: 3 });
    let revised_files = || FilePlan {
        files: vec![
            FileEdit {
                path: "calc.py".into(),
                content: "def answer():\n    return 43\n".into(),
            },
            FileEdit {
                path: "notes.txt".into(),
                content: "ARCHITECT_REVISED".into(),
            },
        ],
    };
    let finished = run_dependency_fixture(
        &fixture,
        3,
        revised_files(),
        "return 0\0Answer equals 43 under revised contract",
    )
    .await;
    assert_eq!(
        finished.tasks[2].run.as_ref().unwrap().baseline,
        held.tasks[1].run.as_ref().unwrap().baseline
    );
    let mut first_new_cycle = architecture_failure();
    first_new_cycle.criteria = vec![Criterion {
        criterion: 1,
        met: false,
        note: "Revised design needs one local correction".into(),
    }];
    fixture.action(Action::ReviewArchitecture {
        task: 3,
        decision: first_new_cycle,
    });
    let new_cycle = fixture.store.snapshot().unwrap();
    assert_eq!(new_cycle.tasks.len(), 4);
    assert_eq!(new_cycle.tasks[3].repair_of, Some(3));
    assert!(!new_cycle.architecture_required(3));
    fixture.action(Action::Approve { task: 4 });
    run_dependency_fixture(
        &fixture,
        4,
        revised_files(),
        "return 0\0Answer equals 43 under revised contract",
    )
    .await;
    fixture.action(Action::Assess {
        task: 4,
        assessment: Assessment {
            accept: true,
            reason: "Revised architecture verified".into(),
            criteria: vec![Criterion {
                criterion: 1,
                met: true,
                note: "Independent revised check passed".into(),
            }],
        },
    });
    fixture.action(Action::ResolveRepair { task: 4 });
    assert_eq!(fixture.store.snapshot().unwrap().resolved_by(1), Some(4));
}

#[tokio::test]
async fn architecture_classification_counts_distinct_runs_and_preserves_human_risk_precedence() {
    use alfredo_tui::assessment::{Outcome, ReviewRisk};
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    let mut risk = architecture_failure();
    risk.risk = Some(ReviewRisk::Security);
    let before = fixture.store.snapshot().unwrap();
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let original = fs::read(&path).unwrap();
    for (index, action) in [
        Action::Decide {
            task: 1,
            decision: architecture_failure(),
        },
        Action::ReviewAndRepair {
            task: 1,
            decision: architecture_failure(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert!(fixture
            .store
            .transact(Request {
                correlation: format!("bypass-architecture-route-{index}"),
                expected_revision: before.revision,
                action
            })
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    assert!(fixture
        .store
        .transact(Request {
            correlation: "risk-cannot-auto-repair".into(),
            expected_revision: before.revision,
            action: Action::ReviewArchitecture {
                task: 1,
                decision: risk.clone()
            }
        })
        .is_err());
    fixture.action(Action::Decide {
        task: 1,
        decision: risk,
    });
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks[0].status,
        TaskStatus::NeedsHumanReview
    );
    assert_eq!(fixture.store.snapshot().unwrap().tasks.len(), 1);
    let mut repeat = architecture_failure();
    repeat.outcome = Outcome::NeedsHumanReview;
    fixture.action(Action::Decide {
        task: 1,
        decision: repeat.clone(),
    });
    fixture.action(Action::Decide {
        task: 1,
        decision: repeat,
    });
    fixture.action(Action::ReviewArchitecture {
        task: 1,
        decision: architecture_failure(),
    });
    let routed = fixture.store.snapshot().unwrap();
    assert_eq!(
        routed.tasks.len(),
        2,
        "Repeated reviews of the same run must not trigger the second-run escalation"
    );
    assert!(!routed.architecture_required(1));
    assert_eq!(routed.tasks[1].status, TaskStatus::Proposed);
}

#[tokio::test]
async fn architect_adoption_inherits_declared_dependencies_and_frozen_execution_inputs() {
    use alfredo_tui::planner::{Plan, Step};
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    fixture.action(Action::Review {
        task: 1,
        accept: true,
    });
    let consumer = propose_dependency(&fixture, vec![1], "result.txt");
    run_dependency_fixture(
        &fixture,
        consumer,
        one_file("result.txt", "FIRST"),
        "(new file)",
    )
    .await;
    fixture.action(Action::ReviewArchitecture {
        task: consumer,
        decision: architecture_failure(),
    });
    fixture.action(Action::Approve { task: 3 });
    let second =
        run_dependency_fixture(&fixture, 3, one_file("result.txt", "SECOND"), "(new file)").await;
    fixture.action(Action::ReviewArchitecture {
        task: 3,
        decision: architecture_failure(),
    });
    let context = fixture.store.architecture_context(3).unwrap();
    let plan = Plan {
        prompt: context.prompt, planner: "fixture-architect".into(), context: None, scope: None, architecture: Some(context.origin),
        tasks: vec![Step { title: "Revise consumer architecture".into(), model: "fixture".into(), dependencies: vec![], acceptance: vec!["Consumer preserves its accepted calculation dependency".into()], policy: WorkPolicy {
            files: vec!["calc.py".into(), "result.txt".into()],
            check: vec!["/usr/bin/python3".into(), "-B".into(), "-c".into(), "from calc import answer; from pathlib import Path; assert answer()==42; assert Path('result.txt').read_text()=='REVISED'".into()],
        } }],
    };
    let mut invalid = plan.clone();
    invalid.tasks.push(plan.tasks[0].clone());
    invalid.tasks[1].dependencies = vec![1];
    let before = fixture.store.snapshot().unwrap();
    assert!(fixture
        .store
        .transact(Request {
            correlation: "architect-multi-step-refused".into(),
            expected_revision: before.revision,
            action: Action::Plan { plan: invalid }
        })
        .is_err());
    fixture.action(Action::Plan { plan });
    let adopted = fixture.store.snapshot().unwrap();
    assert_eq!(adopted.tasks[3].repair_of, Some(3));
    assert_eq!(adopted.tasks[3].dependencies, vec![1]);
    fixture.action(Action::Approve { task: 4 });
    let finished =
        run_dependency_fixture(&fixture, 4, one_file("result.txt", "REVISED"), "return 42").await;
    let parent = second.tasks[2].run.as_ref().unwrap();
    let child = finished.tasks[3].run.as_ref().unwrap();
    assert_eq!(child.baseline, parent.baseline);
    assert_eq!(child.inputs, parent.inputs);
    assert_eq!(child.inputs[0].source_id(), 1);
}

fn architect_server(
    done: bool,
    title: &str,
) -> (
    Ollama,
    thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<serde_json::Value>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Ollama::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        Duration::from_secs(5),
    )
    .unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let title = title.to_string();
    let job = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("Architect request did not arrive: {error}"),
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut header = vec![];
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        let header = String::from_utf8(header).unwrap();
        let length: usize = header
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0; length];
        socket.read_exact(&mut body).unwrap();
        send.send(serde_json::from_slice(&body).unwrap()).unwrap();
        let tasks = serde_json::json!({"tasks":[{"title":title,"model":"fixture","acceptance":["Answer preserves 42"],"dependencies":[],"policy":{"files":["calc.py","notes.txt"],"check":["/bin/true"]}}]});
        let response = format!(
            "{}\n",
            serde_json::json!({"message":{"content":tasks.to_string()},"done":done})
        );
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).unwrap();
    });
    (provider, job, receive)
}

fn finish_architect(planner: &mut alfredo_tui::planner::Planner) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while planner.active() {
        planner.poll();
        assert!(
            Instant::now() < deadline,
            "Architect did not finish: {}",
            planner.notice
        );
        thread::sleep(Duration::from_millis(10));
    }
    planner.poll();
}

#[test]
fn architect_planner_uses_verified_lineage_restores_and_revises_without_replaying_inference() {
    use alfredo_tui::planner::{Planner, SavedDraft};
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    fixture.permit();
    runtime.block_on(run_dependency_fixture(&fixture, 1, good_plan(), "return 0"));
    fixture.action(Action::ReviewArchitecture {
        task: 1,
        decision: architecture_failure(),
    });
    fixture.action(Action::Approve { task: 2 });
    runtime.block_on(run_dependency_fixture(&fixture, 2, good_plan(), "return 0"));
    fixture.action(Action::ReviewArchitecture {
        task: 2,
        decision: architecture_failure(),
    });
    let context = fixture.store.architecture_context(2).unwrap();
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let original = fs::read(&path).unwrap();
    let (provider, job, wire) = architect_server(false, "Incomplete draft");
    let mut disconnected = Planner::default();
    disconnected
        .revise_architecture(&runtime, provider, 2, fixture.store.clone())
        .unwrap();
    finish_architect(&mut disconnected);
    job.join().unwrap();
    assert!(wire.recv().is_ok());
    assert!(disconnected.draft.is_none());
    assert!(disconnected.checkpoint().is_none());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(fixture.store.snapshot().unwrap().architecture_required(2));
    let (provider, job, wire) = architect_server(true, "Revised architecture");
    let mut planner = Planner::default();
    planner
        .revise_architecture(&runtime, provider, 2, fixture.store.clone())
        .unwrap();
    finish_architect(&mut planner);
    job.join().unwrap();
    let request = wire.recv().unwrap();
    assert_eq!(request["model"], context.model);
    assert_eq!(request["think"], false);
    assert_eq!(request["format"]["properties"]["tasks"]["maxItems"], 1);
    assert_eq!(
        request["format"]["properties"]["tasks"]["items"]["properties"]["dependencies"]["maxItems"],
        0
    );
    let messages = request["messages"].to_string();
    assert!(
        messages.contains("CHECK_OK")
            && messages.contains("Architecture requires")
            && messages.contains("Task #1")
            && messages.contains("Task #2")
    );
    assert!(messages.contains("Contract:"));
    assert!(messages.len() < 256 * 1024);
    let draft = planner.draft.as_ref().expect(&planner.notice);
    assert_eq!(draft.architecture.as_ref(), Some(&context.origin));
    assert_eq!(draft.context.as_ref().unwrap().baseline, context.baseline);
    assert_eq!(draft.tasks.len(), 1);
    assert!(draft.tasks[0].dependencies.is_empty());
    assert_eq!(fs::read(&path).unwrap(), original);
    let saved = planner.checkpoint().unwrap();
    let restored_saved: SavedDraft =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    drop(planner);
    let mut restored = Planner::default();
    restored.restore(restored_saved).unwrap();
    assert!(!restored.active());
    assert_eq!(
        restored.draft.as_ref().unwrap().architecture.as_ref(),
        Some(&context.origin)
    );
    let (provider, job, wire) = architect_server(true, "Refined architecture");
    restored
        .revise(
            &runtime,
            provider,
            "Clarify the boundary",
            context.revision,
            fixture.store.clone(),
        )
        .unwrap();
    finish_architect(&mut restored);
    job.join().unwrap();
    let revise_request = wire.recv().unwrap();
    assert!(revise_request["messages"]
        .to_string()
        .contains("Clarify the boundary"));
    assert!(revise_request["messages"]
        .to_string()
        .contains("Revised architecture"));
    assert_eq!(
        restored.draft.as_ref().unwrap().architecture.as_ref(),
        Some(&context.origin)
    );
    assert_eq!(
        restored.draft.as_ref().unwrap().tasks[0].title,
        "Refined architecture"
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    use alfredo_tui::understanding::{Action as ScopeAction, Brief, Request as ScopeRequest};
    let scope = fixture.store.understanding();
    let revision = scope.snapshot().unwrap().revision;
    scope
        .transact(ScopeRequest {
            correlation: "pending-architect-scope".into(),
            expected_revision: revision,
            action: ScopeAction::Draft {
                brief: Brief {
                    destination: "Reliable answer".into(),
                    scope: "Calculation".into(),
                    constraints: "Preserve interface".into(),
                    uncertainty: "Design requires confirmation".into(),
                },
            },
        })
        .unwrap();
    assert!(fixture
        .store
        .transact(Request {
            correlation: "pending-architect-adoption".into(),
            expected_revision: context.revision,
            action: Action::Plan {
                plan: restored.draft.clone().unwrap()
            }
        })
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(fixture.store.snapshot().unwrap().architecture_required(2));
}

#[tokio::test]
async fn adopted_architecture_blocks_old_repairs_and_cancelled_draft_can_be_replaced() {
    use alfredo_tui::{
        assessment::{Assessment, Criterion},
        planner::{Plan, Step},
    };
    let fixture = Fixture::new();
    fixture.permit();
    run_dependency_fixture(&fixture, 1, good_plan(), "return 0").await;
    fixture.action(Action::ReviewArchitecture {
        task: 1,
        decision: architecture_failure(),
    });
    fixture.action(Action::Approve { task: 2 });
    run_dependency_fixture(&fixture, 2, good_plan(), "return 0").await;
    fixture.action(Action::ReviewArchitecture {
        task: 2,
        decision: architecture_failure(),
    });
    let context = fixture.store.architecture_context(2).unwrap();
    let plan = Plan {
        architecture: Some(context.origin.clone()),
        prompt: context.prompt.clone(),
        planner: context.model.clone(),
        context: None,
        scope: None,
        tasks: vec![Step {
            title: "Adopt corrected calculation architecture".into(),
            model: "fixture".into(),
            acceptance: vec!["Corrected design returns 42".into()],
            dependencies: vec![],
            policy: fixture.store.snapshot().unwrap().tasks[1]
                .policy
                .clone()
                .unwrap(),
        }],
    };
    let adoption = Request {
        correlation: "initial-architect-adoption".into(),
        expected_revision: context.revision,
        action: Action::Plan { plan: plan.clone() },
    };
    let (adopted, receipt) = fixture.store.transact(adoption.clone()).unwrap();
    assert_eq!(receipt.task, 3);
    assert!(adopted.architecture_obsolete(1));
    assert!(adopted.architecture_obsolete(2));
    assert!(!adopted.architecture_obsolete(3));
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let bytes = fs::read(&path).unwrap();
    for task in [1, 2] {
        assert!(fixture
            .store
            .transact(Request {
                correlation: format!("obsolete-repair-{task}"),
                expected_revision: adopted.revision,
                action: Action::Repair {
                    task,
                    reason: "Bypass adopted design".into()
                }
            })
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fixture.action(Action::Cancel { task: 3 });
    let cancelled = fixture.store.snapshot().unwrap();
    assert_eq!(cancelled.tasks[2].status, TaskStatus::Cancelled);
    assert!(cancelled.tasks[2].run.is_none());
    assert!(cancelled.architecture_required(2));
    let reopened =
        TaskStore::new(&fixture.root.join("state"), &fixture.workspace, "mission").unwrap();
    let replacement_context = reopened.architecture_context(2).unwrap();
    assert_eq!(replacement_context.origin, context.origin);
    assert!(replacement_context.revision > context.revision);
    assert_eq!(reopened.transact(adoption).unwrap().1, receipt);
    assert_eq!(reopened.snapshot().unwrap().tasks.len(), 3);
    assert!(reopened
        .transact(Request {
            correlation: "ordinary-cancelled-architect-bypass".into(),
            expected_revision: cancelled.revision,
            action: Action::Repair {
                task: 2,
                reason: "Skip explicit revision".into()
            }
        })
        .is_err());
    let (replacement, replacement_receipt) = reopened
        .transact(Request {
            correlation: "replace-cancelled-architect-draft".into(),
            expected_revision: replacement_context.revision,
            action: Action::Plan { plan },
        })
        .unwrap();
    assert_eq!(replacement_receipt.task, 4);
    assert_eq!(replacement.tasks[3].repair_of, Some(2));
    assert_eq!(replacement.tasks[3].status, TaskStatus::Proposed);
    assert!(!replacement.architecture_required(2));
    assert!(replacement.architecture_obsolete(3));
    assert!(!replacement.architecture_obsolete(4));
    assert_eq!(replacement.tasks[2].status, TaskStatus::Cancelled);
    fixture.action(Action::Approve { task: 4 });
    run_dependency_fixture(
        &fixture,
        4,
        good_plan(),
        "return 0\0Corrected design returns 42",
    )
    .await;
    fixture.action(Action::Assess {
        task: 4,
        assessment: Assessment {
            accept: true,
            reason: "Revised design checked".into(),
            criteria: vec![Criterion {
                criterion: 1,
                met: true,
                note: "Retained Python check returned CHECK_OK".into(),
            }],
        },
    });
    fixture.action(Action::ResolveRepair { task: 4 });
    let resolved = reopened.snapshot().unwrap();
    assert_eq!(resolved.resolved_by(1), Some(4));
    assert_eq!(resolved.resolved_by(2), Some(4));
}

#[test]
fn prepared_architect_binds_review_source_and_revision_before_inference() {
    use alfredo_tui::{
        planner::Planner,
        planner_command::{Operation, Outcome},
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    fixture.permit();
    runtime.block_on(run_dependency_fixture(&fixture, 1, good_plan(), "return 0"));
    fixture.action(Action::ReviewArchitecture {
        task: 1,
        decision: architecture_failure(),
    });
    fixture.action(Action::Approve { task: 2 });
    runtime.block_on(run_dependency_fixture(&fixture, 2, good_plan(), "return 0"));
    fixture.action(Action::ReviewArchitecture {
        task: 2,
        decision: architecture_failure(),
    });
    let mut planner = Planner::default();
    let before = fixture.store.snapshot().unwrap();
    let stale = planner
        .prepare_command("architect-stale", "/architect-revise 2", "ignored", &before)
        .unwrap()
        .unwrap();
    let Operation::Architect { origin, revision } = &stale.operation else {
        panic!("Missing Architect operation")
    };
    assert_eq!(*revision, before.revision);
    assert_eq!(
        *origin,
        fixture.store.architecture_context(2).unwrap().origin
    );
    assert!(!planner.active());
    fixture.action(Action::Propose {
        title: "Unrelated intervening task".into(),
        model: "fixture".into(),
        dependencies: vec![],
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Ollama::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        Duration::from_secs(1),
    )
    .unwrap();
    planner
        .dispatch_command(&runtime, provider.clone(), &stale, fixture.store.clone())
        .unwrap();
    finish_architect(&mut planner);
    let events = planner.take_command_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].request, stale);
    assert!(
        matches!(&events[0].outcome, Outcome::Failed { reason } if reason.contains("Task state changed"))
    );
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    let current = fixture.store.snapshot().unwrap();
    let valid = planner
        .prepare_command(
            "architect-current",
            "/architect-revise 2",
            "ignored",
            &current,
        )
        .unwrap()
        .unwrap();
    let mut mismatched = valid.clone();
    mismatched.correlation = "architect-wrong-source".into();
    if let Operation::Architect { origin, .. } = &mut mismatched.operation {
        origin.evidence_sha256 = "0".repeat(64);
    }
    planner
        .dispatch_command(&runtime, provider, &mismatched, fixture.store.clone())
        .unwrap();
    finish_architect(&mut planner);
    let events = planner.take_command_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].request, mismatched);
    assert!(
        matches!(&events[0].outcome, Outcome::Failed { reason } if reason.contains("Architect source"))
    );
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert!(planner.checkpoint().is_none());
    let (provider, job, wire) = architect_server(true, "Prepared architecture revision");
    planner
        .dispatch_command(&runtime, provider, &valid, fixture.store.clone())
        .unwrap();
    finish_architect(&mut planner);
    job.join().unwrap();
    assert!(wire.recv().is_ok());
    let saved = planner.checkpoint().unwrap();
    assert_eq!(saved.origin.as_ref(), Some(&valid));
    assert_eq!(saved.revision, current.revision);
    let events = planner.take_command_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].request, valid);
    assert_eq!(Some(events[0].outcome.clone()), saved.outcome_for(&valid));
    assert_eq!(fixture.store.snapshot().unwrap().revision, current.revision);
}

#[test]
fn automatic_architect_waits_for_saved_review_origin_and_keeps_draft_adoption_separate() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::CommandState,
        conversations::{Autosave, ConversationStore},
        model::App,
        planner_command::Outcome,
        task_control::TaskControl,
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    fixture.permit();
    runtime.block_on(run_dependency_fixture(&fixture, 1, good_plan(), "return 0"));
    fixture.action(Action::ReviewArchitecture {
        task: 1,
        decision: architecture_failure(),
    });
    fixture.action(Action::Approve { task: 2 });
    runtime.block_on(run_dependency_fixture(&fixture, 2, good_plan(), "return 0"));
    let before = fixture.store.snapshot().unwrap();
    let mut control = TaskControl::new(fixture.store.clone());
    control.snapshot = Some(before.clone());
    control.observe_scope(fixture.store.understanding().snapshot().unwrap());
    let (provider, job, wire) = architect_server(true, "Saved automatic architecture revision");
    control.set_provider(provider);
    let source = Request {
        correlation: "saved-architecture-review".into(),
        expected_revision: before.revision,
        action: Action::ReviewArchitecture {
            task: 2,
            decision: architecture_failure(),
        },
    };
    let source_intent = Intent::Task {
        request: source.clone(),
    };
    let mut app = App::new("fixture".into());
    let source_id = app.sessions[0]
        .submit_command("/review 2 architecture".into(), source_intent.clone())
        .unwrap();
    let mut autosave = Autosave::new(ConversationStore::open(&fixture.store, "default").unwrap());
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    assert!(autosave.contains_saved_command(0, &app.sessions[0].commands()[0]));
    control.dispatch_prepared(&runtime, &source_intent).unwrap();
    app.sessions[0].set_command_state(&source_id, CommandState::Submitted);
    assert!(
        control.prepare_architect().unwrap().is_none(),
        "Unacknowledged review cannot schedule architecture work"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let held = fixture.store.snapshot().unwrap();
    assert!(held.architecture_required(2));
    assert!(source_intent.reconcile(Some(&held), None).is_some());
    let request = control.prepare_architect().unwrap().unwrap();
    assert_eq!(request.source, source);
    assert!(!control.planner.active());
    assert!(
        control.prepare_architect().unwrap().is_none(),
        "Preparation consumes only this queued review; it never regenerates automatically"
    );
    assert!(matches!(
        wire.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    app.add_session();
    app.sessions[1].insert("Keep reading while Architect drafts");
    let intent = Intent::ArchitectDraft {
        request: request.clone(),
    };
    let id = app.sessions[0]
        .submit_automatic_command("Architect revision for task #2".into(), intent.clone())
        .unwrap();
    assert!(!autosave.contains_saved_command(0, app.sessions[0].commands().last().unwrap()));
    assert!(matches!(
        wire.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert_eq!(fixture.store.snapshot().unwrap().revision, held.revision);
    assert!(control.planner.checkpoint().is_none());
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    assert!(autosave.contains_saved_command(0, app.sessions[0].commands().last().unwrap()));
    assert!(!autosave.contains_saved_command(1, app.sessions[0].commands().last().unwrap()));
    let mut wrong = intent.clone();
    let Intent::ArchitectDraft {
        request: wrong_request,
    } = &mut wrong
    else {
        unreachable!()
    };
    wrong_request.source.correlation.push_str("-other");
    wrong.validate().unwrap();
    assert!(control.dispatch_prepared(&runtime, &wrong).is_err());
    assert!(!control.planner.active());
    control.dispatch_prepared(&runtime, &intent).unwrap();
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    while control.planner.active() {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    job.join().unwrap();
    assert!(wire.recv().is_ok());
    let events = control.planner.take_command_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].request, request.request);
    assert!(matches!(
        events[0].outcome,
        Outcome::Generated { tasks: 1, .. }
    ));
    let saved = control.planner.checkpoint().unwrap();
    assert_eq!(saved.origin.as_ref(), Some(&request.request));
    assert_eq!(
        saved.outcome_for(&request.request),
        Some(events[0].outcome.clone())
    );
    assert_eq!(fixture.store.snapshot().unwrap().revision, held.revision);
    assert_eq!(fixture.store.snapshot().unwrap().tasks.len(), 2);
    assert!(intent.reconcile(Some(&held), None).is_none());
    assert!(intent.task_receipts(Some(&held)).is_empty());
    // Persist the draft before its presentation outcome, exercising exact-origin recovery.
    autosave
        .finish(&runtime, &app, Default::default(), Some(saved.clone()))
        .unwrap();
    drop(autosave);
    let saved_store = ConversationStore::open(&fixture.store, "default").unwrap();
    let mut restored = saved_store.load().unwrap().unwrap().restore();
    assert_eq!(restored.selected, 1);
    assert_eq!(
        restored.sessions[1].draft,
        "Keep reading while Architect drafts"
    );
    assert!(restored.sessions[1].commands().is_empty());
    assert!(restored
        .sessions
        .iter()
        .all(|session| session.messages.is_empty()));
    assert_eq!(
        restored.sessions[0].commands()[1].state,
        CommandState::Planner {
            outcome: events[0].outcome.clone()
        }
    );
    assert!(restored.sessions[0].retry_command(&id).is_err());
    let mut restarted = TaskControl::new(fixture.store.clone());
    restarted.snapshot = Some(fixture.store.snapshot().unwrap());
    restarted.observe_scope(fixture.store.understanding().snapshot().unwrap());
    restarted.planner.restore(saved.clone()).unwrap();
    assert!(!restarted.planner.active());
    assert!(restarted.prepare_architect().unwrap().is_none());
    assert!(restarted.planner.take_command_events().is_empty());
    let adopt = restarted
        .prepare_command("/plan-save", "fixture")
        .unwrap()
        .unwrap();
    restored.sessions[0]
        .submit_command("/plan-save".into(), adopt.clone())
        .unwrap();
    let mut autosave = Autosave::new(saved_store);
    autosave
        .finish(&runtime, &restored, Default::default(), Some(saved))
        .unwrap();
    assert!(autosave.contains_saved_command(0, restored.sessions[0].commands().last().unwrap()));
    restarted.dispatch_prepared(&runtime, &adopt).unwrap();
    while restarted.pending {
        restarted.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let adopted = fixture.store.snapshot().unwrap();
    assert_eq!(adopted.tasks.len(), 3);
    assert_eq!(adopted.tasks[2].status, TaskStatus::Proposed);
    assert!(adopted.tasks[2].run.is_none());
    assert!(adopt.reconcile(Some(&adopted), None).is_some());
    assert!(
        fixture.store.claim_worker(3).is_err(),
        "Draft save never supplies execution approval"
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("calc.py")).unwrap(),
        "def answer():\n    return 0\n"
    );
}

#[test]
fn queued_automatic_architect_revalidates_canonical_source_after_model_admission() {
    use alfredo_tui::{
        command_intent::Intent, model::Message, planner_command::Outcome, task_control::TaskControl,
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    fixture.permit();
    runtime.block_on(run_dependency_fixture(&fixture, 1, good_plan(), "return 0"));
    fixture.action(Action::ReviewArchitecture {
        task: 1,
        decision: architecture_failure(),
    });
    fixture.action(Action::Approve { task: 2 });
    runtime.block_on(run_dependency_fixture(&fixture, 2, good_plan(), "return 0"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Ollama::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        Duration::from_secs(5),
    )
    .unwrap()
    .with_parallelism(1)
    .unwrap()
    .with_priority(alfredo_tui::inference_admission::Class::Background);
    let (seen, wire) = std::sync::mpsc::channel();
    let (release, held) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) => panic!("Blocking model request did not arrive: {error}"),
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut header = vec![];
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        let header = String::from_utf8(header).unwrap();
        let length: usize = header
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0; length];
        socket.read_exact(&mut body).unwrap();
        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(request["model"], "occupied-slot");
        seen.send(()).unwrap();
        held.recv_timeout(Duration::from_secs(10)).unwrap();
        let response = "{\"message\":{\"content\":\"Done\"},\"done\":true}\n";
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                )
                .as_bytes(),
            )
            .unwrap();
        listener
    });
    let (events, _events) = tokio::sync::mpsc::channel(32);
    let occupying_provider = provider.clone();
    let occupying = runtime.spawn(async move {
        occupying_provider
            .chat(
                0,
                1,
                "occupied-slot".into(),
                vec![Message {
                    role: "user".into(),
                    content: "Hold the only slot".into(),
                }],
                events,
            )
            .await;
    });
    wire.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut control = TaskControl::new(fixture.store.clone());
    control.snapshot = Some(fixture.store.snapshot().unwrap());
    control.observe_scope(fixture.store.understanding().snapshot().unwrap());
    control.set_provider(provider);
    let source = Request {
        correlation: "queued-architecture-review".into(),
        expected_revision: control.snapshot.as_ref().unwrap().revision,
        action: Action::ReviewArchitecture {
            task: 2,
            decision: architecture_failure(),
        },
    };
    control
        .dispatch_prepared(&runtime, &Intent::Task { request: source })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.pending {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let request = control.prepare_architect().unwrap().unwrap();
    // The saved-intent gate is covered separately; this exercises the later queue boundary.
    control
        .dispatch_prepared(
            &runtime,
            &Intent::ArchitectDraft {
                request: request.clone(),
            },
        )
        .unwrap();
    while !control.planner.notice.contains("foreground") {
        control.poll();
        assert!(
            control.planner.active(),
            "Architect stopped before reaching the capacity queue: {}",
            control.planner.notice
        );
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    fixture.action(Action::Propose {
        title: "Task added while Architect is queued".into(),
        model: "fixture".into(),
        dependencies: vec![],
    });
    let changed_revision = fixture.store.snapshot().unwrap().revision;
    release.send(()).unwrap();
    runtime.block_on(occupying).unwrap();
    let listener = server.join().unwrap();
    while control.planner.active() {
        control.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let outcomes = control.planner.take_command_events();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].request, request.request);
    assert!(
        matches!(&outcomes[0].outcome, Outcome::Failed { reason } if reason.contains("Task state changed before model admission")),
        "{:?}",
        outcomes
    );
    assert!(control.planner.checkpoint().is_none());
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "Stale Architect work reached HTTP after queue release"
    );
    assert_eq!(fixture.store.snapshot().unwrap().revision, changed_revision);
}

#[tokio::test]
async fn real_worker_queues_as_background_and_cancels_before_any_model_http() {
    use alfredo_tui::inference_admission::{Class, Coordinator};
    let fixture = Fixture::new();
    fixture.permit();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let coordinator = Coordinator::new(endpoint.clone(), 1).unwrap();
    let occupied = coordinator.acquire(Class::Foreground).await.unwrap();
    let provider = Ollama::new(&endpoint, Duration::from_secs(5))
        .unwrap()
        .with_parallelism(1)
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let store = fixture.store.clone();
    let (observer, progress) = worker::Observer::channel();
    let job = tokio::spawn(async move {
        worker::start_observed(
            store,
            1,
            "cancel-shared-queue".into(),
            3,
            provider,
            flag,
            observer,
        )
        .await
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    while progress.borrow().queue.is_none() {
        assert!(
            !job.is_finished(),
            "Worker ended before shared admission: {}",
            progress.borrow().label()
        );
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let observation = progress.borrow().queue.unwrap();
    assert_eq!(observation.class, Class::Background);
    assert_eq!(observation.active, 1);
    assert_eq!(observation.capacity, 1);
    assert!(progress.borrow().label().contains("background"));
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    cancel.store(true, Ordering::SeqCst);
    let (snapshot, _) = tokio::time::timeout(Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Cancelled);
    let run = snapshot.tasks[0].run.as_ref().unwrap();
    assert_eq!(snapshot.receipts.len(), 5);
    assert!(
        matches!(&snapshot.receipts.last().unwrap().request.action, Action::Finish { task: 1, run: finished, .. } if finished == &run.id)
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.join("calc.py")).unwrap(),
        "def answer():\n    return 0\n"
    );
    assert!(!fixture.workspace.join("notes.txt").exists());
    let worktree = fixture
        .store
        .run_directory(&run.id)
        .unwrap()
        .join("worktree");
    assert_eq!(
        fs::read_to_string(worktree.join("calc.py")).unwrap(),
        "def answer():\n    return 0\n"
    );
    assert!(!worktree.join("notes.txt").exists());
    drop(occupied);
    let available = tokio::time::timeout(
        Duration::from_secs(3),
        coordinator.acquire(Class::Foreground),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    drop(available);
}

// Pause the outer worker future after it reports queue registration. The provider
// task continues independently, so outer-loop cancellation cannot mask a missing
// post-capacity check. A later background permit proves that task left admission.
async fn queued_worker_admission_race(cancel_before_grant: bool) {
    use alfredo_tui::inference_admission::{Class, Coordinator};
    let fixture = Fixture::new();
    fixture.permit();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let coordinator = Coordinator::new(endpoint.clone(), 1).unwrap();
    let occupied = coordinator.acquire(Class::Foreground).await.unwrap();
    let count = Arc::new(AtomicU64::new(0));
    let requests = count.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let server = thread::spawn(move || {
        while !stopping.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut header = vec![];
                    let mut byte = [0];
                    while !header.ends_with(b"\r\n\r\n") {
                        socket.read_exact(&mut byte).unwrap();
                        header.push(byte[0]);
                        assert!(header.len() < 64 * 1024);
                    }
                    let header = String::from_utf8(header).unwrap();
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|s| s.parse::<usize>().unwrap())
                        })
                        .unwrap();
                    let mut body = vec![0; length];
                    socket.read_exact(&mut body).unwrap();
                    requests.fetch_add(1, Ordering::SeqCst);
                    let response = format!(
                        "{}\n",
                        serde_json::json!({"message":{"content":serde_json::to_string(&good_plan()).unwrap()},"done":true})
                    );
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).unwrap();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(error) => panic!("{error}"),
            }
        }
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let (observer, mut progress) = worker::Observer::channel();
    let worker = worker::start_observed(
        fixture.store.clone(),
        1,
        "post-capacity-guard".into(),
        3,
        Ollama::new(&endpoint, Duration::from_secs(3))
            .unwrap()
            .with_parallelism(1)
            .unwrap(),
        cancel.clone(),
        observer,
    );
    tokio::pin!(worker);
    tokio::time::timeout(Duration::from_secs(5), async {
        while progress.borrow().queue.is_none() {
            tokio::select! {
                result = worker.as_mut() => panic!("Worker finished before queue registration: {result:?}"),
                changed = progress.changed() => changed.unwrap(),
            }
        }
    }).await.unwrap();
    assert_eq!(progress.borrow().queue.unwrap().class, Class::Background);
    if cancel_before_grant {
        cancel.store(true, Ordering::SeqCst);
    } else {
        fixture.action(Action::Propose {
            title: "Unrelated task while worker waits".into(),
            model: "fixture".into(),
            dependencies: vec![],
        });
    }
    drop(occupied);
    // Same class FIFO puts this behind the paused worker's provider task. Its
    // eventual admission proves the provider completed its post-capacity path.
    let later = tokio::time::timeout(
        Duration::from_secs(4),
        coordinator.acquire(Class::Background),
    )
    .await
    .unwrap()
    .unwrap();
    drop(later);
    let result = tokio::time::timeout(Duration::from_secs(8), worker.as_mut())
        .await
        .unwrap();
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    let (snapshot, detail) = result.unwrap();
    if cancel_before_grant {
        assert_eq!(snapshot.tasks[0].status, TaskStatus::Cancelled, "{detail}");
        assert_eq!(count.load(Ordering::SeqCst), 0, "Cancelled queued worker reached HTTP when only its provider task was allowed to advance");
    } else {
        assert_eq!(
            snapshot.tasks[0].status,
            TaskStatus::ReviewReady,
            "{detail}"
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(snapshot.tasks[1].status, TaskStatus::Proposed);
    }
}
#[tokio::test]
async fn queued_worker_rechecks_cancellation_after_capacity_before_http() {
    queued_worker_admission_race(true).await;
}
#[tokio::test]
async fn queued_worker_admission_allows_unrelated_task_revision_changes() {
    queued_worker_admission_race(false).await;
}

fn commit_files(fixture: &Fixture, files: &[(&str, &str)]) {
    for (path, content) in files {
        let file = fixture.workspace.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }
    for args in [
        vec!["add", "."],
        vec!["-c", "core.hooksPath=/dev/null", "commit", "-qm", "more"],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&fixture.workspace)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
}

/// Plan one approved task with `policy`; returns its id.
fn plan_task(fixture: &Fixture, prompt: &str, title: &str, policy: WorkPolicy) -> u64 {
    use alfredo_tui::planner::{Plan, Step};
    fixture.action(Action::Plan {
        plan: Plan {
            architecture: None,
            prompt: prompt.into(),
            planner: "fixture".into(),
            context: None,
            scope: None,
            tasks: vec![Step {
                acceptance: vec!["tests pass".into()],
                title: title.into(),
                model: "fixture".into(),
                dependencies: vec![],
                policy,
            }],
        },
    });
    let id = fixture.store.snapshot().unwrap().tasks.len() as u64;
    fixture.action(Action::Approve { task: id });
    id
}

#[tokio::test]
async fn worker_receives_committed_check_and_named_files_as_read_only_reference() {
    let fixture = Fixture::new();
    let test_source = "import unittest\nfrom calc import answer\n\nclass T(unittest.TestCase):\n    def test_answer(self):\n        self.assertEqual(answer(), 42)  # TEST_SOURCE_SENTINEL\n";
    let big = format!("# BIG_SENTINEL\n{}", "x = 1\n".repeat(8 * 1024));
    commit_files(
        &fixture,
        &[
            ("test_calc.py", test_source),
            ("docs/spec.md", "SPEC_SENTINEL: answer is 42\n"),
            ("GOAL_REF.md", "GOAL_SENTINEL\n"),
            ("unrelated.py", "UNRELATED_SENTINEL = 1\n"),
            ("big.py", &big),
        ],
    );
    // Working-file edits are never model input; only the pinned commit is read.
    fs::write(fixture.workspace.join("test_calc.py"), "DIRTY_SENTINEL\n").unwrap();
    let policy = WorkPolicy {
        files: vec!["calc.py".into()],
        check: vec![
            "/usr/bin/python3".into(),
            "-B".into(),
            "-m".into(),
            "unittest".into(),
            "test_calc.py".into(),
        ],
    };
    let task = plan_task(
        &fixture,
        "Make test_calc.py pass; see GOAL_REF.md and big.py.",
        "Implement calc.py per docs/spec.md",
        policy.clone(),
    );
    let (capture, captured) = std::sync::mpsc::channel();
    let plan = one_file("calc.py", "def answer():\n    return 42\n");
    let (provider, server, _) = server_with_capture(plan, Duration::ZERO, "", Some(capture));
    let revision = fixture.store.snapshot().unwrap().revision;
    let (snapshot, detail) = worker::start(
        fixture.store.clone(),
        task,
        "reference-run".into(),
        revision,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    let request = captured.recv().unwrap();
    let prompt = request["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    let reference = prompt
        .find("READ-ONLY REFERENCE FILES")
        .unwrap_or_else(|| panic!("{prompt}"));
    for text in [
        "READ-ONLY FILE test_calc.py\n",
        test_source,
        "READ-ONLY FILE docs/spec.md\nSPEC_SENTINEL: answer is 42\n",
        "READ-ONLY FILE GOAL_REF.md\nGOAL_SENTINEL\n",
        "READ-ONLY REFERENCE OMITTED big.py",
    ] {
        assert!(
            prompt[reference..].contains(text),
            "{text} missing: {prompt}"
        );
    }
    assert!(prompt.contains("never return them"), "{prompt}");
    // Check-argv files come first, before goal/description mentions.
    assert!(
        prompt.find("READ-ONLY FILE test_calc.py").unwrap()
            < prompt.find("READ-ONLY FILE docs/spec.md").unwrap()
    );
    for absent in ["DIRTY_SENTINEL", "UNRELATED_SENTINEL", "BIG_SENTINEL"] {
        assert!(!prompt.contains(absent), "{absent} leaked: {prompt}");
    }
    assert!(
        prompt.contains("Allowed exact files: [\"calc.py\"]"),
        "{prompt}"
    );
    // Blocks answers are unconstrained; the allowed list is enforced after parsing.
    assert!(request.get("format").is_none(), "{request}");
    let finished = snapshot.tasks.iter().find(|t| t.id == task).unwrap();
    assert_eq!(finished.policy.as_ref(), Some(&policy));
    assert_eq!(finished.status, TaskStatus::ReviewReady, "{detail}");
}

#[tokio::test]
async fn unapproved_returned_file_is_named_with_the_allowed_list_and_nothing_is_written() {
    let fixture = Fixture::new();
    fixture.permit();
    let (provider, server, _) = server(one_file("test_calc.py", "hijack"), Duration::ZERO);
    let (snapshot, detail) = worker::start(
        fixture.store.clone(),
        1,
        "unapproved-name".into(),
        3,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Failed);
    assert_eq!(
        detail,
        "Returned unapproved file test_calc.py; only calc.py, notes.txt may be written"
    );
    let directory = fixture
        .store
        .run_directory(&snapshot.tasks[0].run.as_ref().unwrap().id)
        .unwrap();
    assert!(!directory.join("worktree/test_calc.py").exists());
}

#[tokio::test]
async fn failed_check_detail_carries_a_sanitized_relative_output_tail() {
    let fixture = Fixture::new();
    fixture.action(Action::Permit {
        task: 1,
        policy: WorkPolicy {
            files: vec!["calc.py".into()],
            check: vec![
                "/usr/bin/python3".into(),
                "-B".into(),
                "-c".into(),
                "import os, sys; print('OUT_LINE'); [sys.stderr.write(f'noise {i}\\n') for i in range(60)]; sys.stderr.write('  File \"' + os.getcwd() + '/calc.py\", line 2\\n\\x1b[31mAssertionError: TAIL_SENTINEL\\x1b[0m\\n'); sys.exit(1)".into(),
            ],
        },
    });
    fixture.action(Action::Approve { task: 1 });
    let (provider, server, _) = server(
        one_file("calc.py", "def answer():\n    return 0\n"),
        Duration::ZERO,
    );
    let (snapshot, detail) = worker::start(
        fixture.store.clone(),
        1,
        "tail-run".into(),
        3,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Failed);
    let recorded = &snapshot.tasks[0].run.as_ref().unwrap().detail;
    assert_eq!(recorded, &detail);
    assert!(
        detail.starts_with("Check failed (exit 1): stderr: "),
        "{detail}"
    );
    assert!(
        detail.ends_with("File \"calc.py\", line 2 | AssertionError: TAIL_SENTINEL"),
        "{detail}"
    );
    assert!(!detail.contains("/worktree"), "{detail}");
    assert!(
        !detail.contains('\u{1b}') && !detail.contains("[31m"),
        "{detail}"
    );
    assert!(detail.len() <= 1024, "{}", detail.len());
    let tail = worker::output_tail(
        &serde_json::from_str::<worker::Evidence>(&fixture.store.evidence(1).unwrap())
            .unwrap()
            .check
            .unwrap(),
    )
    .unwrap();
    assert_eq!(tail.0, "stderr");
    assert_eq!(tail.1.lines().count(), 40, "{}", tail.1);
    assert!(tail.1.len() <= 4096);
    assert!(
        tail.1.ends_with("AssertionError: TAIL_SENTINEL"),
        "{}",
        tail.1
    );
}

/// Unittest check on calc.py; task 1 is permitted and approved.
fn unittest_fixture() -> Fixture {
    let fixture = Fixture::new();
    commit_files(
        &fixture,
        &[(
            "test_calc.py",
            "import unittest\nfrom calc import answer\n\nclass T(unittest.TestCase):\n    def test_answer(self):\n        self.assertEqual(answer(), 42)\n",
        )],
    );
    fixture.action(Action::Permit {
        task: 1,
        policy: WorkPolicy {
            files: vec!["calc.py".into()],
            check: ["/usr/bin/python3", "-B", "-m", "unittest", "test_calc.py"]
                .map(String::from)
                .to_vec(),
        },
    });
    fixture.action(Action::Approve { task: 1 });
    fixture
}

/// Run `task` against a one-reply fixture; returns the captured request and detail.
async fn run_captured(
    fixture: &Fixture,
    task: u64,
    content: &str,
    done_reason: Option<&'static str>,
) -> (serde_json::Value, String) {
    let (capture, captured) = std::sync::mpsc::channel();
    let (provider, server, _) = server_with_reason(
        one_file("calc.py", content),
        Duration::ZERO,
        "",
        Some(capture),
        true,
        done_reason,
    );
    let revision = fixture.store.snapshot().unwrap().revision;
    let (_, detail) = worker::start(
        fixture.store.clone(),
        task,
        format!("lineage-{task}"),
        revision,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    (captured.recv().unwrap(), detail)
}

fn repair_of(fixture: &Fixture, parent: u64) -> u64 {
    fixture.action(Action::Repair {
        task: parent,
        reason: "Fix the failing test".into(),
    });
    let id = fixture.store.snapshot().unwrap().tasks.len() as u64;
    fixture.action(Action::Approve { task: id });
    id
}

fn last_prompt(request: &serde_json::Value) -> String {
    request["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The "What is still failing" section, which must precede the policy and evidence.
fn failing_section(prompt: &str) -> String {
    let start = prompt
        .find("WHAT IS STILL FAILING")
        .unwrap_or_else(|| panic!("{prompt}"));
    let end = prompt.find("Allowed exact files").unwrap();
    assert!(start < end && end < prompt.find("REPAIR CONTEXT").unwrap());
    prompt[start..end].to_string()
}

fn requested(fixture: &Fixture, task: u64) -> String {
    serde_json::from_str::<worker::Evidence>(&fixture.store.evidence(task).unwrap())
        .unwrap()
        .generation
        .unwrap()
        .summary()
}

#[tokio::test]
async fn identical_repair_is_named_no_progress_and_next_repair_leads_with_failures_hotter() {
    let fixture = unittest_fixture();
    let (request, detail) = run_captured(&fixture, 1, "def answer():\n    return 41\n", None).await;
    assert_eq!(request["options"]["temperature"], 0);
    assert!(!last_prompt(&request).contains("WHAT IS STILL FAILING"));
    assert!(detail.starts_with("Check failed (exit 1)"), "{detail}");

    // Repair 1 keeps the default temperature and leads with the failing assertions.
    let second = repair_of(&fixture, 1);
    let (request, detail) =
        run_captured(&fixture, second, "def answer():\n    return 40\n", None).await;
    assert_eq!(request["options"]["temperature"], 0);
    let prompt = last_prompt(&request);
    assert!(prompt.starts_with("Implement this task: Repair #1:"));
    let section = failing_section(&prompt);
    for line in [
        "FAIL: test_answer",
        "self.assertEqual(answer(), 42)",
        "AssertionError: 41 != 42",
    ] {
        assert!(section.contains(line), "{line} missing: {section}");
    }
    assert!(!section.contains("Traceback") && !section.contains("identical"));
    assert!(!detail.starts_with("No change"), "{detail}");

    // Byte-identical to the previous attempt: named, check output kept, hotter.
    let third = repair_of(&fixture, second);
    let (request, detail) =
        run_captured(&fixture, third, "def answer():\n    return 40\n", None).await;
    assert_eq!(request["options"]["temperature"], 0.3);
    assert!(
        detail.starts_with("No change from previous attempt · Check failed (exit 1): stderr: "),
        "{detail}"
    );
    assert!(detail.contains("AssertionError: 40 != 42"), "{detail}");
    assert!(detail.len() <= 900);
    assert!(requested(&fixture, third).ends_with("temperature 0.3 · answer format blocks"));

    let fourth = repair_of(&fixture, third);
    let (request, detail) =
        run_captured(&fixture, fourth, "def answer():\n    return 42\n", None).await;
    assert_eq!(request["options"]["temperature"], 0.8);
    assert!(requested(&fixture, fourth).ends_with("temperature 0.8 · answer format blocks"));
    // Its own repeated answer is not replayed as conversation history.
    assert_eq!(request["messages"].as_array().unwrap().len(), 1);
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(fourth).unwrap()).unwrap();
    assert!(
        evidence.agent.unwrap().reason.contains("no change"),
        "fresh agent after no progress"
    );
    let section = failing_section(&last_prompt(&request));
    assert!(
        section.contains("previous attempt returned identical code that still fails"),
        "{section}"
    );
    assert!(section.contains("AssertionError: 40 != 42"), "{section}");
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks[3].status,
        TaskStatus::ReviewReady,
        "{detail}"
    );
}

#[tokio::test]
async fn token_limit_failure_is_named_and_next_repair_requests_a_larger_bounded_limit() {
    let fixture = unittest_fixture();
    let content = "def answer():\n    return 42\n";
    let (request, detail) = run_captured(&fixture, 1, content, Some("length")).await;
    assert_eq!(request["options"]["num_predict"], 4096);
    assert!(
        detail.starts_with("Model output hit the 4096-token limit"),
        "{detail}"
    );
    let second = repair_of(&fixture, 1);
    let (request, detail) = run_captured(&fixture, second, content, Some("length")).await;
    assert_eq!(request["options"]["num_predict"], 8192);
    assert!(requested(&fixture, second).contains("token limit 8192"));
    let section = failing_section(&last_prompt(&request));
    assert!(section.contains("hit the 4096-token limit"), "{section}");
    assert!(
        detail.starts_with("Model output hit the 8192-token limit"),
        "{detail}"
    );
    let third = repair_of(&fixture, second);
    let (request, _) = run_captured(&fixture, third, content, None).await;
    assert_eq!(request["options"]["num_predict"], 8192, "bounded");
}

#[test]
fn failing_lines_lead_with_assertions_and_stay_bounded() {
    let output = "F.\n======\nFAIL: test_unique (test_slug.T.test_unique)\n------\nTraceback (most recent call last):\n  File \"test_slug.py\", line 12, in test_unique\n    self.assertEqual(unique_slug(\"A b\", {\"a-b\", \"a-b-2\"}), \"a-b-3\")\nAssertionError: 'a-b-1' != 'a-b-3'\n- a-b-1\n?     ^\n+ a-b-3\n?     ^\n\n------\nRan 6 tests in 0.001s\n\nFAILED (failures=1)\n";
    assert_eq!(
        worker::failing_lines(output),
        "FAIL: test_unique (test_slug.T.test_unique)\n    self.assertEqual(unique_slug(\"A b\", {\"a-b\", \"a-b-2\"}), \"a-b-3\")\nAssertionError: 'a-b-1' != 'a-b-3'\n- a-b-1\n+ a-b-3\nFAILED (failures=1)"
    );
    let noisy = "ModuleNotFoundError: No module named 'slug'\n".to_string()
        + &"FAIL: test_many (t.T.test_many) with a long enough name to count\n".repeat(200);
    let lines = worker::failing_lines(&noisy);
    assert!(lines.starts_with("ModuleNotFoundError: No module named 'slug'"));
    assert!(lines.lines().count() <= 31, "{lines}");
    assert!(lines.len() <= 2048 + 64, "{}", lines.len());
    assert!(lines.ends_with("more failing lines omitted)"), "{lines}");
    assert_eq!(worker::failing_lines("all good\n"), "");
}

fn blocks(files: &[(&str, &str)]) -> String {
    files
        .iter()
        .map(|(path, content)| format!("=== FILE: {path} ===\n{content}=== END FILE ===\n"))
        .collect()
}

#[test]
fn file_blocks_parse_verbatim_code_and_refuse_truncated_or_missing_blocks() {
    // Quotes, backslashes and braces need no escaping; outside text is ignored.
    let code = "def show(todo):\n    print(f'{1}. {todo[\"task\"]}')\n    return \"\\n\"\n";
    let answer = format!(
        "Here is the change.\n{}Done.\n",
        blocks(&[("calc.py", code), ("notes.txt", "note\n\n\n")])
    );
    let plan = worker::parse_answer(&answer).unwrap();
    assert_eq!(plan.files.len(), 2);
    assert_eq!(plan.files[0].path, "calc.py");
    assert_eq!(plan.files[0].content, code);
    // A single trailing newline is normalized.
    assert_eq!(plan.files[1].content, "note\n");
    let crlf = worker::parse_answer(&answer.replace('\n', "\r\n")).unwrap();
    assert_eq!(crlf.files[0].content, code);
    let padded = worker::parse_answer("=== FILE: a.py ===\nx = 1\n\n\n=== END FILE ===\n").unwrap();
    assert_eq!(padded.files[0].content, "x = 1\n");
    // One markdown fence layer inside a block is stripped.
    let fenced =
        worker::parse_answer("=== FILE: a.py ===\n```python\nx = 1\n```\n=== END FILE ===")
            .unwrap();
    assert_eq!(fenced.files[0].content, "x = 1\n");
    // Marker lines must start the line exactly.
    assert_eq!(
        worker::parse_answer("  === FILE: a.py ===\nx\n=== END FILE ===\n").unwrap_err(),
        "Model returned no FILE blocks"
    );
    assert_eq!(
        worker::parse_answer("I cannot do that.").unwrap_err(),
        "Model returned no FILE blocks"
    );
    assert_eq!(
        worker::parse_answer("=== FILE: todo.py ===\nimport json\nprint(f'{todo[").unwrap_err(),
        "Model output ended inside FILE block for todo.py (truncated)"
    );
    // A marker line inside content is ambiguous and refused.
    let nested =
        worker::parse_answer("=== FILE: a.py ===\nx\n=== FILE: b.py ===\ny\n=== END FILE ===\n")
            .unwrap_err();
    assert!(
        nested.contains("a.py") && nested.contains("marker"),
        "{nested}"
    );
    // Legacy JSON answers remain accepted.
    let legacy = worker::parse_answer(&serde_json::to_string(&good_plan()).unwrap()).unwrap();
    assert_eq!(legacy.files.len(), 2);
    assert_eq!(legacy.files[0].content, "def answer():\n    return 42\n");
    // Rendering round-trips through the parser.
    assert_eq!(
        worker::parse_answer(&worker::render_blocks(&plan))
            .unwrap()
            .files[0]
            .content,
        code
    );
    // Duplicates, unapproved paths and binary content are refused by the policy.
    let policy = WorkPolicy {
        files: vec!["calc.py".into(), "notes.txt".into()],
        check: vec!["true".into()],
    };
    let twice = worker::parse_answer(&blocks(&[("calc.py", "a\n"), ("calc.py", "b\n")])).unwrap();
    assert!(worker::validate_plan(&twice, &policy)
        .unwrap_err()
        .contains("more than once"));
    let other = worker::parse_answer(&blocks(&[("test_calc.py", "x\n")])).unwrap();
    assert_eq!(
        worker::validate_plan(&other, &policy).unwrap_err(),
        "Returned unapproved file test_calc.py; only calc.py, notes.txt may be written"
    );
    let binary = worker::parse_answer(&blocks(&[("calc.py", "a\0b\n")])).unwrap();
    assert!(worker::validate_plan(&binary, &policy).is_err());
}

#[test]
fn worker_format_names_parse_and_legacy_generation_reads_unrecorded() {
    assert_eq!(
        worker::WorkerFormat::default(),
        worker::WorkerFormat::Blocks
    );
    assert_eq!(
        worker::WorkerFormat::parse("json"),
        Ok(worker::WorkerFormat::Json)
    );
    assert_eq!(
        worker::WorkerFormat::parse("blocks"),
        Ok(worker::WorkerFormat::Blocks)
    );
    assert!(worker::WorkerFormat::parse("xml").is_err());
    let legacy: alfredo_tui::provider::Generation =
        serde_json::from_str(r#"{"thinking":"off","num_predict":4096,"temperature":0}"#).unwrap();
    assert_eq!(legacy.answer_format, None);
    let mut generation = legacy.clone();
    generation.answer_format = Some(worker::WorkerFormat::Blocks);
    assert!(generation
        .summary()
        .ends_with("temperature 0 · answer format blocks"));
}

#[tokio::test]
async fn blocks_answer_is_requested_without_schema_and_applied_verbatim() {
    let fixture = unittest_fixture();
    let (capture, captured) = std::sync::mpsc::channel();
    let answer = format!(
        "Sure:\n{}",
        blocks(&[("calc.py", "def answer():\n    return int(\"42\")\n")])
    );
    let (provider, server, _) = serve_reply(
        answer.clone(),
        Duration::ZERO,
        "",
        Some(capture),
        true,
        None,
    );
    let (snapshot, detail) = worker::start(
        fixture.store.clone(),
        1,
        "blocks-run".into(),
        fixture.store.snapshot().unwrap().revision,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(
        snapshot.tasks[0].status,
        TaskStatus::ReviewReady,
        "{detail}"
    );
    let request = captured.recv().unwrap();
    assert!(request.get("format").is_none(), "{request}");
    // No schema, but the worker keeps the structured thinking policy and sampling.
    assert_eq!(request["think"], false);
    assert_eq!(request["options"]["temperature"], 0);
    let prompt = last_prompt(&request);
    assert!(prompt.contains("=== FILE: <path> ==="), "{prompt}");
    assert!(!prompt.contains("JSON schema"), "{prompt}");
    // The format is restated after the long source context.
    assert!(
        prompt.trim_end().ends_with(
            "Answer only with FILE blocks, one per changed file, each ending with === END FILE ==="
        ),
        "{prompt}"
    );
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    assert_eq!(
        evidence.generation.as_ref().unwrap().answer_format,
        Some(worker::WorkerFormat::Blocks)
    );
    assert!(evidence.patch.contains("+    return int(\"42\")"));
    let directory = fixture.store.run_directory(&evidence.run).unwrap();
    assert_eq!(
        fs::read_to_string(directory.join("model-response.txt")).unwrap(),
        answer
    );
}

#[tokio::test]
async fn json_worker_format_keeps_the_constrained_schema_request() {
    let fixture = unittest_fixture();
    let (capture, captured) = std::sync::mpsc::channel();
    let (provider, server, _) = server_with_capture(
        one_file("calc.py", "def answer():\n    return 42\n"),
        Duration::ZERO,
        "",
        Some(capture),
    );
    let (snapshot, detail) = worker::start(
        fixture.store.clone(),
        1,
        "json-run".into(),
        fixture.store.snapshot().unwrap().revision,
        provider.with_worker_format(worker::WorkerFormat::Json),
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(
        snapshot.tasks[0].status,
        TaskStatus::ReviewReady,
        "{detail}"
    );
    let request = captured.recv().unwrap();
    assert_eq!(
        request["format"]["properties"]["files"]["items"]["properties"]["path"]["enum"],
        serde_json::json!(["calc.py"])
    );
    assert_eq!(request["think"], false);
    assert!(last_prompt(&request).contains("JSON schema"));
    assert!(!last_prompt(&request).contains("=== FILE:"));
    let evidence: worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    assert_eq!(
        evidence.generation.unwrap().answer_format,
        Some(worker::WorkerFormat::Json)
    );
}

async fn run_reply(fixture: &Fixture, task: u64, answer: String) -> (serde_json::Value, String) {
    let (capture, captured) = std::sync::mpsc::channel();
    let (provider, server, _) = serve_reply(answer, Duration::ZERO, "", Some(capture), true, None);
    let (_, detail) = worker::start(
        fixture.store.clone(),
        task,
        format!("reply-{task}"),
        fixture.store.snapshot().unwrap().revision,
        provider,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    server.join().unwrap();
    (captured.recv().unwrap(), detail)
}

#[tokio::test]
async fn truncated_blocks_fail_named_and_repair_says_so_with_prior_files_as_blocks() {
    let fixture = unittest_fixture();
    let (_, detail) = run_reply(
        &fixture,
        1,
        "=== FILE: calc.py ===\ndef answer():\n    return {todo[".into(),
    )
    .await;
    assert_eq!(
        detail,
        "Model output ended inside FILE block for calc.py (truncated)"
    );
    let second = repair_of(&fixture, 1);
    let (request, detail) = run_reply(
        &fixture,
        second,
        blocks(&[("calc.py", "def answer():\n    return \"41\"\n")]),
    )
    .await;
    let section = failing_section(&last_prompt(&request));
    assert!(
        section.contains("previous response was truncated inside the FILE block for calc.py"),
        "{section}"
    );
    assert!(detail.starts_with("Check failed"), "{detail}");

    // A fresh repair (different model) sees the prior attempt's files as blocks,
    // and the prior patch unescaped rather than inside JSON evidence.
    fixture.action(Action::Repair {
        task: second,
        reason: "Fix the failing test".into(),
    });
    let third = fixture.store.snapshot().unwrap().tasks.len() as u64;
    fixture.action(Action::Assign {
        task: third,
        model: "other".into(),
    });
    fixture.action(Action::Approve { task: third });
    let (request, _) = run_reply(
        &fixture,
        third,
        blocks(&[("calc.py", "def answer():\n    return 42\n")]),
    )
    .await;
    assert_eq!(request["messages"].as_array().unwrap().len(), 1);
    let prompt = last_prompt(&request);
    assert!(prompt.contains("WHAT IS STILL FAILING"), "{prompt}");
    assert!(
        prompt
            .contains("=== FILE: calc.py ===\ndef answer():\n    return \"41\"\n=== END FILE ==="),
        "{prompt}"
    );
    assert!(prompt.contains("+    return \"41\""), "{prompt}");
    assert!(!prompt.contains("return \\\"41\\\""), "{prompt}");
}

#[tokio::test]
async fn owner_instruction_leads_the_repair_request_above_what_is_still_failing() {
    let fixture = unittest_fixture();
    run_captured(&fixture, 1, "def answer():\n    return 41\n", None).await;
    // A queued note carries the check tail after its separator; the request
    // leads with the note alone and the failures follow in their own section.
    fixture.action(Action::Repair {
        task: 1,
        reason: format!(
            "{}answer must be the integer 42{}Check failed (exit 1)",
            alfredo_tui::instruct::OWNER,
            alfredo_tui::instruct::AFTER_CHECK
        ),
    });
    fixture.action(Action::Approve { task: 2 });
    let (request, _) = run_captured(&fixture, 2, "def answer():\n    return 42\n", None).await;
    let prompt = last_prompt(&request);
    let head = &prompt[..prompt.find("WHAT IS STILL FAILING").unwrap()];
    assert_eq!(
        head,
        "OWNER INSTRUCTION (from the repository owner; follow it within the approved files and check below)\nanswer must be the integer 42\n\nImplement this task: Repair #1: Owner: answer must be the integer 42 · after check: Check failed (exit 1)\n"
    );
    assert!(failing_section(&prompt).contains("AssertionError: 41 != 42"));
    // An ordinary repair keeps its first line.
    let fixture = unittest_fixture();
    run_captured(&fixture, 1, "def answer():\n    return 41\n", None).await;
    let second = repair_of(&fixture, 1);
    let (request, _) = run_captured(&fixture, second, "def answer():\n    return 42\n", None).await;
    assert!(last_prompt(&request).starts_with("Implement this task: Repair #1:"));
}

fn policy_of(files: &[&str]) -> WorkPolicy {
    WorkPolicy {
        files: files.iter().map(|f| f.to_string()).collect(),
        check: vec!["/bin/true".into()],
    }
}

#[test]
fn single_fence_is_the_file_when_policy_allows_exactly_one() {
    let one = policy_of(&["calc.py"]);
    for answer in [
        "```\ndef answer():\n    return 42\n```",
        "Here you go:\n```python\ndef answer():\n    return 42\n```\nDone.\n",
        "```python\r\ndef answer():\r\n    return 42\r\n```\r\n",
    ] {
        let plan =
            worker::parse_answer_for(answer, &one).unwrap_or_else(|e| panic!("{answer}: {e}"));
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].path, "calc.py");
        assert_eq!(plan.files[0].content, "def answer():\n    return 42\n");
    }
}

#[test]
fn fence_fallback_refuses_multiple_fences_multiple_files_and_unclosed() {
    let one = policy_of(&["calc.py"]);
    let two_fences = "```\na = 1\n```\ntext\n```\nb = 2\n```";
    assert_eq!(
        worker::parse_answer_for(two_fences, &one).unwrap_err(),
        "Model returned no FILE blocks"
    );
    let two_files = policy_of(&["calc.py", "util.py"]);
    assert_eq!(
        worker::parse_answer_for("```\na = 1\n```", &two_files).unwrap_err(),
        "Model returned no FILE blocks"
    );
    assert_eq!(
        worker::parse_answer_for("```\na = 1\n", &one).unwrap_err(),
        "Model returned no FILE blocks"
    );
    // FILE blocks still win over the fallback.
    let plan =
        worker::parse_answer_for("=== FILE: calc.py ===\nx\n=== END FILE ===\n", &one).unwrap();
    assert_eq!(plan.files[0].content, "x\n");
}

#[tokio::test]
async fn repair_answering_with_one_bare_fence_is_applied() {
    let fixture = unittest_fixture();
    let (_, detail) = run_reply(&fixture, 1, "```\ndef answer():\n    return 41\n```".into()).await;
    assert!(detail.starts_with("Check failed"), "{detail}");
    let second = repair_of(&fixture, 1);
    let (_, detail) = run_reply(
        &fixture,
        second,
        "```python\ndef answer():\n    return 42\n```".into(),
    )
    .await;
    assert!(!detail.contains("no FILE blocks"), "{detail}");
    assert!(!detail.starts_with("Check failed"), "{detail}");
}

#[tokio::test]
async fn test_repair_after_accepted_dependency_gets_its_source_and_authority_line() {
    let fixture = Fixture::new();
    fixture.action(Action::Permit {
        task: 1,
        policy: policy_of(&["calc.py"]),
    });
    fixture.action(Action::Approve { task: 1 });
    run_dependency_fixture(
        &fixture,
        1,
        one_file("calc.py", "def answer():\n    return 42  # DEP_SENTINEL\n"),
        "",
    )
    .await;
    fixture.action(Action::Review {
        task: 1,
        accept: true,
    });
    fixture.action(Action::Propose {
        title: "Write test_calc.py".into(),
        model: "fixture".into(),
        dependencies: vec![1],
    });
    fixture.action(Action::Permit {
        task: 2,
        policy: WorkPolicy {
            files: vec!["test_calc.py".into()],
            check: ["/usr/bin/python3", "-B", "-m", "unittest", "test_calc.py"]
                .map(String::from)
                .to_vec(),
        },
    });
    fixture.action(Action::Approve { task: 2 });
    let wrong = "import unittest\nfrom calc import answer\n\nclass T(unittest.TestCase):\n    def test_answer(self):\n        self.assertEqual(answer(), 41)\n";
    let (request, detail) = run_reply(&fixture, 2, blocks(&[("test_calc.py", wrong)])).await;
    assert!(detail.starts_with("Check failed"), "{detail}");
    let first = last_prompt(&request);
    assert!(!first.contains("ACCEPTED IMPLEMENTATION"), "{first}");
    assert!(!first.contains("DEP_SENTINEL"), "{first}");

    let second = repair_of(&fixture, 2);
    let (request, _) = run_reply(&fixture, second, blocks(&[("test_calc.py", wrong)])).await;
    let prompt = last_prompt(&request);
    assert!(
        prompt.contains("READ-ONLY FILE calc.py\ndef answer():\n    return 42  # DEP_SENTINEL\n"),
        "{prompt}"
    );
    let line = prompt
        .find("ACCEPTED IMPLEMENTATION IS AUTHORITATIVE")
        .unwrap_or_else(|| panic!("{prompt}"));
    assert!(
        prompt[line..].contains("fix the test expectation to match it"),
        "{prompt}"
    );
    assert!(
        prompt[line..].contains("unless the OWNER INSTRUCTION"),
        "{prompt}"
    );
    // Reference only: the allowed list is unchanged.
    assert!(
        prompt.contains("Allowed exact files: [\"test_calc.py\"]"),
        "{prompt}"
    );
}
