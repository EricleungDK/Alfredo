//! Autopilot drives the existing governed transactions; real HTTP fixtures and workers.
use alfredo_tui::{
    assessment::{Criterion, Decision, Outcome, ReviewRisk},
    autopilot::{self, Autopilot, RunState},
    command_intent::Intent,
    provider::Ollama,
    task_control::TaskControl,
    tasks::{Action, Request, TaskStatus, TaskStore},
    understanding,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
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
            "alfredo-autopilot-{}-{}",
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
        fs::write(workspace.join("README.md"), "Calculator fixture\n").unwrap();
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
    fn head(&self) -> String {
        git(&self.workspace, &["rev-parse", "HEAD"])
    }
    fn status(&self) -> String {
        git(&self.workspace, &["status", "--porcelain"])
    }
    fn directory(&self) -> PathBuf {
        self.store.conversation_directory().unwrap()
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

/// Threaded fixture Ollama: every POST is recorded and answered by `respond`.
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
fn two_task_plan() -> String {
    json!({"tasks": [
        {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
         "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
        {"title": "Add app reporting the answer", "acceptance": ["main() reports answer=42"], "model": "fixture",
         "dependencies": [1], "policy": {"files": ["app.py"], "check": check("from app import main; assert main() == 'answer=42'")}},
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
fn app() -> String {
    files(
        "app.py",
        "from calc import answer\n\n\ndef main():\n    return f'answer={answer()}'\n",
    )
}

fn control(fixture: &Fixture, server: &Server, runtime: &Runtime) -> TaskControl {
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(server.provider());
    control.refresh(runtime);
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending || control.scope_status.revision.is_none() {
        control.poll();
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(2));
    }
    control
}

/// Mirrors the terminal loop: autopilot submissions and automatic launches both
/// go through `dispatch_prepared`, exactly like saved console intents.
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
        if until(autopilot, control) {
            return;
        }
        if let Some(submission) = autopilot.tick(runtime, control) {
            assert!(!submission.text.trim().is_empty());
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
            "Timed out: {label}\n{:?}\n{}\n{}\n{:?}",
            autopilot.status(control),
            control.notice,
            control.planner.notice,
            control.snapshot.as_ref().map(|s| s
                .tasks
                .iter()
                .map(|t| (t.id, t.status.clone(), t.repair_of))
                .collect::<Vec<_>>())
        );
        thread::sleep(Duration::from_millis(3));
    }
}

fn finished(autopilot: &Autopilot, control: &TaskControl) -> bool {
    autopilot.status(control).is_some_and(|status| {
        matches!(
            status.state,
            RunState::Done | RunState::Partial | RunState::Failed
        )
    })
}

#[test]
fn go_plans_approves_dispatches_repairs_and_integrates_on_one_local_branch() {
    let fixture = Fixture::new();
    let head = fixture.head();
    let server = Server::new(|request, _| {
        if planner(request) {
            return two_task_plan();
        }
        let prompt = worker_prompt(request);
        if prompt.starts_with("Implement this task: Repair #1:") {
            assert!(prompt.contains("REPAIR CONTEXT"), "{prompt}");
            good_calc()
        } else if prompt.starts_with("Implement this task: Make answer") {
            bad_calc()
        } else if prompt.starts_with("Implement this task: Add app") {
            app()
        } else {
            panic!("Unexpected prompt {prompt}")
        }
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    assert!(autopilot.status(&control).is_none());
    autopilot
        .start("Make answer return 42 and add app", "fixture", 2, &control)
        .unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "completion",
        finished,
    );

    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Done, "{status:?}");
    assert_eq!((status.done, status.total, status.failed), (2, 2, 0));
    assert_eq!(status.repairs, 1);
    let snapshot = fixture.store.snapshot().unwrap();
    let statuses: Vec<_> = snapshot
        .tasks
        .iter()
        .map(|task| (task.id, task.status.clone(), task.repair_of))
        .collect();
    assert_eq!(
        statuses,
        vec![
            (1, TaskStatus::Failed, None),
            (2, TaskStatus::Accepted, None),
            (3, TaskStatus::Accepted, Some(1)),
        ]
    );
    // Acceptance used the existing criterion review path; dependents used the
    // accepted repair through the existing resolution receipt.
    assert!(snapshot.receipts.iter().any(|receipt| matches!(
        &receipt.request.action,
        Action::Decide { task: 3, decision } if decision.outcome == Outcome::Approved
            && decision.reason == "autopilot: check passed"
            && decision.criteria.iter().all(|criterion| criterion.met)
    )));
    assert!(snapshot
        .receipts
        .iter()
        .any(|receipt| matches!(receipt.request.action, Action::ResolveRepair { task: 3 })));
    assert!(snapshot.receipts.iter().any(|receipt| matches!(
        &receipt.request.action,
        Action::Repair { task: 1, reason } if reason.starts_with("autopilot:") && reason.contains("Check")
    )));
    let task2 = snapshot.tasks.iter().find(|task| task.id == 2).unwrap();
    assert_eq!(task2.run.as_ref().unwrap().inputs[0].source_task, Some(3));

    let branch = status.branch.clone().expect("integration branch");
    assert!(branch.starts_with("alfredo/go-"), "{branch}");
    assert_eq!(fixture.head(), head, "user HEAD must not move");
    assert_eq!(fixture.status(), "", "working files and index untouched");
    assert_eq!(
        git(&fixture.workspace, &["symbolic-ref", "HEAD"]),
        "refs/heads/main"
    );
    assert_eq!(
        git(&fixture.workspace, &["show", &format!("{branch}:calc.py")]),
        "def answer():\n    return 42"
    );
    assert!(git(&fixture.workspace, &["show", &format!("{branch}:app.py")]).contains("answer="));
    assert!(git(
        &fixture.workspace,
        &["merge-base", "--is-ancestor", &head, &branch]
    )
    .is_empty());
    let report = autopilot.report().unwrap();
    for text in [
        "#1",
        "repair #3",
        "#2",
        &branch,
        &format!("git switch {branch}"),
        &format!("git merge {branch}"),
    ] {
        assert!(report.contains(text), "{text} missing from {report}");
    }
    // Every worker, plus exactly one planner request.
    assert_eq!(server.count(), 4);
}

fn render_rows(
    app: &alfredo_tui::model::App,
    control: &TaskControl,
    w: u16,
    h: u16,
) -> Vec<String> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw_with_tasks(frame, app, control))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn existing_test_is_worker_reference_repairs_carry_output_and_nothing_accepted_reads_failed() {
    let fixture = Fixture::new();
    let test_source = "import unittest\nfrom calc import answer\n\nclass T(unittest.TestCase):\n    def test_answer(self):\n        self.assertEqual(answer(), 42)  # EXISTING_TEST_SENTINEL\n";
    fs::write(fixture.workspace.join("test_calc.py"), test_source).unwrap();
    git(&fixture.workspace, &["add", "."]);
    git(
        &fixture.workspace,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "test"],
    );
    let server = Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [{"title": "Make test_calc.py pass", "acceptance": ["tests pass"],
                "model": "fixture", "dependencies": [], "policy": {"files": ["calc.py"],
                "check": ["/usr/bin/python3", "-B", "-m", "unittest", "test_calc.py"]}}]})
            .to_string();
        }
        bad_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start(
            "Create calc.py so test_calc.py passes",
            "fixture",
            1,
            &control,
        )
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failed", finished);

    let prompts = server.prompts();
    let first = prompts
        .iter()
        .find(|p| p.starts_with("Implement this task: Make test_calc.py pass"))
        .unwrap();
    assert!(first.contains("READ-ONLY FILE test_calc.py\n"), "{first}");
    assert!(first.contains("EXISTING_TEST_SENTINEL"), "{first}");
    let snapshot = fixture.store.snapshot().unwrap();
    let repair = snapshot
        .tasks
        .iter()
        .find(|t| t.repair_of == Some(1))
        .unwrap();
    assert!(
        repair.title.contains("Check failed (exit 1): stderr: ")
            && repair.title.contains("AssertionError: 41 != 42"),
        "{}",
        repair.title
    );
    assert!(!repair.title.contains("/worktree/"), "{}", repair.title);

    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Failed, "{status:?}");
    assert_eq!(
        (status.done, status.total, status.failed, status.repairs),
        (0, 1, 1, 1)
    );
    let row = alfredo_tui::dashboard::autopilot_row(&status, 100);
    assert!(
        row.contains("Autopilot ✗ failed   0/1   1 failed   1 repair   "),
        "{row}"
    );
    let report = autopilot.report().unwrap();
    assert!(report.starts_with("✗ Autopilot failed\n"), "{report}");
    assert!(
        report.contains("\nTasks     0/1 accepted\nRepairs   1\n"),
        "{report}"
    );
    let notice = autopilot.take_finished_notice().unwrap();
    assert!(
        notice.starts_with("Autopilot failed   0/1 accepted   1 repair"),
        "{notice}"
    );
    assert!(!notice.contains('\n'));
    assert!(autopilot.take_finished_notice().is_none());

    control.snapshot = Some(snapshot);
    control.autopilot = Some(status);
    control.set_visible(true);
    let app = alfredo_tui::model::App::new("fixture".into());
    let rows = render_rows(&app, &control, 100, 30);
    assert!(
        rows.iter().any(|row| row.contains("├ work  0/1 done ")),
        "{rows:#?}"
    );
    assert!(rows[1].contains("   1 repair   "), "{rows:#?}");
}

#[test]
fn finished_autopilot_replaces_the_stale_start_notice_in_the_footer_once() {
    let fixture = Fixture::new();
    fixture.store.select_mission(true).unwrap();
    let server = Server::new(|request, _| {
        assert!(planner(request));
        json!({"tasks": [{"title": "Bad", "acceptance": ["x"], "model": "fixture", "dependencies": [2],
            "policy": {"files": ["calc.py"], "check": ["true"]}}]})
        .to_string()
    });
    let runtime = Runtime::new().unwrap();
    let mut work = alfredo_tui::workstation::Workstation::open(
        &fixture.root.join("state"),
        &fixture.workspace,
        "mission",
        "default",
        "fixture",
        server.provider(),
    )
    .unwrap();
    work.tasks.refresh(&runtime);
    let deadline = Instant::now() + Duration::from_secs(5);
    while work.tasks.pending || work.tasks.scope_status.revision.is_none() {
        work.tasks.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    work.app.notice = work
        .autopilot
        .start("Improve calc", "fixture", 2, &work.tasks)
        .unwrap();
    work.sync_autopilot();
    assert!(work.app.notice.starts_with("Autopilot started"));
    drive(
        &mut work.autopilot,
        &mut work.tasks,
        &runtime,
        "failed",
        finished,
    );
    assert!(work.sync_autopilot());
    assert!(
        work.app
            .notice
            .starts_with("Autopilot failed   Planning failed 3 times"),
        "{}",
        work.app.notice
    );
    let rows = render_rows(&work.app, &work.tasks, 100, 30);
    let screen = rows.join("\n");
    assert!(!screen.contains("Autopilot started"), "{screen}");
    assert!(rows[29].contains("Autopilot failed"), "{screen}");
    // No plan was saved: no `0/0` progress.
    assert!(rows[1].contains("Autopilot ✗ failed   00:"), "{screen}");
    assert!(!rows[1].contains("0/0"), "{screen}");
    // Reported once; a user notice afterwards is not overwritten.
    work.app.notice = "USER_NOTICE".into();
    work.sync_autopilot();
    assert_eq!(work.app.notice, "USER_NOTICE");
}

#[test]
fn finished_state_saved_by_an_older_build_still_loads_with_an_honest_state() {
    use sha2::{Digest, Sha256};
    let fixture = Fixture::new();
    let directory = fixture.directory();
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!(
        "autopilot-{:x}.json",
        Sha256::digest("default".as_bytes())
    ));
    // Exact field set written before partial/failed finish states existed.
    fs::write(
        &path,
        json!({"version": 1, "id": "0123456789abcdef", "goal": "Old goal", "model": "fixture",
            "max_repairs": 2, "phase": "done", "paused": false, "started": 100, "finished": 160,
            "plan_attempts": 0, "plan_error": null, "plan_request": "p", "save_request": "s",
            "first": null, "count": 1, "retry_cancelled": [],
            "notice": "Autopilot done · 0/1 accepted · no integration branch",
            "report": "Autopilot finished: Old goal\nNo integration branch: no task was accepted",
            "branch": null})
        .to_string(),
    )
    .unwrap();
    let control = TaskControl::new(fixture.store.clone());
    let mut autopilot = Autopilot::open(&directory, "default").unwrap();
    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Failed);
    assert_eq!((status.done, status.total), (0, 1));
    assert_eq!(status.elapsed, Duration::from_secs(60));
    assert!(autopilot.report().unwrap().contains("Old goal"));
    assert!(
        autopilot.take_finished_notice().is_none(),
        "no replayed notice"
    );
}

#[test]
fn invalid_plan_is_retried_twice_with_its_validation_errors_then_stops() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        assert!(planner(request));
        json!({"tasks": [{"title": "Bad", "acceptance": ["x"], "model": "fixture", "dependencies": [2],
            "policy": {"files": ["calc.py"], "check": ["true"]}}]})
        .to_string()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Improve calc", "fixture", 2, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failure", finished);
    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Failed);
    let prompts = server.prompts();
    assert_eq!(prompts.len(), 3, "{prompts:?}");
    assert!(!prompts[0].contains("rejected"));
    for prompt in &prompts[1..] {
        assert!(
            prompt.contains("Improve calc") && prompt.contains("dependencies"),
            "{prompt}"
        );
    }
    assert!(prompts[2].contains("attempt 1") && prompts[2].contains("attempt 2"));
    assert!(
        autopilot.notice().contains("Planning failed 3 times"),
        "{}",
        autopilot.notice()
    );
    assert!(autopilot.report().unwrap().contains("dependencies"));
    assert!(fixture.store.snapshot().unwrap().tasks.is_empty());
}

#[test]
fn exhausted_repairs_hold_the_task_block_dependents_and_integrate_independent_work() {
    let fixture = Fixture::new();
    let head = fixture.head();
    let server = Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [
                {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
                {"title": "Add app reporting the answer", "acceptance": ["main() reports answer=42"], "model": "fixture",
                 "dependencies": [1], "policy": {"files": ["app.py"], "check": check("from app import main; assert main() == 'answer=42'")}},
                {"title": "Write notes", "acceptance": ["notes exist"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["notes.txt"], "check": check("from pathlib import Path; assert Path('notes.txt').read_text()")}},
            ]})
            .to_string();
        }
        let prompt = worker_prompt(request);
        if prompt.starts_with("Implement this task: Write notes") {
            files("notes.txt", "independent\n")
        } else {
            bad_calc()
        }
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Answer, app and notes", "fixture", 1, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Partial);
    assert_eq!((status.done, status.total, status.failed), (1, 3, 2));
    assert!(
        alfredo_tui::dashboard::autopilot_row(&status, 100)
            .contains("Autopilot ◐ partial   1/3   2 failed"),
        "{status:?}"
    );
    let snapshot = fixture.store.snapshot().unwrap();
    let find = |id| snapshot.tasks.iter().find(|task| task.id == id).unwrap();
    assert_eq!(find(1).status, TaskStatus::Failed);
    assert_eq!(find(2).status, TaskStatus::Approved);
    assert!(find(2).run.is_none(), "dependent must stay blocked");
    assert_eq!(find(3).status, TaskStatus::Accepted);
    let repairs: Vec<_> = snapshot
        .tasks
        .iter()
        .filter(|task| task.repair_of.is_some())
        .collect();
    assert_eq!(repairs.len(), 1, "budget of one repair");
    assert_eq!(
        repairs[0].status,
        TaskStatus::Failed,
        "{}",
        autopilot.report().unwrap()
    );
    let branch = status.branch.clone().expect("accepted subset branch");
    assert!(git(
        &fixture.workspace,
        &["show", &format!("{branch}:notes.txt")]
    )
    .contains("independent"));
    assert_eq!(
        git(&fixture.workspace, &["show", &format!("{branch}:calc.py")]),
        "def answer():\n    return 0"
    );
    assert_eq!(fixture.head(), head);
    let report = autopilot.report().unwrap();
    assert!(report.contains("repair budget exhausted"), "{report}");
    assert!(report.contains("blocked by #1"), "{report}");
    assert!(
        report.contains("the branch holds the accepted tasks only"),
        "{report}"
    );
}

#[test]
fn pause_stops_decisions_human_hold_is_never_resolved_and_restart_restores_paused() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [
                {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
            ]})
            .to_string();
        }
        good_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Answer only", "fixture", 2, &control)
        .unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "worker start",
        |_, c| !c.workers.is_empty(),
    );
    autopilot.pause(&mut control);
    assert!(!control.dispatch.enabled);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Paused);
    // Running workers finish; paused autopilot records no review.
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "worker finish",
        |_, c| {
            c.workers.is_empty()
                && c.snapshot.as_ref().unwrap().tasks[0].status == TaskStatus::ReviewReady
        },
    );
    for _ in 0..20 {
        assert!(autopilot.tick(&runtime, &mut control).is_none());
    }
    // A human records a risk hold while paused.
    let revision = fixture.store.snapshot().unwrap().revision;
    fixture
        .store
        .transact(Request {
            correlation: "human-hold".into(),
            expected_revision: revision,
            action: Action::Decide {
                task: 1,
                decision: Decision {
                    failure: None,
                    risk: Some(ReviewRisk::Security),
                    outcome: Outcome::NeedsHumanReview,
                    reason: "Inspect secret handling".into(),
                    criteria: vec![Criterion {
                        criterion: 1,
                        met: false,
                        note: "Unverified".into(),
                    }],
                    limitations: vec![],
                },
            },
        })
        .unwrap();
    let requests = server.count();
    drop(autopilot);
    // Restart: state survives and is restored paused without replaying anything.
    let mut control = self::control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Paused);
    assert_eq!(status.goal, "Answer only");
    for _ in 0..20 {
        control.poll();
        assert!(autopilot.tick(&runtime, &mut control).is_none());
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(server.count(), requests);
    autopilot.resume(&control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "held", finished);
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks[0].status, TaskStatus::NeedsHumanReview);
    assert_eq!(snapshot.tasks.len(), 1, "no repair of a human hold");
    let status = autopilot.status(&control).unwrap();
    assert_eq!((status.done, status.failed), (0, 1));
    assert_eq!(status.state, RunState::Failed, "held with nothing accepted");
    assert!(status.branch.is_none());
    assert!(autopilot.report().unwrap().contains("human review"));
    assert_eq!(server.count(), requests);
}

#[test]
fn stop_cancels_running_workers_through_the_cancel_path() {
    let fixture = Fixture::new();
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let server = Server::new(move |request, _| {
        if planner(request) {
            return json!({"tasks": [
                {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
            ]})
            .to_string();
        }
        let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(10));
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
        "worker request",
        |_, _| server.count() == 2,
    );
    autopilot.stop(&runtime, &mut control);
    assert!(!control.dispatch.enabled);
    assert!(control
        .workers
        .values()
        .all(|flag| flag.load(Ordering::SeqCst)));
    drop(release);
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "cancelled",
        |_, c| c.workers.is_empty(),
    );
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Cancelled
    );
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Paused);
}

#[test]
fn new_project_scope_gate_is_satisfied_with_a_goal_scope_and_user_drafts_are_never_confirmed() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        assert!(planner(request));
        json!({"tasks": [{"title": "Scaffold", "acceptance": ["exists"], "model": "fixture", "dependencies": [],
            "policy": {"files": ["calc.py"], "check": ["true"]}}]})
        .to_string()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    let goal = "Build a new app for calculating answers";
    assert!(alfredo_tui::wayfinder::entry_mode(goal).is_some());
    autopilot.start(goal, "fixture", 2, &control).unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "plan saved",
        |_, c| c.snapshot.as_ref().is_some_and(|s| !s.tasks.is_empty()),
    );
    let scope = fixture.store.understanding().snapshot().unwrap();
    assert!(scope.confirmed);
    assert!(scope.brief.as_ref().unwrap().destination.contains(goal));
    let snapshot = fixture.store.snapshot().unwrap();
    let plan = snapshot.plan_for_task(1).unwrap();
    assert_eq!(plan.scope.as_ref().unwrap().revision, scope.revision);
    autopilot.stop(&runtime, &mut control);

    // A pending user-authored draft is never confirmed on the user's behalf.
    let other = Fixture::new();
    let scope = other.store.understanding();
    scope
        .transact(understanding::Request {
            correlation: "user-draft".into(),
            expected_revision: 0,
            action: understanding::Action::Draft {
                brief: understanding::Brief {
                    destination: "User destination".into(),
                    scope: "User scope".into(),
                    constraints: "User constraints".into(),
                    uncertainty: "User uncertainty".into(),
                },
            },
        })
        .unwrap();
    let control = self::control(&other, &server, &runtime);
    let mut autopilot = Autopilot::open(&other.directory(), "default").unwrap();
    let error = autopilot
        .start("Build a new app", "fixture", 2, &control)
        .unwrap_err();
    assert!(error.contains("/scope-confirm 1"), "{error}");
    assert!(!scope.snapshot().unwrap().confirmed);
    assert!(autopilot.status(&control).is_none());
}

#[test]
fn commands_and_header_line_expose_read_only_status() {
    for text in ["/go build it", "/pause", "/resume", "/stop", "/autopilot"] {
        assert!(autopilot::is_command(text), "{text}");
    }
    for text in ["/plan x", "/goal", "go", "/pauses"] {
        assert!(!autopilot::is_command(text), "{text}");
    }
    let status = autopilot::Status {
        state: RunState::Running,
        goal: "Make answer return 42".into(),
        done: 1,
        total: 2,
        failed: 0,
        repairs: 1,
        elapsed: Duration::from_secs(75),
        branch: None,
    };
    let line = status.line();
    for text in [
        "Autopilot",
        "running",
        "1/2",
        "repairs 1",
        "01:15",
        "Make answer",
    ] {
        assert!(line.contains(text), "{text} missing from {line}");
    }
    assert!(!line.contains('\n'));
}

#[test]
fn workstation_restores_autopilot_paused_and_blocks_switching_while_it_runs() {
    let fixture = Fixture::new();
    fixture.store.select_mission(true).unwrap();
    let open = |conversation: &str| {
        alfredo_tui::workstation::Workstation::open(
            &fixture.root.join("state"),
            &fixture.workspace,
            "mission",
            conversation,
            "fixture",
            Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
        )
        .unwrap()
    };
    let mut work = open("default");
    assert!(work.autopilot.status(&work.tasks).is_none());
    work.autopilot
        .start("Make answer return 42", "fixture", 2, &work.tasks)
        .unwrap();
    assert!(work.can_switch().unwrap_err().contains("autopilot"));
    work.autopilot.pause(&mut work.tasks);
    assert!(work.can_switch().is_ok());
    work.autopilot.resume(&work.tasks).unwrap();
    drop(work);
    let work = open("default");
    let status = work.autopilot.status(&work.tasks).unwrap();
    assert_eq!(status.state, RunState::Paused);
    assert!(work.can_switch().is_ok());
    // Another conversation set owns an independent autopilot.
    let other = open("second");
    assert!(other.autopilot.status(&other.tasks).is_none());
}

#[test]
fn header_shows_one_autopilot_line_and_the_report_opens_in_mission_work() {
    let fixture = Fixture::new();
    let mut control = TaskControl::new(fixture.store.clone());
    control.snapshot = Some(fixture.store.snapshot().unwrap());
    let app = alfredo_tui::model::App::new("fixture".into());
    let render = |control: &TaskControl| {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
        terminal
            .draw(|frame| alfredo_tui::ui::draw_with_tasks(frame, &app, control))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
    };
    assert!(!render(&control).join("\n").contains("Autopilot"));
    control.autopilot = Some(autopilot::Status {
        state: RunState::Running,
        goal: "Make answer return 42".into(),
        done: 1,
        total: 2,
        failed: 0,
        repairs: 1,
        elapsed: Duration::from_secs(5),
        branch: None,
    });
    let rows = render(&control);
    let lines: Vec<_> = rows
        .iter()
        .filter(|row| row.contains("Autopilot"))
        .collect();
    assert_eq!(lines.len(), 1, "{rows:#?}");
    assert!(
        lines[0].contains("running   1/2   1 repair"),
        "{}",
        lines[0]
    );
    assert!(!lines[0].contains("Make answer return 42"), "{}", lines[0]);
    control.set_visible(true);
    control.autopilot_report =
        Some("Autopilot finished: goal\nREPORT_SENTINEL git switch alfredo/go-x".into());
    assert!(render(&control).join("\n").contains("REPORT_SENTINEL"));
    control.set_visible(false);
    assert!(control.autopilot_report.is_none());
}

#[test]
fn cli_documents_and_validates_autopilot_flags() {
    let binary = env!("CARGO_BIN_EXE_alfredo-tui");
    let help = Command::new(binary).arg("--help").output().unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    for text in [
        "--go GOAL",
        "--max-repairs",
        "/go GOAL",
        "/pause",
        "/resume",
        "/stop",
        "F5",
    ] {
        assert!(help.contains(text), "{text} missing from help");
    }
    assert_eq!(alfredo_tui::autopilot::DEFAULT_MAX_REPAIRS, 3);
    assert!(help.contains("--max-repairs, default 3"), "{help}");
    for args in [
        vec!["--go"],
        vec!["--go", " "],
        vec!["--max-repairs"],
        vec!["--max-repairs", "x"],
        vec!["--max-repairs", "17"],
    ] {
        let output = Command::new(binary).args(&args).output().unwrap();
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty());
    }
    let completion = alfredo_tui::commands::Completion::open("/go").unwrap();
    assert!(completion.choices.iter().any(|choice| choice.name == "/go"));
    for name in ["/pause", "/resume", "/stop", "/autopilot"] {
        assert!(alfredo_tui::commands::COMMANDS
            .iter()
            .any(|(command, _)| *command == name));
    }
}

#[test]
fn manual_dispatch_off_pauses_autopilot_instead_of_being_overridden() {
    let fixture = Fixture::new();
    let mut control = TaskControl::new(fixture.store.clone());
    control.snapshot = Some(fixture.store.snapshot().unwrap());
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 2, &control).unwrap();
    autopilot.observe_manual("/tasks", &mut control);
    assert!(autopilot.running());
    autopilot.observe_manual("/dispatch off", &mut control);
    assert!(!autopilot.running());
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Paused);
}

#[test]
fn shell_string_check_is_rejected_before_unattended_approval_and_replanned() {
    let fixture = Fixture::new();
    let server = Server::new(|request, index| {
        assert!(planner(request), "no worker may start from a rejected plan");
        let check = if index == 0 {
            json!(["python3 -m unittest"])
        } else {
            json!(["/usr/bin/python3", "-m", "unittest"])
        };
        json!({"tasks": [{"title": "Add tests", "acceptance": ["tests pass"], "model": "fixture",
            "dependencies": [], "policy": {"files": ["test_calc.py"], "check": check}}]})
        .to_string()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Add calc tests", "fixture", 2, &control)
        .unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "plan saved",
        |_, c| c.snapshot.as_ref().is_some_and(|s| !s.tasks.is_empty()),
    );
    autopilot.pause(&mut control);
    let prompts = server.prompts();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("argv"), "{}", prompts[1]);
    let task = &fixture.store.snapshot().unwrap().tasks[0];
    assert_eq!(
        task.policy.as_ref().unwrap().check,
        vec!["/usr/bin/python3", "-m", "unittest"]
    );
}

/// How a concurrent writer overtakes a command that already captured its revision.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Race {
    /// The store moves; this controller still holds the older snapshot.
    Claim,
    /// The store moves and this controller observes it before dispatching.
    Admission,
    /// The scope lock is held by another writer until the refusal is observed.
    ScopeBusy,
}

/// Another writer (a parallel worker receipt or a human) lands one unrelated receipt.
fn concurrent_write(fixture: &Fixture) {
    let revision = fixture.store.snapshot().unwrap().revision;
    fixture
        .store
        .transact(Request {
            correlation: format!("concurrent-{revision}"),
            expected_revision: revision,
            action: Action::Propose {
                title: "Unrelated concurrent note".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        })
        .unwrap();
}

/// `drive` without periodic refresh, so only the controller's own receipts move
/// its snapshot. Before a command selected by `races` is dispatched (after it
/// captured its revision), `concurrent_write` overtakes it. Stops when finished
/// or paused.
fn drive_racing(
    fixture: &Fixture,
    autopilot: &mut Autopilot,
    control: &mut TaskControl,
    runtime: &Runtime,
    mut races: impl FnMut(&str) -> Option<Race>,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    let scope = fixture.store.understanding();
    let mut race = |label: &str, control: &mut TaskControl| {
        let race = races(label)?;
        if race == Race::ScopeBusy {
            return Some(scope.lock().unwrap());
        }
        concurrent_write(fixture);
        if race == Race::Admission {
            control.refresh(runtime);
            while control.pending {
                control.poll();
                assert!(Instant::now() < deadline, "refresh: {}", control.notice);
                thread::sleep(Duration::from_millis(1));
            }
        }
        None
    };
    let release = |held: &mut Option<_>, control: &mut TaskControl, launch: bool| {
        while held.is_some() {
            control.poll();
            let seen = if launch {
                !control.dispatch.transient.is_empty()
            } else {
                !control.pending
            };
            if seen {
                *held = None;
            }
            assert!(Instant::now() < deadline, "scope busy: {}", control.notice);
            thread::sleep(Duration::from_millis(1));
        }
    };
    loop {
        control.poll();
        if finished(autopilot, control) || !autopilot.running() {
            return;
        }
        if let Some(submission) = autopilot.tick(runtime, control) {
            let mut held = race(&submission.text, control);
            let _ = control.dispatch_prepared(runtime, &submission.intent);
            release(&mut held, control, false);
        }
        if let Ok(Some(request)) = control.prepare_dispatch() {
            let mut held = race(&format!("launch #{}", request.task), control);
            let _ = control.dispatch_prepared(runtime, &Intent::DispatchRun { request });
            release(&mut held, control, true);
        }
        assert!(
            Instant::now() < deadline,
            "Timed out racing\n{:?}\n{}",
            autopilot.status(control),
            control.notice
        );
        thread::sleep(Duration::from_millis(3));
    }
}

fn starts(snapshot: &alfredo_tui::tasks::Snapshot, id: u64) -> usize {
    snapshot
        .receipts
        .iter()
        .filter(
            |receipt| matches!(receipt.request.action, Action::Start { task, .. } if task == id),
        )
        .count()
}

fn repair_launch_race(race: Race) {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [
                {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
                {"title": "Write notes", "acceptance": ["notes exist"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["notes.txt"], "check": check("from pathlib import Path; assert Path('notes.txt').read_text()")}},
            ]})
            .to_string();
        }
        if worker_prompt(request).starts_with("Implement this task: Write notes") {
            files("notes.txt", "independent\n")
        } else {
            bad_calc()
        }
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Answer and notes", "fixture", 1, &control)
        .unwrap();
    let mut raced = 0;
    drive_racing(&fixture, &mut autopilot, &mut control, &runtime, |label| {
        (label == "launch #3" && raced == 0).then(|| {
            raced += 1;
            race
        })
    });
    assert_eq!(raced, 1, "the repair launch was raced");
    let status = autopilot.status(&control).unwrap();
    let report = autopilot.report().unwrap_or_default().to_string();
    assert_eq!(
        status.state,
        RunState::Partial,
        "{status:?} {}",
        autopilot.notice()
    );
    assert_eq!((status.done, status.total, status.failed), (1, 2, 1));
    assert!(report.contains("repair budget exhausted"), "{report}");
    assert!(!report.contains("launch failed"), "{report}");
    let snapshot = fixture.store.snapshot().unwrap();
    let repairs: Vec<_> = snapshot
        .tasks
        .iter()
        .filter(|task| task.repair_of.is_some())
        .collect();
    assert_eq!(repairs.len(), 1, "exactly one repair child");
    assert_eq!(
        (repairs[0].id, repairs[0].status.clone()),
        (3, TaskStatus::Failed)
    );
    assert_eq!(starts(&snapshot, 3), 1, "exactly one repair run");
    assert!(
        control.dispatch.failures.is_empty(),
        "{:?}",
        control.dispatch.failures
    );
}

#[test]
fn repair_launch_overtaken_at_its_claim_retries_on_current_state_once() {
    repair_launch_race(Race::Claim);
}

#[test]
fn repair_launch_overtaken_before_admission_retries_on_current_state_once() {
    repair_launch_race(Race::Admission);
}

fn one_good_task() -> Server {
    Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [
                {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
            ]})
            .to_string();
        }
        good_calc()
    })
}

fn decisions(snapshot: &alfredo_tui::tasks::Snapshot, id: u64) -> usize {
    snapshot
        .receipts
        .iter()
        .filter(
            |receipt| matches!(receipt.request.action, Action::Decide { task, .. } if task == id),
        )
        .count()
}

#[test]
fn review_overtaken_by_a_concurrent_write_is_resubmitted_on_current_state() {
    let fixture = Fixture::new();
    let server = one_good_task();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    let mut raced = 0;
    drive_racing(&fixture, &mut autopilot, &mut control, &runtime, |label| {
        (label.contains("/review 1") && raced == 0).then(|| {
            raced += 1;
            Race::Claim
        })
    });
    assert_eq!(raced, 1);
    let status = autopilot.status(&control).unwrap();
    assert_eq!(
        status.state,
        RunState::Done,
        "{status:?} {}",
        autopilot.notice()
    );
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(decisions(&snapshot, 1), 1, "exactly one review");
    assert_eq!(starts(&snapshot, 1), 1, "exactly one run");
}

#[test]
fn resolve_repair_overtaken_by_a_concurrent_write_is_resubmitted_on_current_state() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            return two_task_plan();
        }
        let prompt = worker_prompt(request);
        if prompt.starts_with("Implement this task: Repair #1:") {
            good_calc()
        } else if prompt.starts_with("Implement this task: Make answer") {
            bad_calc()
        } else {
            app()
        }
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Make answer return 42 and add app", "fixture", 2, &control)
        .unwrap();
    let mut raced = 0;
    drive_racing(&fixture, &mut autopilot, &mut control, &runtime, |label| {
        (label.contains("/resolve-repair 3") && raced == 0).then(|| {
            raced += 1;
            Race::Claim
        })
    });
    assert_eq!(raced, 1);
    let status = autopilot.status(&control).unwrap();
    assert_eq!(
        status.state,
        RunState::Done,
        "{status:?} {}",
        autopilot.notice()
    );
    assert_eq!(status.repairs, 1);
    let snapshot = fixture.store.snapshot().unwrap();
    let resolutions = snapshot
        .receipts
        .iter()
        .filter(|receipt| matches!(receipt.request.action, Action::ResolveRepair { .. }))
        .count();
    assert_eq!(resolutions, 1, "exactly one resolution");
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .filter(|t| t.repair_of.is_some())
            .count(),
        1
    );
    assert_eq!(starts(&snapshot, 3), 1);
}

#[test]
fn five_consecutive_overtaken_reviews_pause_then_resume_reviews_once() {
    let fixture = Fixture::new();
    let server = one_good_task();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    let mut raced = 0;
    drive_racing(&fixture, &mut autopilot, &mut control, &runtime, |label| {
        label.contains("/review 1").then(|| {
            raced += 1;
            Race::Claim
        })
    });
    assert_eq!(raced, 5, "bounded to five transient refusals");
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Paused);
    assert!(
        autopilot.notice().contains("task state kept changing"),
        "{}",
        autopilot.notice()
    );
    assert_eq!(decisions(&fixture.store.snapshot().unwrap(), 1), 0);
    autopilot.resume(&control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Done);
    assert_eq!(decisions(&fixture.store.snapshot().unwrap(), 1), 1);
}

#[test]
fn busy_scope_lock_on_review_is_transient_and_spends_no_attempt() {
    let fixture = Fixture::new();
    let server = one_good_task();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    let mut raced = 0;
    drive_racing(&fixture, &mut autopilot, &mut control, &runtime, |label| {
        (label.contains("/review 1") && raced < 5).then(|| {
            raced += 1;
            Race::ScopeBusy
        })
    });
    assert_eq!(raced, 5);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Paused);
    assert!(
        autopilot.notice().contains("task state kept changing"),
        "{}",
        autopilot.notice()
    );
    assert_eq!(decisions(&fixture.store.snapshot().unwrap(), 1), 0);
    autopilot.resume(&control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Done);
    assert_eq!(decisions(&fixture.store.snapshot().unwrap(), 1), 1);
}

#[test]
fn busy_scope_lock_on_launch_defers_without_spending_the_attempt() {
    let fixture = Fixture::new();
    let server = one_good_task();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    let mut raced = 0;
    drive_racing(&fixture, &mut autopilot, &mut control, &runtime, |label| {
        (label == "launch #1" && raced < 2).then(|| {
            raced += 1;
            Race::ScopeBusy
        })
    });
    assert_eq!(raced, 2);
    assert!(control.dispatch.failures.is_empty());
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Done);
    assert_eq!(starts(&fixture.store.snapshot().unwrap(), 1), 1);
}

#[test]
fn five_consecutive_overtaken_launches_pause_without_spending_the_attempt() {
    let fixture = Fixture::new();
    let server = one_good_task();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    let mut raced = 0;
    drive_racing(&fixture, &mut autopilot, &mut control, &runtime, |label| {
        (label == "launch #1").then(|| {
            raced += 1;
            Race::Claim
        })
    });
    assert_eq!(raced, 5, "bounded to five transient refusals");
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Paused);
    assert!(
        autopilot.notice().contains("task state kept changing"),
        "{}",
        autopilot.notice()
    );
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(starts(&snapshot, 1), 0);
    assert!(control.dispatch.failures.is_empty());
    autopilot.resume(&control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Done);
    assert_eq!(starts(&fixture.store.snapshot().unwrap(), 1), 1);
}

#[test]
fn identical_repair_is_reported_as_no_progress_and_the_next_repair_samples_hotter() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [{"title": "Make answer return 42", "acceptance": ["answer() is 42"],
                "model": "fixture", "dependencies": [], "policy": {"files": ["calc.py"],
                "check": check("from calc import answer; assert answer() == 42, answer()")}}]})
            .to_string();
        }
        bad_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Make answer return 42", "fixture", 2, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failed", finished);
    let snapshot = fixture.store.snapshot().unwrap();
    let second = snapshot
        .tasks
        .iter()
        .find(|t| t.repair_of == Some(2))
        .unwrap();
    assert!(
        second
            .title
            .contains("autopilot: No change from previous attempt · Check failed"),
        "{}",
        second.title
    );
    let temperatures: Vec<_> = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| !planner(request))
        .map(|request| request["options"]["temperature"].as_f64().unwrap())
        .collect();
    assert_eq!(temperatures, vec![0.0, 0.0, 0.6]);
    let last = server.prompts().pop().unwrap();
    assert!(
        last.contains("previous attempt returned identical code that still fails"),
        "{last}"
    );
}

fn textutil_plan(first_check: Value) -> String {
    json!({"tasks": [
        {"title": "Create textutil", "acceptance": ["word_count and reverse_words work"], "model": "fixture",
         "dependencies": [], "policy": {"files": ["textutil.py"], "check": first_check}},
        {"title": "Test textutil", "acceptance": ["tests cover both"], "model": "fixture",
         "dependencies": [1], "policy": {"files": ["test_textutil.py"],
         "check": ["/usr/bin/python3", "-m", "unittest", "test_textutil.py"]}},
    ]})
    .to_string()
}

#[test]
fn check_needing_a_later_tasks_file_is_replanned_with_accumulated_errors() {
    let fixture = Fixture::new();
    let server = Server::new(|request, index| {
        assert!(planner(request), "no worker may start from a rejected plan");
        textutil_plan(if index < 2 {
            json!(["/usr/bin/python3", "-m", "unittest", "test_textutil.py"])
        } else {
            json!(["/usr/bin/python3", "-c", "import textutil"])
        })
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Create textutil and its tests", "fixture", 2, &control)
        .unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "plan saved",
        |_, c| c.snapshot.as_ref().is_some_and(|s| !s.tasks.is_empty()),
    );
    autopilot.pause(&mut control);
    let prompts = server.prompts();
    assert_eq!(prompts.len(), 3, "{prompts:?}");
    let error = "Task 1 check references test_textutil.py, which does not exist yet and is not written by task 1 or its dependencies";
    assert!(prompts[1].contains(error), "{}", prompts[1]);
    assert!(
        prompts[2].contains("attempt 1") && prompts[2].contains("attempt 2"),
        "{}",
        prompts[2]
    );
    assert_eq!(prompts[2].matches(error).count(), 2, "{}", prompts[2]);
    let tasks = fixture.store.snapshot().unwrap().tasks;
    assert_eq!(tasks.len(), 2);
    assert_eq!(
        tasks[0].policy.as_ref().unwrap().check,
        vec!["/usr/bin/python3", "-c", "import textutil"]
    );
}

/// `drive`, recording each choice in the conversation exactly as the terminal
/// does: autopilot commands, automatic launches and controller/planner outcomes.
fn drive_recorded(
    app: &mut alfredo_tui::model::App,
    autopilot: &mut Autopilot,
    control: &mut TaskControl,
    runtime: &Runtime,
) {
    use alfredo_tui::console_command::CommandState;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut refreshed = Instant::now();
    loop {
        control.poll();
        let session = &mut app.sessions[0];
        for event in control.take_control_events() {
            let intent = Intent::Control {
                request: event.request,
            };
            let id = alfredo_tui::console_command::ConsoleCommand::identity(&intent);
            session.set_command_state(
                &id,
                CommandState::Control {
                    outcome: event.outcome,
                },
            );
        }
        for event in control.planner.take_command_events() {
            let ids: Vec<_> = session
                .commands()
                .iter()
                .filter(|command| command.intent.planner_request() == Some(&event.request))
                .map(|command| command.id.clone())
                .collect();
            for id in ids {
                session.set_command_state(
                    &id,
                    CommandState::Planner {
                        outcome: event.outcome.clone(),
                    },
                );
            }
        }
        if finished(autopilot, control) && control.workers.is_empty() && !control.owner.active() {
            return;
        }
        let submission = alfredo_tui::instruct::Instructions::tick(control, autopilot)
            .or_else(|| autopilot.tick(runtime, control));
        if let Some(submission) = submission {
            let id = session
                .submit_autopilot_command(submission.text, submission.intent.clone())
                .unwrap();
            let state = match control.dispatch_prepared(runtime, &submission.intent) {
                Ok(()) => CommandState::Submitted,
                Err(reason) => CommandState::Refused { reason },
            };
            session.set_command_state(&id, state);
        }
        if let Ok(Some(request)) = control.prepare_dispatch() {
            let parent = session
                .commands()
                .iter()
                .find(|command| {
                    matches!(&command.intent, Intent::Control { request: source } if *source == request.source)
                })
                .unwrap()
                .sequence;
            let intent = Intent::DispatchRun {
                request: request.clone(),
            };
            let id = session
                .submit_automatic_command(
                    format!(
                        "Automatic /run {} · dispatch command #{parent}",
                        request.task
                    ),
                    intent.clone(),
                )
                .unwrap();
            if control.dispatch_prepared(runtime, &intent).is_ok() {
                session.set_command_state(&id, CommandState::Submitted);
            }
        }
        if refreshed.elapsed() > Duration::from_millis(100) {
            control.refresh_background(runtime);
            refreshed = Instant::now();
        }
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(3));
    }
}

#[test]
fn chat_collapses_autopilot_commands_into_one_line_per_task() {
    let fixture = Fixture::new();
    let server = one_good_task();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    let mut app = alfredo_tui::model::App::new("fixture".into());
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    drive_recorded(&mut app, &mut autopilot, &mut control, &runtime);
    assert!(app.sessions[0].commands().len() >= 5);
    control.set_visible(false);
    let rows = render_rows(&app, &control, 100, 30);
    let screen = rows.join("\n");
    let start = alfredo_tui::ui::pane_width(100) as usize + 2;
    let pane: Vec<String> = rows[2..rows.len() - 6]
        .iter()
        .map(|row| {
            row.chars()
                .skip(start)
                .collect::<String>()
                .trim_end_matches(['│', ' '])
                .to_string()
        })
        .filter(|row| !row.is_empty())
        .collect();
    assert_eq!(
        pane,
        [
            "✓ plan drafted",
            "✓ #1 planned → approved → started → check passed → accepted"
        ],
        "{screen}"
    );
    // An owner follow-up's steps collapse too, wrapping under their text.
    alfredo_tui::instruct::Instructions::give_to(
        &mut control,
        alfredo_tui::agent_view::Target::Task(1),
        "keep answer at 42",
    )
    .unwrap();
    drive_recorded(&mut app, &mut autopilot, &mut control, &runtime);
    control.set_visible(false);
    let rows = render_rows(&app, &control, 100, 30);
    let screen = rows.join("\n");
    let pane: Vec<String> = rows[2..rows.len() - 6]
        .iter()
        .map(|row| {
            row.chars()
                .skip(start)
                .collect::<String>()
                .trim_end_matches(['│', ' '])
                .to_string()
        })
        .filter(|row| !row.is_empty())
        .collect();
    assert_eq!(
        pane[2..],
        [
            "✓ #2 proposed → files and check set → approved → started → check",
            "  passed → accepted"
        ],
        "{screen}"
    );
}

#[test]
fn finished_report_reads_as_labeled_sections_and_the_footer_uses_wide_gaps() {
    let fixture = Fixture::new();
    let server = one_good_task();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    let goal = "create calc.py so that answer() returns the integer forty-two, with a check that proves it and nothing else changed anywhere in the repository";
    autopilot.start(goal, "fixture", 1, &control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    let branch = autopilot.status(&control).unwrap().branch.unwrap();
    assert_eq!(
        autopilot.report().unwrap(),
        format!(
            "✓ Autopilot done\n{goal}\n\nTasks     1/1 accepted\nRepairs   0\nBranch    {branch}\n\n✓ #1  Make answer return 42\n\nReview    git switch {branch}\nMerge     git merge {branch}"
        )
    );
    assert_eq!(
        autopilot.take_finished_notice().unwrap(),
        format!("Autopilot done   1/1 accepted   git switch {branch}")
    );
    let app = alfredo_tui::model::App::new("fixture".into());
    control.set_visible(true);
    control.autopilot_report = autopilot.report().map(str::to_owned);
    let rows = render_rows(&app, &control, 100, 30);
    let screen = rows.join("\n");
    let start = alfredo_tui::ui::pane_width(100) as usize + 2;
    let pane: Vec<String> = rows
        .iter()
        .map(|row| {
            row.chars()
                .skip(start)
                .collect::<String>()
                .trim_end_matches(['│', ' '])
                .to_string()
        })
        .collect();
    let at = pane
        .iter()
        .position(|row| row == "✓ Autopilot done")
        .unwrap_or_else(|| panic!("{screen}"));
    // The goal takes one line, cut with an ellipsis.
    assert!(
        pane[at + 1].starts_with("create calc.py so that"),
        "{screen}"
    );
    assert!(pane[at + 1].ends_with('…'), "{screen}");
    assert_eq!(
        pane[at + 2..at + 11],
        [
            "".to_string(),
            "Tasks     1/1 accepted".into(),
            "Repairs   0".into(),
            format!("Branch    {branch}"),
            "".into(),
            "✓ #1  Make answer return 42".into(),
            "".into(),
            format!("Review    git switch {branch}"),
            format!("Merge     git merge {branch}"),
        ],
        "{screen}"
    );
}
