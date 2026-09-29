//! Owner instructions from the agent view drive the existing governed
//! transactions beside autopilot; real HTTP fixtures, real workers and checks.
use alfredo_tui::{
    agent_view::Target,
    autopilot::{Autopilot, RunState},
    command_intent::Intent,
    instruct::{self, Instructions},
    provider::Ollama,
    task_control::TaskControl,
    tasks::{Action, Request, Snapshot, TaskStatus, TaskStore, WorkPolicy},
    worker::Progress,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

static ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    store: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-instruct-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        for args in [
            vec!["init", "-q", "--initial-branch=main"],
            vec!["config", "user.name", "Fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
        ] {
            git(&workspace, &args);
        }
        fs::write(workspace.join(".gitattributes"), "* text eol=lf\n").unwrap();
        fs::write(workspace.join("calc.py"), "def answer():\n    return 0\n").unwrap();
        git(&workspace, &["add", "."]);
        git(
            &workspace,
            &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture"],
        );
        let store = TaskStore::new(&root.join("state"), &workspace, "mission").unwrap();
        Self {
            root,
            workspace,
            store,
        }
    }
    fn directory(&self) -> PathBuf {
        self.store.conversation_directory().unwrap()
    }
    fn action(&self, action: Action) -> Snapshot {
        let revision = self.store.snapshot().unwrap().revision;
        self.store
            .transact(Request {
                correlation: format!("fixture-{revision}"),
                expected_revision: revision,
                action,
            })
            .unwrap()
            .0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git(workspace: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// Threaded fixture Ollama: every POST is recorded, then answered by `respond`
/// (which may block on a gate the test controls).
struct Server {
    endpoint: String,
    requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(respond: impl Fn(&Value, usize) -> String + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let respond = Arc::new(respond);
        let (seen, halt) = (requests.clone(), stop.clone());
        let handle = thread::spawn(move || {
            while !halt.load(Ordering::SeqCst) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                let (seen, respond) = (seen.clone(), respond.clone());
                thread::spawn(move || {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut header = Vec::new();
                    let mut byte = [0];
                    while !header.ends_with(b"\r\n\r\n") {
                        if stream.read_exact(&mut byte).is_err() {
                            return;
                        }
                        header.push(byte[0]);
                    }
                    let header = String::from_utf8(header).unwrap();
                    if header.starts_with("GET") {
                        let body = r#"{"models":[{"name":"fixture"}]}"#;
                        let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
                        return;
                    }
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
                    stream.read_exact(&mut body).unwrap();
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    let index = {
                        let mut seen = seen.lock().unwrap();
                        seen.push(request.clone());
                        seen.len() - 1
                    };
                    let content = respond(&request, index);
                    let body = format!("{}\n", json!({"message":{"content":content},"done":true}));
                    let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
                });
            }
        });
        Self {
            endpoint,
            requests,
            stop,
            handle: Some(handle),
        }
    }
    fn provider(&self) -> Ollama {
        Ollama::new(&self.endpoint, Duration::from_secs(5)).unwrap()
    }
    fn prompts(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| {
                request["messages"].as_array().unwrap().last().unwrap()["content"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn planner(request: &Value) -> bool {
    request["messages"][0]["content"]
        .as_str()
        .is_some_and(|text| text.starts_with("Act as Frontier Architect"))
}
fn worker_prompt(request: &Value) -> String {
    request["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string()
}
fn check(expression: &str) -> Value {
    json!(["/usr/bin/python3", "-B", "-c", expression])
}
fn one_task_plan() -> String {
    json!({"tasks": [
        {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
         "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
    ]})
    .to_string()
}
fn files(path: &str, content: &str) -> String {
    json!({"files": [{"path": path, "content": content}]}).to_string()
}
fn good_calc() -> String {
    files("calc.py", "def answer():\n    return 42\n")
}
fn bad_calc() -> String {
    files("calc.py", "def answer():\n    return 41\n")
}
fn owned(prompt: &str) -> bool {
    prompt.starts_with(instruct::HEADER)
}

fn control(fixture: &Fixture, server: &Server, runtime: &Runtime) -> TaskControl {
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(server.provider());
    control.owner = Instructions::open(&fixture.directory(), "default").unwrap();
    control.refresh(runtime);
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending || control.scope_status.revision.is_none() {
        control.poll();
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(2));
    }
    control
}

/// Mirrors the terminal loop: owner steps first, then autopilot, then automatic
/// launches, all through `dispatch_prepared` like saved console intents.
fn drive(
    autopilot: &mut Autopilot,
    control: &mut TaskControl,
    runtime: &Runtime,
    label: &str,
    until: impl Fn(&Autopilot, &TaskControl) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut refreshed = Instant::now();
    loop {
        control.poll();
        control.autopilot_roots = autopilot.roots(control);
        if until(autopilot, control) {
            return;
        }
        if let Some(submission) = Instructions::tick(control, autopilot) {
            assert!(submission.text.starts_with("You · "), "{}", submission.text);
            let _ = control.dispatch_prepared(runtime, &submission.intent);
        } else if let Some(submission) = autopilot.tick(runtime, control) {
            let _ = control.dispatch_prepared(runtime, &submission.intent);
        }
        if let Ok(Some(request)) = control.prepare_dispatch() {
            let _ = control.dispatch_prepared(runtime, &Intent::DispatchRun { request });
        }
        if refreshed.elapsed() > Duration::from_millis(100) {
            control.refresh_background(runtime);
            refreshed = Instant::now();
        }
        assert!(
            Instant::now() < deadline,
            "Timed out: {label}\n{:?}\n{}\n{:?}\n{:?}",
            autopilot.status(control),
            control.notice,
            control.owner.instructions(),
            control.snapshot.as_ref().map(|s| s
                .tasks
                .iter()
                .map(|t| (t.id, t.status.clone(), t.repair_of))
                .collect::<Vec<_>>())
        );
        thread::sleep(Duration::from_millis(3));
    }
}

/// Dispatch one prepared intent and wait for its acknowledgment or refusal.
fn settle(control: &mut TaskControl, runtime: &Runtime, intent: &Intent) {
    control.dispatch_prepared(runtime, intent).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.pending || control.intent_pending(intent) {
        control.poll();
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(2));
    }
}

fn finished(autopilot: &Autopilot, control: &TaskControl) -> bool {
    autopilot
        .status(control)
        .is_some_and(|status| status.state.finished())
        && control.workers.is_empty()
}

fn status_of(control: &TaskControl, id: u64) -> Option<TaskStatus> {
    control
        .snapshot
        .as_ref()?
        .tasks
        .iter()
        .find(|task| task.id == id)
        .map(|task| task.status.clone())
}

fn repair_reason(snapshot: &Snapshot, id: u64) -> String {
    snapshot
        .receipts
        .iter()
        .find_map(|receipt| match &receipt.request.action {
            Action::Repair { reason, .. } if receipt.task == id => Some(reason.clone()),
            Action::ReviewAndRepair { decision, .. } if receipt.task == id => {
                Some(decision.reason.clone())
            }
            _ => None,
        })
        .unwrap_or_default()
}

fn starts(snapshot: &Snapshot, id: u64) -> usize {
    snapshot
        .receipts
        .iter()
        .filter(|r| matches!(r.request.action, Action::Start { task, .. } if task == id))
        .count()
}

#[test]
fn steering_a_generating_worker_cancels_it_and_reruns_with_the_note_outside_the_repair_budget() {
    let fixture = Fixture::new();
    let (release, gate) = mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let server = Server::new(move |request, _| {
        if planner(request) {
            return one_task_plan();
        }
        let prompt = worker_prompt(request);
        if owned(&prompt) {
            return good_calc();
        }
        // The first generation is held until the test ends: only a steer finishes it.
        let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(30));
        bad_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    // No repair budget at all: a steer must not need one.
    autopilot.start("Answer", "fixture", 0, &control).unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "generation",
        |_, c| {
            server.count() == 2
                && c.worker_stage(1).is_some()
                && status_of(c, 1) == Some(TaskStatus::Running)
        },
    );
    let notice =
        Instructions::give_to(&mut control, Target::Task(1), "use the integer 42").unwrap();
    assert_eq!(notice, "Steering #1 · cancelling the generation");
    assert!(control.owner.held().contains(&1));
    drive(&mut autopilot, &mut control, &runtime, "steered", finished);
    drop(release);

    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Done, "{status:?}");
    assert_eq!((status.done, status.total, status.repairs), (1, 1, 0));
    let snapshot = fixture.store.snapshot().unwrap();
    let tasks: Vec<_> = snapshot
        .tasks
        .iter()
        .map(|t| (t.id, t.status.clone(), t.repair_of))
        .collect();
    assert_eq!(
        tasks,
        [
            (1, TaskStatus::Cancelled, None),
            (2, TaskStatus::Accepted, Some(1))
        ]
    );
    assert_eq!(repair_reason(&snapshot, 2), "Owner: use the integer 42");
    // Same inherited policy; the steered run is one more start of the family.
    assert_eq!(snapshot.tasks[0].policy, snapshot.tasks[1].policy);
    assert_eq!(starts(&snapshot, 2), 1);
    // The cancelled request, then a new one led by the owner's instruction.
    let prompts = server.prompts();
    assert_eq!(prompts.len(), 3, "{prompts:?}");
    assert!(prompts[1].starts_with("Implement this task: Make answer return 42"));
    assert!(prompts[2].starts_with(&format!(
        "{}\nuse the integer 42\n\nImplement this task: Repair #1: Owner: use the integer 42\n",
        instruct::HEADER
    )));
    assert!(
        !prompts[2].contains("WHAT IS STILL FAILING"),
        "{}",
        prompts[2]
    );
    let item = &control.owner.instructions()[0];
    assert!(item.done);
    assert_eq!(item.status, "repair #2 started");
    // Recorded in the owner's instruction file beside autopilot state.
    let reopened = Instructions::open(&fixture.directory(), "default").unwrap();
    assert_eq!(reopened.instructions(), control.owner.instructions());
}

/// Autopilot and a failed task: drive until the first attempt failed, before
/// autopilot has decided on that state.
fn failed_first(server: &Server, fixture: &Fixture, runtime: &Runtime) -> (TaskControl, Autopilot) {
    let mut control = control(fixture, server, runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 2, &control).unwrap();
    drive(&mut autopilot, &mut control, runtime, "failure", |_, c| {
        status_of(c, 1) == Some(TaskStatus::Failed) && c.workers.is_empty()
    });
    (control, autopilot)
}

fn failing_then_owner_server() -> Server {
    Server::new(|request, _| {
        if planner(request) {
            return one_task_plan();
        }
        let prompt = worker_prompt(request);
        if owned(&prompt) {
            assert!(prompt.contains("\nuse 42, not 41\n"), "{prompt}");
            assert!(prompt.contains("WHAT IS STILL FAILING"), "{prompt}");
            good_calc()
        } else if prompt.starts_with("Implement this task: Make answer") {
            bad_calc()
        } else {
            panic!("Autopilot's own repair must not run: {prompt}")
        }
    })
}

fn assert_single_owner_repair(fixture: &Fixture, autopilot: &Autopilot, control: &TaskControl) {
    let status = autopilot.status(control).unwrap();
    assert_eq!(status.state, RunState::Done, "{status:?}");
    assert_eq!((status.done, status.total, status.repairs), (1, 1, 1));
    let snapshot = fixture.store.snapshot().unwrap();
    let owner: Vec<_> = snapshot
        .tasks
        .iter()
        .filter(|task| repair_reason(&snapshot, task.id).starts_with("Owner: "))
        .collect();
    assert_eq!(owner.len(), 1);
    assert_eq!(owner[0].status, TaskStatus::Accepted);
    assert_eq!(
        repair_reason(&snapshot, owner[0].id),
        "Owner: use 42, not 41"
    );
    // Any autopilot proposal for the same failure was cancelled before it ran.
    for task in snapshot
        .tasks
        .iter()
        .filter(|t| t.id != 1 && t.id != owner[0].id)
    {
        assert_eq!(task.status, TaskStatus::Cancelled);
        assert!(task.run.is_none());
    }
    let runs: usize = snapshot.tasks.iter().map(|t| starts(&snapshot, t.id)).sum();
    assert_eq!(runs, 2, "the failed run and the owner's repair only");
}

#[test]
fn failed_task_note_withdraws_autopilots_unsent_repair_and_repairs_with_the_note() {
    let fixture = Fixture::new();
    let server = failing_then_owner_server();
    let runtime = Runtime::new().unwrap();
    let (mut control, mut autopilot) = failed_first(&server, &fixture, &runtime);
    let unsent = autopilot.tick(&runtime, &mut control).unwrap();
    assert!(
        unsent.text.contains("/repair 1 autopilot:"),
        "{}",
        unsent.text
    );
    Instructions::give_to(&mut control, Target::Task(1), "use 42, not 41").unwrap();
    // The terminal withdraws an autopilot decision for a family the owner took.
    assert!(control
        .owner
        .supersedes(&unsent.intent, control.snapshot.as_ref()));
    drive(&mut autopilot, &mut control, &runtime, "repair", finished);
    assert_single_owner_repair(&fixture, &autopilot, &control);
}

#[test]
fn same_revision_race_autopilot_repair_first_is_replaced_by_the_owner_note() {
    let fixture = Fixture::new();
    let server = failing_then_owner_server();
    let runtime = Runtime::new().unwrap();
    let (mut control, mut autopilot) = failed_first(&server, &fixture, &runtime);
    let revision = control.snapshot.as_ref().unwrap().revision;
    let automatic = autopilot.tick(&runtime, &mut control).unwrap();
    Instructions::give_to(&mut control, Target::Task(1), "use 42, not 41").unwrap();
    let owner = Instructions::tick(&mut control, &mut autopilot).unwrap();
    assert!(
        owner.text.starts_with("You · /repair 1 Owner: use 42"),
        "{}",
        owner.text
    );
    for intent in [&automatic.intent, &owner.intent] {
        let Intent::Task { request } = intent else {
            panic!("task intent")
        };
        assert_eq!(request.expected_revision, revision);
    }
    settle(&mut control, &runtime, &automatic.intent);
    settle(&mut control, &runtime, &owner.intent);
    assert!(
        control.intent_transient(&owner.intent),
        "stale, nothing written"
    );
    drive(&mut autopilot, &mut control, &runtime, "repair", finished);
    assert_single_owner_repair(&fixture, &autopilot, &control);
}

#[test]
fn same_revision_race_owner_first_leaves_autopilot_nothing_to_decide() {
    let fixture = Fixture::new();
    let server = failing_then_owner_server();
    let runtime = Runtime::new().unwrap();
    let (mut control, mut autopilot) = failed_first(&server, &fixture, &runtime);
    let automatic = autopilot.tick(&runtime, &mut control).unwrap();
    Instructions::give_to(&mut control, Target::Task(1), "use 42, not 41").unwrap();
    let owner = Instructions::tick(&mut control, &mut autopilot).unwrap();
    settle(&mut control, &runtime, &owner.intent);
    settle(&mut control, &runtime, &automatic.intent);
    assert!(control.intent_transient(&automatic.intent));
    // Autopilot prepares again on current state and leaves the held family alone.
    assert!(autopilot
        .tick(&runtime, &mut control)
        .is_none_or(|next| !next.text.contains("/repair")));
    drive(&mut autopilot, &mut control, &runtime, "repair", finished);
    assert_single_owner_repair(&fixture, &autopilot, &control);
    assert_eq!(fixture.store.snapshot().unwrap().tasks.len(), 2);
}

fn review_server() -> Server {
    Server::new(|request, _| {
        if planner(request) {
            return one_task_plan();
        }
        let prompt = worker_prompt(request);
        if owned(&prompt) {
            assert!(prompt.contains("\nkeep a docstring\n"), "{prompt}");
            return files(
                "calc.py",
                "def answer():\n    \"\"\"The answer.\"\"\"\n    return 42\n",
            );
        }
        good_calc()
    })
}

#[test]
fn awaiting_review_note_records_needs_repair_and_autopilot_never_accepts_the_old_result() {
    let fixture = Fixture::new();
    let server = review_server();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 2, &control).unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "review ready",
        |_, c| status_of(c, 1) == Some(TaskStatus::ReviewReady) && c.workers.is_empty(),
    );
    // Autopilot's acceptance and the owner's note are prepared on one revision.
    let accept = autopilot.tick(&runtime, &mut control).unwrap();
    assert!(
        accept.text.contains("/review 1 (approved"),
        "{}",
        accept.text
    );
    Instructions::give_to(&mut control, Target::Task(1), "keep a docstring").unwrap();
    assert!(control
        .owner
        .supersedes(&accept.intent, control.snapshot.as_ref()));
    let owner = Instructions::tick(&mut control, &mut autopilot).unwrap();
    assert_eq!(owner.text, "You · /review 1");
    settle(&mut control, &runtime, &owner.intent);
    settle(&mut control, &runtime, &accept.intent);
    assert!(control.intent_transient(&accept.intent));
    drive(&mut autopilot, &mut control, &runtime, "repair", finished);

    let snapshot = fixture.store.snapshot().unwrap();
    let decision = snapshot.decision_for_task(1).unwrap();
    assert_eq!(
        decision.outcome,
        alfredo_tui::assessment::Outcome::NeedsRepair
    );
    assert_eq!(decision.reason, "Owner: keep a docstring");
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Rejected);
    assert_eq!(
        (
            snapshot.tasks[1].status.clone(),
            snapshot.tasks[1].repair_of
        ),
        (TaskStatus::Accepted, Some(1))
    );
    assert!(git(
        &fixture.workspace,
        &[
            "show",
            &format!(
                "{}:calc.py",
                autopilot.status(&control).unwrap().branch.unwrap()
            )
        ]
    )
    .contains("The answer."));
}

#[test]
fn accepted_task_note_creates_a_follow_up_that_autopilot_reviews_and_integrates() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            return one_task_plan();
        }
        let prompt = worker_prompt(request);
        if prompt.starts_with("Implement this task: also add half()") {
            assert!(
                prompt.contains("return 42"),
                "builds on the accepted result: {prompt}"
            );
            return files(
                "calc.py",
                "def answer():\n    return 42\n\n\ndef half():\n    return 21\n",
            );
        }
        good_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 2, &control).unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "first run",
        finished,
    );
    let first = autopilot.status(&control).unwrap().branch.unwrap();
    let notice = Instructions::give_to(&mut control, Target::Task(1), "also add half()").unwrap();
    assert_eq!(notice, "Follow-up of #1 with the same files and check");
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "follow-up",
        |a, c| finished(a, c) && a.status(c).unwrap().total == 2,
    );

    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Done, "{status:?}");
    assert_eq!((status.done, status.total), (2, 2));
    let snapshot = fixture.store.snapshot().unwrap();
    let follow = &snapshot.tasks[1];
    assert_eq!(follow.title, "also add half()");
    assert_eq!(follow.dependencies, vec![1]);
    assert_eq!(
        follow.policy, snapshot.tasks[0].policy,
        "inherited policy only"
    );
    assert_eq!(follow.model, snapshot.tasks[0].model);
    assert_eq!(follow.status, TaskStatus::Accepted);
    let branch = status.branch.unwrap();
    assert_eq!(branch, format!("{first}-2"));
    let calc = git(&fixture.workspace, &["show", &format!("{branch}:calc.py")]);
    assert!(
        calc.contains("return 42") && calc.contains("def half"),
        "{calc}"
    );
    // The first branch is left exactly where it was.
    assert!(!git(&fixture.workspace, &["show", &format!("{first}:calc.py")]).contains("half"));
}

/// A running task observed in its check (a synthetic live observation).
fn checking_task(fixture: &Fixture) -> (TaskControl, tokio::sync::watch::Sender<Progress>) {
    for action in [
        Action::Propose {
            title: "Make answer return 42".into(),
            model: "fixture".into(),
            dependencies: vec![],
        },
        Action::Permit {
            task: 1,
            policy: WorkPolicy {
                files: vec!["calc.py".into()],
                check: vec!["true".into()],
            },
        },
        Action::Approve { task: 1 },
        Action::Start {
            task: 1,
            baseline: "a".repeat(40),
            inputs: vec![],
        },
    ] {
        fixture.action(action);
    }
    let mut control = TaskControl::new(fixture.store.clone());
    control.owner = Instructions::open(&fixture.directory(), "default").unwrap();
    control.snapshot = Some(fixture.store.snapshot().unwrap());
    let (sender, receiver) = tokio::sync::watch::channel(Progress {
        stage: "Running approved check",
        ..Progress::default()
    });
    control.attach_progress(1, receiver, Arc::new(AtomicBool::new(false)));
    (control, sender)
}

fn finish(fixture: &Fixture, status: TaskStatus, detail: &str) {
    let snapshot = fixture.store.snapshot().unwrap();
    let run = snapshot.tasks[0].run.clone().unwrap();
    let evidence = serde_json::to_vec(&alfredo_tui::worker::Evidence {
        agent: None,
        candidate_commit: None,
        model_metrics: None,
        generation: None,
        run: run.id.clone(),
        baseline: run.baseline.clone(),
        status: status.clone(),
        detail: detail.into(),
        patch: String::new(),
        check: (status == TaskStatus::ReviewReady).then(|| {
            serde_json::from_value(json!({
                "schema_version": 1, "request_id": "check:run", "request_digest": "d", "effect": "local-agent",
                "status": "completed", "started_at": "0", "ended_at": "1", "exit_code": 0,
                "stdout": "", "stderr": "OK", "stdout_bytes": 0, "stderr_bytes": 2,
                "stdout_sha256": "", "stderr_sha256": "", "effect_started": true, "reconciliation_required": false,
                "error_code": "", "error_message": "", "receipt_id": "r", "owner_pid": null, "owner_identity": "",
                "process_pid": null, "process_identity": "", "provider": "fixture"
            }))
            .unwrap()
        }),
    })
    .unwrap();
    let directory = fixture.store.run_directory(&run.id).unwrap();
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("evidence.json"), &evidence).unwrap();
    fixture.action(Action::Finish {
        task: 1,
        run: run.id,
        status,
        evidence_sha256: format!("{:x}", Sha256::digest(&evidence)),
        detail: detail.into(),
    });
}

#[test]
fn note_during_the_check_is_queued_persisted_and_becomes_the_repair_reason_on_failure() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let (mut control, _progress) = checking_task(&fixture);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    let notice = Instructions::give_to(&mut control, Target::Task(1), "return 42").unwrap();
    assert_eq!(notice, "Note queued for #1 · applies if the check fails");
    assert!(Instructions::tick(&mut control, &mut autopilot).is_none());
    // Persisted beside autopilot state; a restart keeps the queued note.
    let reopened = Instructions::open(&fixture.directory(), "default").unwrap();
    assert_eq!(reopened.instructions()[0].note, "return 42");
    assert_eq!(
        reopened.instructions()[0].status,
        "queued · applies if the check fails"
    );
    finish(
        &fixture,
        TaskStatus::Failed,
        "Check failed (exit 1): stderr: AssertionError",
    );
    control.detach_progress(1);
    control.snapshot = Some(fixture.store.snapshot().unwrap());
    let repair = Instructions::tick(&mut control, &mut autopilot).unwrap();
    assert!(
        repair
            .text
            .starts_with("You · /repair 1 Owner: return 42 · after check: Check failed (exit 1)"),
        "{}",
        repair.text
    );
    settle(&mut control, &runtime, &repair.intent);
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks[1].repair_of, Some(1));
    assert_eq!(
        instruct::owner_note(&snapshot, 2).as_deref(),
        Some("return 42")
    );
}

#[test]
fn note_during_the_check_is_dropped_with_a_notice_when_the_check_passes() {
    let fixture = Fixture::new();
    let (mut control, _progress) = checking_task(&fixture);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    Instructions::give_to(&mut control, Target::Task(1), "return 42").unwrap();
    finish(&fixture, TaskStatus::ReviewReady, "Approved check passed");
    control.detach_progress(1);
    control.snapshot = Some(fixture.store.snapshot().unwrap());
    assert!(Instructions::tick(&mut control, &mut autopilot).is_none());
    assert_eq!(
        control.owner.take_notice().as_deref(),
        Some("Note not needed: check passed")
    );
    assert!(control.owner.held().is_empty());
    assert_eq!(fixture.store.snapshot().unwrap().tasks.len(), 1);
    let notes = control.owner.notes(Target::Task(1));
    assert_eq!(notes[0].status, "Note not needed: check passed");
}

#[test]
fn held_for_human_review_refuses_and_names_the_review_command() {
    let fixture = Fixture::new();
    let (mut control, _progress) = checking_task(&fixture);
    finish(&fixture, TaskStatus::ReviewReady, "Approved check passed");
    fixture.action(Action::Decide {
        task: 1,
        decision: alfredo_tui::assessment::Decision {
            failure: None,
            risk: Some(alfredo_tui::assessment::ReviewRisk::Security),
            outcome: alfredo_tui::assessment::Outcome::NeedsHumanReview,
            reason: "Touches credentials".into(),
            criteria: vec![],
            limitations: vec![],
        },
    });
    control.detach_progress(1);
    control.snapshot = Some(fixture.store.snapshot().unwrap());
    let error = Instructions::give_to(&mut control, Target::Task(1), "ship it").unwrap_err();
    assert_eq!(
        error,
        "#1 is held for human review; resolve it with /review 1 JSON first"
    );
    assert!(control.owner.instructions().is_empty());
    // Empty and oversized notes are refused before anything is recorded.
    assert!(Instructions::give_to(&mut control, Target::Task(1), "  ").is_err());
}

#[test]
fn architect_draft_note_revises_the_plan_autopilot_then_saves() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            let prompt = worker_prompt(request);
            if prompt.contains("Revision request: name the task Compute answer") {
                return json!({"tasks": [
                    {"title": "Compute answer", "acceptance": ["answer() returns 42"], "model": "fixture",
                     "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
                ]})
                .to_string();
            }
            return one_task_plan();
        }
        good_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 2, &control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "draft", |_, c| {
        !c.planner.active() && c.planner.checkpoint().is_some()
    });
    Instructions::give_to(
        &mut control,
        Target::Architect,
        "name the task Compute answer",
    )
    .unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "revised run",
        finished,
    );
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.tasks[0].title, "Compute answer");
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Done);
    assert_eq!(
        server
            .prompts()
            .iter()
            .filter(|prompt| prompt.contains("Revision request:"))
            .count(),
        1
    );
}
