use alfredo_tui::{
    planner::{Plan, Step},
    provider::Ollama,
    task_control::TaskControl,
    tasks::{Action, Request, TaskStatus, TaskStore, WorkPolicy},
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: std::path::PathBuf,
    store: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-plan-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("workspace")).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(root.join("workspace"))
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        fs::write(
            root.join("workspace/README.md"),
            "Calculation project fixture: use unit tests.",
        )
        .unwrap();
        for args in [
            vec!["add", "README.md"],
            vec!["-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture"],
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(root.join("workspace"))
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        let store =
            TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
        Self { root, store }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn plan() -> Plan {
    let policy = WorkPolicy {
        files: vec!["calc.py".into()],
        check: vec!["python3".into(), "-m".into(), "unittest".into()],
    };
    Plan {
        architecture: None,
        prompt: "Implement the requested calculation and integration".into(),
        planner: "fixture".into(),
        context: None,
        scope: None,
        tasks: vec![
            Step {
                acceptance: vec![
                    "Acceptance contract sentinel: preserve expected calculation behavior".into(),
                ],
                title: "Implement calculation and pass its unit tests".into(),
                model: "fixture".into(),
                dependencies: vec![],
                policy: policy.clone(),
            },
            Step {
                acceptance: vec![
                    "Acceptance contract sentinel: preserve expected calculation behavior".into(),
                ],
                title: "Integrate calculation and pass integration tests".into(),
                model: "fixture".into(),
                dependencies: vec![1],
                policy,
            },
        ],
    }
}
#[test]
fn plan_is_atomic_proposed_replayable_and_dependency_ids_are_rebased() {
    let fixture = Fixture::new();
    fixture
        .store
        .transact(Request {
            correlation: "existing".into(),
            expected_revision: 0,
            action: Action::Propose {
                title: "existing".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        })
        .unwrap();
    let request = Request {
        correlation: "plan".into(),
        expected_revision: 1,
        action: Action::Plan { plan: plan() },
    };
    let (snapshot, receipt) = fixture.store.transact(request.clone()).unwrap();
    assert_eq!(snapshot.revision, 2);
    assert_eq!(receipt.task, 2);
    assert_eq!(snapshot.tasks.len(), 3);
    assert_eq!(snapshot.tasks[2].dependencies, vec![2]);
    assert!(snapshot
        .tasks
        .iter()
        .all(|task| task.status == TaskStatus::Proposed && task.run.is_none()));
    assert_eq!(
        snapshot.tasks[1].policy,
        Some(plan().tasks[0].policy.clone())
    );
    assert_eq!(fixture.store.transact(request.clone()).unwrap().1, receipt);
    assert_eq!(fixture.store.snapshot().unwrap().tasks, snapshot.tasks);
    assert_eq!(
        alfredo_tui::activity::entries(&snapshot, "#3")[0].summary,
        "Plan proposed · 2 tasks"
    );
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let bytes = fs::read(&path).unwrap();
    let mut bad = plan();
    bad.tasks[1].dependencies = vec![2];
    assert!(fixture
        .store
        .transact(Request {
            correlation: "bad".into(),
            expected_revision: 2,
            action: Action::Plan { plan: bad }
        })
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let mut changed = request;
    if let Action::Plan { plan } = &mut changed.action {
        plan.tasks[0].model = "other".into();
    }
    assert!(fixture
        .store
        .transact(changed)
        .unwrap_err()
        .contains("Correlation"));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(fixture
        .store
        .transact(Request {
            correlation: "run".into(),
            expected_revision: 2,
            action: Action::Start {
                task: 2,
                baseline: "0".repeat(40),
                inputs: vec![]
            }
        })
        .is_err());
}
fn server(content: String, done: bool, delay: Duration) -> (Ollama, thread::JoinHandle<()>) {
    let (provider, handle, _) = server_capture(content, done, delay);
    (provider, handle)
}
fn server_capture(
    content: String,
    done: bool,
    delay: Duration,
) -> (
    Ollama,
    thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<serde_json::Value>,
) {
    let (sent, received) = std::sync::mpsc::channel();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let provider = Ollama::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        Duration::from_secs(3),
    )
    .unwrap();
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
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
        assert_eq!(request["format"]["properties"]["tasks"]["maxItems"], 16);
        assert_eq!(
            request["format"]["properties"]["tasks"]["items"]["properties"]["acceptance"]
                ["minItems"],
            1
        );
        assert_eq!(request["think"], false);
        assert_eq!(request["model"], "fixture");
        assert!(request["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("Calculation project fixture"));
        let scope_payload = request["messages"][2]["content"].as_str().unwrap();
        assert!(scope_payload.contains("Project scope reference"));
        let scope: alfredo_tui::understanding::Binding =
            serde_json::from_str(scope_payload.split_once('\n').unwrap().1).unwrap();
        scope.validate().unwrap();
        if scope.confirmed {
            assert_eq!(scope.brief.unwrap().destination, "Scope fixture result");
        }
        let _ = sent.send(request);
        thread::sleep(delay);
        let body = format!(
            "{}\n",
            serde_json::json!({"message":{"content":content},"done":done})
        );
        let _ = socket.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        );
    });
    (provider, handle, received)
}
fn wait(control: &mut TaskControl, predicate: impl Fn(&TaskControl) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate(control) {
        control.poll();
        assert!(
            Instant::now() < deadline,
            "{} / {}",
            control.notice,
            control.planner.notice
        );
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn generated_plan_requires_explicit_save_and_approval_and_renders_narrow_preview() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    confirm_scope(&fixture.store);
    let (provider, server) = server(
        serde_json::json!({"tasks":plan().tasks}).to_string(),
        true,
        Duration::ZERO,
    );
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(provider);
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    control
        .command(&runtime, "/plan Implement calculation", "fixture")
        .unwrap();
    wait(&mut control, |c| c.planner.draft.is_some());
    assert_eq!(
        control.planner.draft.as_ref().unwrap().scope,
        Some(Box::new(
            fixture.store.understanding().snapshot().unwrap().binding()
        ))
    );
    assert!(fixture.store.snapshot().unwrap().tasks.is_empty());
    control.command(&runtime, "/tasks", "fixture").unwrap();
    wait(&mut control, |c| !c.pending);
    assert!(!control.planner.visible);
    control.command(&runtime, "/plan", "fixture").unwrap();
    assert!(control.planner.visible);
    let app = alfredo_tui::model::App::new("fixture".into());
    for (width, height) in [(100, 30), (32, 10)] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| alfredo_tui::ui::draw_with_tasks(frame, &app, &control))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Plan draft"), "{text}");
    }
    control.command(&runtime, "/plan-save", "fixture").unwrap();
    wait(&mut control, |c| !c.pending);
    assert!(
        control.notice.contains("2 proposed tasks"),
        "{}",
        control.notice
    );
    assert!(control.planner.draft.is_none());
    assert!(control.command(&runtime, "/plan-save", "fixture").is_err());
    assert_eq!(fixture.store.snapshot().unwrap().tasks.len(), 2);
    server.join().unwrap();
}
#[test]
fn planner_instruction_runs_existing_tests_and_keeps_them_out_of_written_files() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    confirm_scope(&fixture.store);
    let (provider, server, requests) = server_capture(
        serde_json::json!({"tasks":plan().tasks}).to_string(),
        true,
        Duration::ZERO,
    );
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(provider);
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    control
        .command(&runtime, "/plan Make test_calc.py pass", "fixture")
        .unwrap();
    wait(&mut control, |c| c.planner.draft.is_some());
    server.join().unwrap();
    let request = requests.recv().unwrap();
    let system = request["messages"][0]["content"].as_str().unwrap();
    assert!(system.starts_with("Act as Frontier Architect"), "{system}");
    for text in [
        "When the goal references existing test files, set the check to run those tests",
        "Do not list existing test files in policy files unless the goal asks to change them",
        "workers receive them as read-only reference",
        "Each task's check must run using only files that already exist, files that task writes, or files written by the tasks it depends on",
        "the implementation task's check must be a direct smoke check of its own file (for example [\"python3\", \"-c\", \"import textutil\"])",
        "or put the implementation and its tests in one task",
        "Every task writes at least one policy file",
        "Tasks that write the same file must be ordered by a dependency",
        "The check program must be a bare program name on /usr/bin:/bin or an absolute path",
    ] {
        assert!(system.contains(text), "{text} missing: {system}");
    }
    assert_eq!(
        request["format"]["properties"]["tasks"]["items"]["required"],
        serde_json::json!(["title", "acceptance", "model", "dependencies", "policy"])
    );
    assert_eq!(
        request["format"]["properties"]["tasks"]["items"]["properties"]["policy"]["required"],
        serde_json::json!(["files", "check"])
    );
}

#[test]
fn malformed_or_incomplete_model_output_cannot_be_saved_as_a_plan() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for (content, done) in [
        ("I completed and approved everything".into(), true),
        (
            {
                let mut legacy = plan();
                for step in &mut legacy.tasks {
                    step.acceptance.clear();
                }
                serde_json::json!({"tasks":legacy.tasks}).to_string()
            },
            true,
        ),
        (serde_json::json!({"tasks":plan().tasks}).to_string(), false),
    ] {
        let fixture = Fixture::new();
        let (provider, server) = server(content, done, Duration::ZERO);
        let mut control = TaskControl::new(fixture.store.clone());
        control.set_provider(provider);
        control.refresh(&runtime);
        wait(&mut control, |c| !c.pending);
        control
            .command(&runtime, "/plan Implement calculation", "fixture")
            .unwrap();
        wait(&mut control, |c| {
            c.planner.notice.contains("no action taken")
        });
        assert!(control.command(&runtime, "/plan-save", "fixture").is_err());
        assert!(fixture.store.snapshot().unwrap().tasks.is_empty());
        server.join().unwrap();
    }
}

#[test]
fn stale_plan_save_retains_draft_and_never_partially_proposes_tasks() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    let mut control = TaskControl::new(fixture.store.clone());
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    control.planner.draft = Some(plan());
    control.planner.revision = 0;
    fixture
        .store
        .transact(Request {
            correlation: "concurrent".into(),
            expected_revision: 0,
            action: Action::Propose {
                title: "Other task".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        })
        .unwrap();
    control.command(&runtime, "/plan-save", "fixture").unwrap();
    wait(&mut control, |c| !c.pending);
    assert!(control.notice.contains("changed"));
    assert!(control.planner.draft.is_some());
    assert_eq!(fixture.store.snapshot().unwrap().tasks.len(), 1);
}

#[test]
fn legacy_schema_cannot_carry_plan_authority() {
    let fixture = Fixture::new();
    fixture
        .store
        .transact(Request {
            correlation: "plan".into(),
            expected_revision: 0,
            action: Action::Plan { plan: plan() },
        })
        .unwrap();
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["schema_version"] = 5.into();
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(fixture.store.snapshot().unwrap_err().contains("schema v6"));
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn cancelling_an_unobserved_model_completion_never_publishes_a_late_draft() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    let (provider, server) = server(
        serde_json::json!({"tasks":plan().tasks}).to_string(),
        true,
        Duration::ZERO,
    );
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(provider);
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    control
        .command(&runtime, "/plan Implement calculation", "fixture")
        .unwrap();
    server.join().unwrap();
    control
        .command(&runtime, "/plan-cancel", "fixture")
        .unwrap();
    for _ in 0..10 {
        control.poll();
        thread::sleep(Duration::from_millis(2));
    }
    assert!(control.planner.draft.is_none());
    assert!(control.command(&runtime, "/plan-save", "fixture").is_err());
    assert!(fixture.store.snapshot().unwrap().tasks.is_empty());
}

#[tokio::test]
async fn committed_context_excludes_working_edits_symlinks_secrets_and_oversized_blobs() {
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    fs::write(workspace.join(".env"), "TRACKED_SECRET").unwrap();
    fs::write(
        workspace.join("AGENTS.md"),
        "Follow project tests and exact policy.",
    )
    .unwrap();
    fs::write(workspace.join("large.txt"), "x".repeat(9000)).unwrap();
    fs::write(workspace.join("binary.bin"), [0, 255, 1]).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", workspace.join("outside-link")).unwrap();
    for index in 0..270 {
        fs::write(workspace.join(format!("file-{index:03}.txt")), "source").unwrap();
    }
    for args in [
        vec!["add", "."],
        vec![
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "context fixture",
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
    let before = fs::read(
        fixture
            .store
            .conversation_directory()
            .unwrap()
            .join("tasks.json"),
    )
    .ok();
    fs::write(workspace.join("README.md"), "UNCOMMITTED_INSTRUCTIONS").unwrap();
    fs::write(workspace.join("untracked.txt"), "UNTRACKED_SECRET").unwrap();
    let context = alfredo_tui::planning_context::capture(
        &workspace,
        &format!(
            "calculation readme large binary {}",
            "file-000 ".repeat(100)
        ),
    )
    .await
    .unwrap();
    context.validate().unwrap();
    assert_eq!(context.paths.len(), 256);
    assert!(context.omitted_paths > 0);
    assert_eq!(context.sources.len(), 8);
    assert_eq!(context.sources[0].path, "AGENTS.md");
    assert!(context.omitted_sources > 0);
    assert!(context
        .sources
        .iter()
        .any(|source| source.path == "README.md"
            && source.content.contains("Calculation project fixture")));
    let json = serde_json::to_string(&context).unwrap();
    for forbidden in [
        "TRACKED_SECRET",
        "UNTRACKED_SECRET",
        "UNCOMMITTED_INSTRUCTIONS",
        "outside-link",
        "root:x:",
    ] {
        assert!(!json.contains(forbidden));
    }
    assert!(!context
        .sources
        .iter()
        .any(|source| matches!(source.path.as_str(), "large.txt" | "binary.bin")));
    assert_eq!(
        fs::read_to_string(workspace.join("README.md")).unwrap(),
        "UNCOMMITTED_INSTRUCTIONS"
    );
    assert_eq!(
        fs::read(
            fixture
                .store
                .conversation_directory()
                .unwrap()
                .join("tasks.json")
        )
        .ok(),
        before
    );
}

#[tokio::test]
async fn grounded_plan_retains_context_and_refuses_a_changed_execution_baseline() {
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    let mut draft = plan();
    draft.context = Some(
        alfredo_tui::planning_context::capture(&workspace, &draft.prompt)
            .await
            .unwrap(),
    );
    let request = Request {
        correlation: "grounded".into(),
        expected_revision: 0,
        action: Action::Plan {
            plan: draft.clone(),
        },
    };
    fixture.store.transact(request).unwrap();
    assert_eq!(
        fixture
            .store
            .snapshot()
            .unwrap()
            .plan_for_task(2)
            .unwrap()
            .context,
        draft.context
    );
    fixture
        .store
        .transact(Request {
            correlation: "approve".into(),
            expected_revision: 1,
            action: Action::Approve { task: 1 },
        })
        .unwrap();
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(&workspace)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--allow-empty",
            "-qm",
            "new baseline"
        ])
        .status()
        .unwrap()
        .success());
    let error = alfredo_tui::worker::start(
        fixture.store.clone(),
        1,
        "start".into(),
        2,
        Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await
    .unwrap_err();
    assert!(error.contains("Plan baseline changed"), "{error}");
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.revision, 2);
    assert!(snapshot.tasks[0].run.is_none());
    let path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    old["schema_version"] = 7.into();
    let bytes = serde_json::to_vec(&old).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(fixture.store.snapshot().unwrap_err().contains("schema v8"));
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn missing_repository_context_refuses_inference_and_plan_save() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    fs::remove_dir_all(fixture.root.join("workspace/.git")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Ollama::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        Duration::from_secs(1),
    )
    .unwrap();
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(provider);
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    control
        .command(&runtime, "/plan Implement calculation", "fixture")
        .unwrap();
    wait(&mut control, |c| {
        c.planner.notice.contains("no action taken")
    });
    assert!(control.command(&runtime, "/plan-save", "fixture").is_err());
    assert!(fixture.store.snapshot().unwrap().tasks.is_empty());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

fn confirm_scope(store: &TaskStore) {
    use alfredo_tui::understanding::{Action as ScopeAction, Brief, Request as ScopeRequest};
    let scope = store.understanding();
    let revision = scope.snapshot().unwrap().revision;
    scope
        .transact(ScopeRequest {
            correlation: format!("scope-{revision}"),
            expected_revision: revision,
            action: ScopeAction::Draft {
                brief: Brief {
                    destination: "Scope fixture result".into(),
                    scope: "Calculation only".into(),
                    constraints: "Keep compatibility".into(),
                    uncertainty: "Measure speed".into(),
                },
            },
        })
        .unwrap();
    scope
        .transact(ScopeRequest {
            correlation: format!("confirm-{revision}"),
            expected_revision: revision + 1,
            action: ScopeAction::Confirm {
                draft_revision: revision + 1,
            },
        })
        .unwrap();
}
#[test]
fn plan_scope_changes_reject_publication_and_worker_claims_without_rewriting_history() {
    let fixture = Fixture::new();
    confirm_scope(&fixture.store);
    let mut draft = plan();
    draft.scope = Some(Box::new(
        fixture.store.understanding().snapshot().unwrap().binding(),
    ));
    let request = Request {
        correlation: "scoped-plan".into(),
        expected_revision: 0,
        action: Action::Plan {
            plan: draft.clone(),
        },
    };
    let (saved, _) = fixture.store.transact(request.clone()).unwrap();
    assert_eq!(saved.schema_version, 17);
    assert_eq!(
        fixture
            .store
            .snapshot()
            .unwrap()
            .plan_for_task(1)
            .unwrap()
            .scope,
        draft.scope
    );
    fixture
        .store
        .transact(Request {
            correlation: "approve-scoped".into(),
            expected_revision: 1,
            action: Action::Approve { task: 1 },
        })
        .unwrap();
    confirm_scope(&fixture.store); // Even identical text has a new explicit agreement revision.
    assert!(fixture.store.transact(request).is_ok()); // Exact acknowledged replay has no effect.
    let revision = fixture.store.snapshot().unwrap().revision;
    assert!(fixture
        .store
        .transact(Request {
            correlation: "stale-plan".into(),
            expected_revision: revision,
            action: Action::Plan { plan: draft }
        })
        .unwrap_err()
        .contains("Plan scope changed"));
    assert!(fixture
        .store
        .claim_worker(1)
        .err()
        .unwrap()
        .to_string()
        .contains("Plan scope changed"));
    assert!(fixture
        .store
        .transact(Request {
            correlation: "stale-start".into(),
            expected_revision: revision,
            action: Action::Start {
                task: 1,
                baseline: "a".repeat(40),
                inputs: vec![]
            }
        })
        .unwrap_err()
        .contains("Plan scope changed"));
    assert_eq!(fixture.store.snapshot().unwrap().revision, revision);
    let mut fresh = plan();
    fresh.scope = Some(Box::new(
        fixture.store.understanding().snapshot().unwrap().binding(),
    ));
    fixture
        .store
        .transact(Request {
            correlation: "fresh-plan".into(),
            expected_revision: revision,
            action: Action::Plan { plan: fresh },
        })
        .unwrap();
    let namespace = fs::read_dir(fixture.root.join("state/rust-tasks-v1"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let path = namespace.join("tasks.json");
    let bytes = fs::read(&path).unwrap();
    let mut downgraded: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    downgraded["schema_version"] = 8.into();
    let invalid = serde_json::to_vec(&downgraded).unwrap();
    fs::write(&path, &invalid).unwrap();
    assert!(fixture
        .store
        .snapshot()
        .unwrap_err()
        .contains("requires schema v9"));
    assert_eq!(fs::read(&path).unwrap(), invalid);
    fs::write(&path, bytes).unwrap();
    let mut malformed = fixture.store.understanding().snapshot().unwrap().binding();
    malformed.draft_revision = u64::MAX;
    assert!(malformed.validate().is_err());
}

#[test]
fn stale_plan_is_visible_without_consuming_attempt_and_dispatch_skips_to_fresh_plan() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    // Also cover plans saved before the explicit scope flow existed.
    fixture
        .store
        .transact(Request {
            correlation: "legacy-plan".into(),
            expected_revision: 0,
            action: Action::Plan { plan: plan() },
        })
        .unwrap();
    fixture
        .store
        .transact(Request {
            correlation: "approve-old".into(),
            expected_revision: 1,
            action: Action::Approve { task: 1 },
        })
        .unwrap();
    let mut control = TaskControl::new(fixture.store.clone());
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    let snapshot = control.snapshot.as_ref().unwrap();
    assert!(control
        .scope_status
        .run_blocker(snapshot, &snapshot.tasks[0])
        .is_none());
    confirm_scope(&fixture.store);
    let mut fresh = plan();
    fresh.scope = Some(Box::new(
        fixture.store.understanding().snapshot().unwrap().binding(),
    ));
    fixture
        .store
        .transact(Request {
            correlation: "fresh-plan".into(),
            expected_revision: 2,
            action: Action::Plan { plan: fresh },
        })
        .unwrap();
    fixture
        .store
        .transact(Request {
            correlation: "approve-fresh".into(),
            expected_revision: 3,
            action: Action::Approve { task: 3 },
        })
        .unwrap();
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    let before = fixture.store.snapshot().unwrap();
    assert!(control
        .command(&runtime, "/run 1", "fixture")
        .unwrap_err()
        .contains("Plan scope changed"));
    assert!(control.dispatch.attempts.is_empty());
    assert!(control.dispatch.failures.is_empty());
    control.dispatch.enabled = true;
    let next = control
        .dispatch
        .next_matching(&before, &Default::default(), |task| {
            control.scope_status.run_blocker(&before, task).is_none()
        });
    assert_eq!(next, Some(3));
    control
        .command(&runtime, "/tasks plan scope changed", "fixture")
        .unwrap();
    wait(&mut control, |c| !c.pending);
    assert_eq!(
        control
            .visible_tasks()
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let app = alfredo_tui::model::App::new("fixture".into());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw_with_tasks(frame, &app, &control))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Plan scope changed"), "{text}");
    assert_eq!(
        serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

#[test]
fn revision_uses_previous_tasks_and_preserves_explicit_save_and_approval() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    let original = plan();
    let mut replacement = original.tasks.clone();
    replacement[1].title = "Add boundary coverage".into();
    replacement[1].policy.files.push("test_calc.py".into());
    let (provider, server, received) = server_capture(
        serde_json::json!({"tasks":replacement}).to_string(),
        true,
        Duration::from_millis(30),
    );
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(provider);
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    control.planner.draft = Some(original.clone());
    control
        .command(
            &runtime,
            "/plan-revise Add boundary coverage",
            "different-chat-model",
        )
        .unwrap();
    assert!(control.command(&runtime, "/plan-save", "fixture").is_err());
    assert!(control
        .command(&runtime, "/plan-revise More edits", "fixture")
        .is_err());
    let stable = control.planner.checkpoint().unwrap();
    assert_eq!(stable.plan, original);
    assert_eq!(stable.revision, 0);
    wait(&mut control, |c| !c.planner.active());
    let request = received.recv_timeout(Duration::from_secs(1)).unwrap();
    let messages = request["messages"].as_array().unwrap();
    let previous = messages
        .iter()
        .find_map(|m| {
            m["content"]
                .as_str()
                .filter(|s| s.starts_with("Previous unsaved task draft"))
        })
        .unwrap();
    let sent: Vec<Step> = serde_json::from_str(previous.split_once('\n').unwrap().1).unwrap();
    assert_eq!(sent, original.tasks);
    assert!(messages.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .contains(&original.prompt));
    assert!(messages.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .ends_with("Revision request: Add boundary coverage"));
    assert_eq!(control.planner.draft.as_ref().unwrap().tasks, replacement);
    assert!(fixture.store.snapshot().unwrap().tasks.is_empty());
    control.command(&runtime, "/plan-save", "fixture").unwrap();
    wait(&mut control, |c| !c.pending);
    let state = fixture.store.snapshot().unwrap();
    assert_eq!(state.tasks.len(), 2);
    assert!(state
        .tasks
        .iter()
        .all(|task| task.status == TaskStatus::Proposed && task.run.is_none()));
    assert_eq!(state.tasks[1].title, "Add boundary coverage");
    server.join().unwrap();
}

#[test]
fn failed_revision_restores_prior_draft_and_original_stale_revision() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for done in [true, false] {
        let fixture = Fixture::new();
        let (provider, server) = server("incomplete replacement".into(), done, Duration::ZERO);
        let mut control = TaskControl::new(fixture.store.clone());
        control.set_provider(provider);
        fixture
            .store
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
        control.refresh(&runtime);
        wait(&mut control, |c| !c.pending);
        control.planner.draft = Some(plan());
        control.planner.revision = 0;
        control
            .command(&runtime, "/plan-revise Add tests", "fixture")
            .unwrap();
        wait(&mut control, |c| !c.planner.active());
        assert_eq!(control.planner.draft, Some(plan()));
        assert_eq!(control.planner.revision, 0);
        assert!(control.planner.notice.contains("previous draft retained"));
        control.command(&runtime, "/plan-save", "fixture").unwrap();
        wait(&mut control, |c| !c.pending);
        assert!(control.notice.contains("changed"));
        assert_eq!(fixture.store.snapshot().unwrap().tasks.len(), 1);
        server.join().unwrap();
    }
}

#[test]
fn cancelling_revision_discards_both_drafts_and_late_response() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    let (provider, server, received) = server_capture(
        serde_json::json!({"tasks":plan().tasks}).to_string(),
        true,
        Duration::from_millis(40),
    );
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(provider);
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    control.planner.draft = Some(plan());
    control
        .command(&runtime, "/plan-revise Add tests", "fixture")
        .unwrap();
    received.recv_timeout(Duration::from_secs(2)).unwrap();
    control
        .command(&runtime, "/plan-cancel", "fixture")
        .unwrap();
    server.join().unwrap();
    control.poll();
    assert!(!control.planner.active());
    assert!(control.planner.draft.is_none());
    assert!(control.command(&runtime, "/plan-save", "fixture").is_err());
    assert!(fixture.store.snapshot().unwrap().tasks.is_empty());
}

#[test]
fn invalid_revision_preserves_draft_without_starting_inference() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap());
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    assert!(control
        .command(&runtime, "/plan-revise Add tests", "fixture")
        .is_err());
    control.planner.draft = Some(plan());
    for request in [
        "/plan-revise".to_string(),
        format!("/plan-revise {}", "x".repeat(8192)),
    ] {
        assert!(control.command(&runtime, &request, "fixture").is_err());
        assert_eq!(control.planner.draft, Some(plan()));
        assert!(!control.planner.active());
    }
}

#[test]
fn acceptance_contract_is_bounded_and_legacy_plans_gain_no_inferred_criteria() {
    for criteria in [
        vec![" ".into()],
        vec!["same".into(), " same ".into()],
        vec!["x".repeat(1025)],
        vec!["界".repeat(400)],
        vec!["line\nbreak".into()],
        (0..17).map(|i| format!("criterion {i}")).collect(),
    ] {
        let mut invalid = plan();
        invalid.tasks[0].acceptance = criteria;
        assert!(invalid.validate().is_err());
    }
    let mut legacy = serde_json::to_value(plan()).unwrap();
    for step in legacy["tasks"].as_array_mut().unwrap() {
        step.as_object_mut().unwrap().remove("acceptance");
    }
    let restored: Plan = serde_json::from_value(legacy).unwrap();
    restored.validate().unwrap();
    assert!(restored.tasks.iter().all(|s| s.acceptance.is_empty()));
}

#[test]
fn v9_plan_upgrade_preserves_contract_absence_and_v10_rejects_downgrade() {
    let fixture = Fixture::new();
    let mut legacy = plan();
    for step in &mut legacy.tasks {
        step.acceptance.clear();
    }
    fixture
        .store
        .transact(Request {
            correlation: "old-plan".into(),
            expected_revision: 0,
            action: Action::Plan { plan: legacy },
        })
        .unwrap();
    let directory = fixture.store.conversation_directory().unwrap();
    let path = directory.join("tasks.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["schema_version"] = 9.into();
    let original = serde_json::to_vec_pretty(&value).unwrap();
    fs::write(&path, &original).unwrap();
    assert!(fixture
        .store
        .snapshot()
        .unwrap()
        .acceptance_for_task(1)
        .is_empty());
    fixture
        .store
        .transact(Request {
            correlation: "new-plan".into(),
            expected_revision: 1,
            action: Action::Plan { plan: plan() },
        })
        .unwrap();
    assert_eq!(
        fs::read(directory.join("tasks-v9-backup.json")).unwrap(),
        original
    );
    let snapshot = fixture.store.snapshot().unwrap();
    assert!(snapshot.acceptance_for_task(1).is_empty());
    assert_eq!(snapshot.acceptance_for_task(3), plan().tasks[0].acceptance);
    assert_eq!(snapshot.tasks[2].status, TaskStatus::Proposed);
    let mut downgraded = serde_json::to_value(snapshot).unwrap();
    downgraded["schema_version"] = 9.into();
    let bytes = serde_json::to_vec(&downgraded).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(fixture.store.snapshot().unwrap_err().contains("schema v10"));
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[tokio::test]
async fn pinned_context_uses_exact_commit_instead_of_head_or_working_bytes() {
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace").canonicalize().unwrap();
    let original = alfredo_tui::planning_context::capture(&workspace, "calculation")
        .await
        .unwrap();
    fs::write(workspace.join("README.md"), "NEW_HEAD_SENTINEL").unwrap();
    for args in [
        vec!["add", "README.md"],
        vec![
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "new baseline",
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
    fs::write(workspace.join("README.md"), "DIRTY_WORKTREE_SENTINEL").unwrap();
    let pinned =
        alfredo_tui::planning_context::capture_at(&workspace, "calculation", &original.baseline)
            .await
            .unwrap();
    assert_eq!(pinned, original);
    let current = alfredo_tui::planning_context::capture(&workspace, "calculation")
        .await
        .unwrap();
    assert_ne!(current.baseline, original.baseline);
    assert!(current
        .sources
        .iter()
        .any(|source| source.content == "NEW_HEAD_SENTINEL"));
    assert!(!serde_json::to_string(&current)
        .unwrap()
        .contains("DIRTY_WORKTREE_SENTINEL"));
    for invalid in [
        "HEAD".to_string(),
        format!("{}^{{commit}}", original.baseline),
        original.sources[0].blob.clone(),
        "0".repeat(40),
    ] {
        assert!(
            alfredo_tui::planning_context::capture_at(&workspace, "calculation", &invalid)
                .await
                .is_err(),
            "Accepted invalid pinned baseline {invalid}"
        );
    }
    assert_eq!(
        fs::read_to_string(workspace.join("README.md")).unwrap(),
        "DIRTY_WORKTREE_SENTINEL"
    );
}

#[test]
fn activity_refresh_after_failed_plan_save_preserves_complete_unsaved_checkpoint() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    fixture
        .store
        .transact(Request {
            correlation: "other-user-change".into(),
            expected_revision: 0,
            action: Action::Propose {
                title: "Other task".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        })
        .unwrap();
    let mut control = TaskControl::new(fixture.store.clone());
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    control
        .planner
        .restore(alfredo_tui::planner::SavedDraft {
            origin: None,
            plan: plan(),
            revision: 0,
        })
        .unwrap();
    let checkpoint = control.planner.checkpoint().unwrap();
    let state_path = fixture
        .store
        .conversation_directory()
        .unwrap()
        .join("tasks.json");
    let original = fs::read(&state_path).unwrap();
    control.command(&runtime, "/plan-save", "fixture").unwrap();
    wait(&mut control, |c| !c.pending);
    assert!(control.notice.contains("changed"), "{}", control.notice);
    assert_eq!(control.planner.checkpoint(), Some(checkpoint.clone()));
    // The failed save retains its exact request until the user reloads state.
    control.command(&runtime, "/retry-task", "fixture").unwrap();
    wait(&mut control, |c| !c.pending);
    assert!(control.notice.contains("changed"), "{}", control.notice);
    assert_eq!(control.planner.checkpoint(), Some(checkpoint.clone()));
    control.command(&runtime, "/activity", "fixture").unwrap();
    wait(&mut control, |c| !c.pending);
    assert!(control.activity.is_some());
    assert_eq!(control.planner.checkpoint(), Some(checkpoint.clone()));
    assert_eq!(fs::read(&state_path).unwrap(), original);
    control.command(&runtime, "/plan", "fixture").unwrap();
    assert!(control.planner.visible);
    assert_eq!(control.planner.checkpoint(), Some(checkpoint));
}

#[test]
fn empty_policy_files_name_the_task() {
    let mut plan = plan();
    plan.tasks[1].policy.files.clear();
    assert_eq!(
        plan.validate().unwrap_err(),
        "Task 2 lists no policy files; every task must write at least one file."
    );
}

#[test]
fn manual_plan_shows_validation_warnings_without_blocking_save() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = Fixture::new();
    confirm_scope(&fixture.store);
    let mut tasks = plan().tasks;
    tasks[0].policy = WorkPolicy {
        files: vec!["textutil.py".into()],
        check: vec![
            "python3".into(),
            "-m".into(),
            "unittest".into(),
            "test_textutil.py".into(),
        ],
    };
    let (provider, server) = server(
        serde_json::json!({ "tasks": tasks }).to_string(),
        true,
        Duration::ZERO,
    );
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(provider);
    control.refresh(&runtime);
    wait(&mut control, |c| !c.pending);
    control
        .command(&runtime, "/plan Implement calculation", "fixture")
        .unwrap();
    wait(&mut control, |c| c.planner.draft.is_some());
    server.join().unwrap();
    let preview = control.planner.preview();
    assert!(
        preview.contains("Validation warnings · /plan-save still allowed"),
        "{preview}"
    );
    assert!(
        preview.contains("Task 1 check references test_textutil.py, which does not exist yet"),
        "{preview}"
    );
    control.command(&runtime, "/plan-save", "fixture").unwrap();
    wait(&mut control, |c| !c.pending);
    assert_eq!(fixture.store.snapshot().unwrap().tasks.len(), 2);
}
