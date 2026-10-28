//! Autopilot drives the existing governed transactions; real HTTP fixtures and workers.
mod ollama_fixture;
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
    /// The mission state directory. `conversation_directory` takes the store lock
    /// with a 250 ms deadline, so while workers hold it on a loaded runner a bare
    /// `unwrap` flaked (#442); retry the busy refusal for a bounded time instead.
    fn directory(&self) -> PathBuf {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.store.conversation_directory() {
                Ok(directory) => return directory,
                Err(error) if error.contains("busy") && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(20))
                }
                Err(error) => panic!("conversation directory: {error}"),
            }
        }
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

/// `respond` returns this to make the fixture close the socket without answering.
const DROP_CONNECTION: &str = "\0drop-connection";

/// Threaded fixture Ollama: every POST is recorded and answered by `respond`.
struct Server {
    endpoint: String,
    requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(respond: impl Fn(&Value, usize) -> String + Send + Sync + 'static) -> Self {
        Self::with_usage(respond, |_| None)
    }
    /// `usage` returns the server's final-frame count fields for a request, if any.
    fn with_usage(
        respond: impl Fn(&Value, usize) -> String + Send + Sync + 'static,
        usage: impl Fn(&Value) -> Option<Value> + Send + Sync + 'static,
    ) -> Self {
        let usage = Arc::new(usage);
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
                let (seen, respond, usage) = (seen.clone(), respond.clone(), usage.clone());
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
                    if content == DROP_CONNECTION {
                        // Closes without a response, as a restarting Ollama does.
                        return;
                    }
                    let mut frame = json!({"message":{"content":content},"done":true});
                    if let Some(Value::Object(counts)) = usage(&request) {
                        frame.as_object_mut().unwrap().extend(counts);
                    }
                    let body = format!("{frame}\n");
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
    // The tip's subject names the goal; the candidate commits below read as task titles.
    assert_eq!(
        git(&fixture.workspace, &["log", "-1", "--format=%s", &branch]),
        "Make answer return 42 and add app"
    );
    let subjects = git(
        &fixture.workspace,
        &["log", "--format=%s", &format!("{head}..{branch}")],
    );
    assert!(!subjects.contains("Alfredo candidate"), "{subjects}");
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

fn go_with(server: &Server, fixture: &Fixture) -> (Autopilot, TaskControl) {
    let runtime = Runtime::new().unwrap();
    let mut control = control(fixture, server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
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
    (autopilot, control)
}

fn scripted(request: &Value, _: usize) -> String {
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
}

#[test]
fn finished_go_sums_planner_worker_and_repair_tokens_and_saves_the_planner_counts() {
    let fixture = Fixture::new();
    let server = Server::with_usage(scripted, |request| {
        let (prompt, eval, ns) = if planner(request) {
            (1000, 100, 2_000_000_000u64)
        } else {
            (200, 50, 1_000_000_000)
        };
        Some(json!({"prompt_eval_count": prompt, "eval_count": eval, "total_duration": ns}))
    });
    let (mut autopilot, _control) = go_with(&server, &fixture);
    // Planner 1000/100 + three worker runs (task 1, its repair 3, task 2) of 200/50.
    let want = "tokens 1.6k in / 250 out · model 5 s";
    let notice = autopilot.take_finished_notice().unwrap();
    assert!(notice.contains(want), "{notice}");
    assert!(autopilot.report().unwrap().contains(want));
    let snapshot = fixture.store.snapshot().unwrap();
    let usage = snapshot
        .receipts
        .iter()
        .find_map(|receipt| match &receipt.request.action {
            Action::Plan { plan } => plan.usage,
            _ => None,
        })
        .expect("plan receipt keeps the planner usage");
    assert_eq!(
        (usage.prompt_tokens, usage.completion_tokens, usage.model_ns),
        (Some(1000), Some(100), Some(2_000_000_000))
    );
}

#[test]
fn finished_go_without_server_counts_has_no_totals_line_and_no_plan_usage() {
    let fixture = Fixture::new();
    let server = Server::new(scripted);
    let (mut autopilot, _control) = go_with(&server, &fixture);
    let notice = autopilot.take_finished_notice().unwrap();
    assert!(!notice.contains("tokens"), "{notice}");
    assert!(!autopilot.report().unwrap().contains("tokens"));
    let snapshot = fixture.store.snapshot().unwrap();
    assert!(snapshot
        .receipts
        .iter()
        .all(|receipt| match &receipt.request.action {
            Action::Plan { plan } => plan.usage.is_none(),
            _ => true,
        }));
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
fn saved_state_with_an_out_of_range_repair_limit_is_not_loaded() {
    let fixture = Fixture::new();
    let directory = fixture.directory();
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        autopilot::state_path(&directory, "default"),
        json!({"version": 1, "id": "0123456789abcdef", "goal": "Goal", "model": "fixture",
            "max_repairs": 2, "phase": "done", "paused": false, "started": 100, "finished": 160,
            "first": 1, "count": 1, "limits": {"1": 300}})
        .to_string(),
    )
    .unwrap();
    let control = TaskControl::new(fixture.store.clone());
    let opened = Autopilot::open(&directory, "default");
    assert!(opened.map_or(true, |autopilot| autopilot.status(&control).is_none()));
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

/// The two-task plan whose worker and first repair write wrong content; the owner's
/// `/repair` (title `Repair #3:`) gets the right content once `fixed` is set.
fn repair_recovery_server(fixed: Arc<AtomicBool>) -> Server {
    Server::new(move |request, _| {
        if planner(request) {
            return two_task_plan();
        }
        let prompt = worker_prompt(request);
        if prompt.starts_with("Implement this task: Repair #3:") && fixed.load(Ordering::SeqCst) {
            good_calc()
        } else if prompt.starts_with("Implement this task: Add app") {
            app()
        } else if prompt.starts_with("Implement this task: Repair #1:")
            && fixed.load(Ordering::SeqCst)
        {
            good_calc()
        } else {
            bad_calc()
        }
    })
}

/// Type `/repair ID hint` as the owner does and wait for the saved repair task.
fn owner_repair(control: &mut TaskControl, runtime: &Runtime, id: u64, hint: &str) -> u64 {
    let before = control.snapshot.as_ref().unwrap().tasks.len();
    let intent = control
        .prepare_command(&format!("/repair {id} {hint}"), "fixture")
        .unwrap()
        .expect("repair is a task command");
    control.dispatch_prepared(runtime, &intent).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.snapshot.as_ref().unwrap().tasks.len() == before {
        control.poll();
        assert!(
            Instant::now() < deadline,
            "repair saved: {}",
            control.notice
        );
        thread::sleep(Duration::from_millis(2));
    }
    control.snapshot.as_ref().unwrap().tasks.len() as u64
}

fn failed_two_task_run(
    fixture: &Fixture,
    server: &Server,
    runtime: &Runtime,
) -> (Autopilot, TaskControl) {
    let mut control = control(fixture, server, runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Make answer return 42 and add app", "fixture", 1, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, runtime, "failed", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Failed);
    (autopilot, control)
}

#[test]
fn repair_after_a_failed_go_reenters_the_loop_unblocks_dependents_and_integrates() {
    let fixture = Fixture::new();
    let head = fixture.head();
    let fixed = Arc::new(AtomicBool::new(false));
    let server = repair_recovery_server(fixed.clone());
    let runtime = Runtime::new().unwrap();
    let (mut autopilot, mut control) = failed_two_task_run(&fixture, &server, &runtime);
    let report = autopilot.report().unwrap().to_string();
    assert!(report.contains("repair budget exhausted (1/1)"), "{report}");
    assert!(
        report.contains("Next: Try a larger model, /repair 3 <hint>"),
        "{report}"
    );
    let resume = autopilot.resume(&control).unwrap_err();
    assert!(
        resume.contains("/repair 3 <hint>") && resume.contains("/go GOAL"),
        "{resume}"
    );

    fixed.store(true, Ordering::SeqCst);
    assert_eq!(
        owner_repair(&mut control, &runtime, 3, "return 42 please"),
        4
    );
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "repaired",
        |autopilot, control| {
            autopilot
                .status(control)
                .is_some_and(|status| status.state == RunState::Done)
        },
    );
    let status = autopilot.status(&control).unwrap();
    assert_eq!((status.done, status.total, status.failed), (2, 2, 0));
    let snapshot = fixture.store.snapshot().unwrap();
    let find = |id| snapshot.tasks.iter().find(|task| task.id == id).unwrap();
    assert_eq!(find(4).status, TaskStatus::Accepted);
    assert_eq!(find(2).status, TaskStatus::Accepted, "the dependent ran");
    assert_eq!(snapshot.tasks.len(), 4, "no further repair was created");
    let branch = status.branch.clone().expect("integration branch");
    assert!(branch.starts_with("alfredo/go-"), "{branch}");
    assert_eq!(
        git(&fixture.workspace, &["show", &format!("{branch}:calc.py")]),
        "def answer():\n    return 42"
    );
    assert!(git(&fixture.workspace, &["show", &format!("{branch}:app.py")]).contains("main"));
    let report = autopilot.report().unwrap();
    assert!(report.contains("via repair #4"), "{report}");
    assert_eq!(fixture.head(), head);
    assert_eq!(fixture.status(), "");
}

#[test]
fn repair_after_a_failed_go_that_fails_again_ends_failed_with_the_same_guidance() {
    let fixture = Fixture::new();
    let fixed = Arc::new(AtomicBool::new(false));
    let server = repair_recovery_server(fixed);
    let runtime = Runtime::new().unwrap();
    let (mut autopilot, mut control) = failed_two_task_run(&fixture, &server, &runtime);
    assert_eq!(owner_repair(&mut control, &runtime, 3, "try harder"), 4);
    // The loop re-entered: wait until the owner's repair has run and failed.
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "refailed",
        |autopilot, control| {
            let snapshot = control.snapshot.as_ref().unwrap();
            autopilot
                .status(control)
                .is_some_and(|status| status.state == RunState::Failed)
                && snapshot
                    .tasks
                    .iter()
                    .any(|task| task.id == 4 && task.status == TaskStatus::Failed)
                && autopilot
                    .report()
                    .is_some_and(|report| report.contains("Last failure:"))
        },
    );
    let report = autopilot.report().unwrap().to_string();
    assert!(report.contains("repair budget exhausted (2/2)"), "{report}");
    assert!(
        report.contains("Next: Try a larger model, /repair 4 <hint>"),
        "{report}"
    );
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(
        snapshot.tasks.len(),
        4,
        "only the explicit /repair added one"
    );
    assert!(autopilot
        .resume(&control)
        .unwrap_err()
        .contains("/repair 4 <hint>"));
}

#[test]
fn repair_after_a_partial_go_reruns_only_its_family_and_integrates_a_new_branch() {
    let fixture = Fixture::new();
    let fixed = Arc::new(AtomicBool::new(false));
    let flag = fixed.clone();
    let server = Server::new(move |request, _| {
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
            files("notes.txt", "")
        } else if prompt.starts_with("Implement this task: Add app") {
            app()
        } else if prompt.contains("Repair #1:") && flag.load(Ordering::SeqCst) {
            good_calc()
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
    drive(&mut autopilot, &mut control, &runtime, "partial", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Failed);
    let before = control.snapshot.as_ref().unwrap().tasks.len();
    let head = |control: &TaskControl, root: u64| {
        let snapshot = control.snapshot.as_ref().unwrap();
        snapshot
            .tasks
            .iter()
            .filter(|task| snapshot.repair_root(task.id) == Some(root))
            .map(|task| task.id)
            .max()
            .unwrap()
    };
    let (answer_head, notes_head) = (head(&control, 1), head(&control, 3));
    fixed.store(true, Ordering::SeqCst);
    let repair = owner_repair(&mut control, &runtime, answer_head, "return 42 please");
    assert_eq!(repair as usize, before + 1);
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "repaired",
        |autopilot, control| {
            autopilot.status(control).is_some_and(|status| {
                status.state == RunState::Partial && status.done == 2 && status.branch.is_some()
            })
        },
    );
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(
        snapshot.tasks.len(),
        before + 1,
        "only the owner's repair was added"
    );
    assert_eq!(
        head(&control, 3),
        notes_head,
        "the other failed family stays stuck"
    );
    let status = autopilot.status(&control).unwrap();
    let branch = status.branch.unwrap();
    assert_eq!(
        git(&fixture.workspace, &["show", &format!("{branch}:calc.py")]),
        "def answer():\n    return 42"
    );
    let report = autopilot.report().unwrap();
    assert!(report.contains("repair budget exhausted"), "{report}");
    assert!(
        report.contains("the branch holds the accepted tasks only"),
        "{report}"
    );
}

#[test]
fn a_finished_run_restored_after_restart_replays_nothing_and_still_honours_a_later_repair() {
    let fixture = Fixture::new();
    let fixed = Arc::new(AtomicBool::new(false));
    let server = repair_recovery_server(fixed.clone());
    let runtime = Runtime::new().unwrap();
    let (autopilot, mut control) = failed_two_task_run(&fixture, &server, &runtime);
    let tasks = control.snapshot.as_ref().unwrap().tasks.len();
    drop(autopilot);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    for _ in 0..50 {
        control.poll();
        assert!(autopilot.tick(&runtime, &mut control).is_none());
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Failed);
    assert_eq!(fixture.store.snapshot().unwrap().tasks.len(), tasks);
    fixed.store(true, Ordering::SeqCst);
    owner_repair(&mut control, &runtime, 3, "return 42 please");
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "repaired",
        |autopilot, control| {
            autopilot
                .status(control)
                .is_some_and(|status| status.state == RunState::Done)
        },
    );
}

#[test]
fn cancelled_owner_repairs_do_not_pile_up_automatic_repairs() {
    let fixture = Fixture::new();
    let fixed = Arc::new(AtomicBool::new(false));
    let server = repair_recovery_server(fixed);
    let runtime = Runtime::new().unwrap();
    let (mut autopilot, mut control) = failed_two_task_run(&fixture, &server, &runtime);
    assert_eq!(owner_repair(&mut control, &runtime, 3, "first hint"), 4);
    // Re-enter and approve the repair, but never launch it, then cancel it unstarted.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        control.poll();
        if let Some(submission) = autopilot.tick(&runtime, &mut control) {
            let _ = control.dispatch_prepared(&runtime, &submission.intent);
        }
        let status = &control.snapshot.as_ref().unwrap().tasks[3].status;
        if *status == TaskStatus::Approved {
            break;
        }
        assert!(Instant::now() < deadline, "repair approved: {status:?}");
        thread::sleep(Duration::from_millis(3));
    }
    let intent = control
        .prepare_command("/cancel-task 4", "fixture")
        .unwrap()
        .unwrap();
    control.dispatch_prepared(&runtime, &intent).unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "refinished",
        |autopilot, control| {
            control.snapshot.as_ref().unwrap().tasks[3].status == TaskStatus::Cancelled
                && finished(autopilot, control)
        },
    );
    assert_eq!(owner_repair(&mut control, &runtime, 3, "second hint"), 5);
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "refailed",
        |autopilot, control| {
            control.snapshot.as_ref().unwrap().tasks[4].status == TaskStatus::Failed
                && finished(autopilot, control)
                && autopilot
                    .report()
                    .is_some_and(|report| report.contains("Last failure:"))
        },
    );
    for _ in 0..50 {
        control.poll();
        assert!(autopilot.tick(&runtime, &mut control).is_none());
        thread::sleep(Duration::from_millis(2));
    }
    let report = autopilot.report().unwrap();
    assert!(report.contains("repair budget exhausted (2/2)"), "{report}");
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks.len(),
        5,
        "the owner's two repairs are the only ones added"
    );
}

#[test]
fn repairs_outside_this_run_and_before_its_finish_do_not_reenter() {
    let fixture = Fixture::new();
    let fixed = Arc::new(AtomicBool::new(false));
    let server = repair_recovery_server(fixed);
    let runtime = Runtime::new().unwrap();
    let (mut autopilot, mut control) = failed_two_task_run(&fixture, &server, &runtime);
    // Everything the run repaired itself (#3) predates its finish.
    for _ in 0..50 {
        control.poll();
        assert!(autopilot.tick(&runtime, &mut control).is_none());
        thread::sleep(Duration::from_millis(2));
    }
    // A manual task of the owner's own, not adopted by the run, fails and is repaired.
    let manual = owner_followup(&fixture);
    observe(&mut control, &runtime, manual);
    let intent = control
        .prepare_command("/dispatch on", "fixture")
        .unwrap()
        .unwrap();
    control.dispatch_prepared(&runtime, &intent).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        control.poll();
        if let Ok(Some(request)) = control.prepare_dispatch() {
            let _ = control.dispatch_prepared(&runtime, &Intent::DispatchRun { request });
        }
        control.refresh_background(&runtime);
        let snapshot = control.snapshot.as_ref().unwrap();
        if snapshot
            .tasks
            .iter()
            .any(|t| t.id == manual && t.status == TaskStatus::Failed)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "manual task fails {:?} {}",
            snapshot
                .tasks
                .iter()
                .map(|t| (t.id, t.status.clone()))
                .collect::<Vec<_>>(),
            control.dispatch.enabled
        );
        thread::sleep(Duration::from_millis(5));
    }
    let repair = owner_repair(&mut control, &runtime, manual, "try again");
    for _ in 0..80 {
        control.poll();
        assert!(autopilot.tick(&runtime, &mut control).is_none());
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Failed);
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks.len() as u64, repair);
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .find(|t| t.id == repair)
            .unwrap()
            .status,
        TaskStatus::Proposed,
        "the run did not approve a repair outside it"
    );
}

#[test]
fn repair_after_an_environment_failure_does_not_resume_the_loop() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [
                {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["calc.py"], "check": check(
                    "import sys; sys.stderr.write('bwrap: No permissions to create new namespace\\n'); sys.exit(1)")}},
            ]})
            .to_string();
        }
        good_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 2, &control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "sandbox", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Failed);
    let repair = owner_repair(&mut control, &runtime, 1, "again");
    for _ in 0..80 {
        control.poll();
        assert!(autopilot.tick(&runtime, &mut control).is_none());
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Failed);
    assert!(autopilot
        .resume(&control)
        .unwrap_err()
        .contains("already finished"));
    let snapshot = fixture.store.snapshot().unwrap();
    let task = snapshot.tasks.iter().find(|t| t.id == repair).unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Proposed,
        "nothing was approved or run"
    );
}

/// A worker whose check dies the way a sandbox refused by the host does (the
/// check's own process prints Bubblewrap's setup error as its first stderr line),
/// driven through the real worker finalize, evidence and settle path.
fn sandbox_refused(max_repairs: u32) {
    sandbox_refused_with(max_repairs, |_| None);
}

#[test]
fn failed_run_shows_the_token_totals_line_when_the_server_reported_counts() {
    let (report, notice) = sandbox_refused_with(0, |request| {
        let (prompt, eval) = if planner(request) {
            (1000, 100)
        } else {
            (200, 50)
        };
        Some(json!({"prompt_eval_count": prompt, "eval_count": eval}))
    });
    assert!(report.contains("tokens 1.2k in / 150 out"), "{report}");
    assert!(notice.starts_with("Autopilot failed"), "{notice}");
    assert!(notice.contains("tokens 1.2k in / 150 out"), "{notice}");
    let (report, notice) = sandbox_refused_with(0, |_| None);
    assert!(
        !report.contains("tokens") && !notice.contains("tokens"),
        "{report}{notice}"
    );
}

fn sandbox_refused_with(
    max_repairs: u32,
    usage: impl Fn(&Value) -> Option<Value> + Send + Sync + 'static,
) -> (String, String) {
    let fixture = Fixture::new();
    let server = Server::with_usage(
        |request, _| {
            if planner(request) {
                return json!({"tasks": [
                {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["calc.py"], "check": check(
                    "import sys; sys.stderr.write('bwrap: No permissions to create new namespace\\n'); sys.exit(1)")}},
            ]})
            .to_string();
            }
            good_calc()
        },
        usage,
    );
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Make the answer 42", "fixture", max_repairs, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "sandbox", finished);
    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Failed);
    assert_eq!(status.repairs, 0);
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks.len(), 1, "no repair task is created");
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Failed);
    let evidence: alfredo_tui::worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    assert_eq!(
        evidence.failure_code.as_deref(),
        Some(alfredo_tui::execution::SANDBOX_UNAVAILABLE)
    );
    let report = autopilot.report().unwrap().to_string();
    assert!(
        report.contains("bwrap: No permissions to create new namespace"),
        "{report}"
    );
    assert!(report.contains("sudo apt install bubblewrap"), "{report}");
    assert!(!report.contains("repair budget exhausted"), "{report}");
    assert!(
        !server
            .prompts()
            .iter()
            .any(|prompt| prompt.contains("Repair")),
        "{:?}",
        server.prompts()
    );
    let notice = autopilot.take_finished_notice().unwrap();
    (report, notice)
}

#[test]
fn sandbox_refusal_stops_autopilot_without_repairs_and_quotes_the_cause() {
    sandbox_refused(2);
}

#[test]
fn sandbox_refusal_is_quoted_even_with_no_repair_budget() {
    sandbox_refused(0);
}

fn commit_files(fixture: &Fixture, files: &[(&str, usize)]) {
    for (name, bytes) in files {
        fs::write(fixture.workspace.join(name), "x = 1\n".repeat(bytes / 6)).unwrap();
    }
    git(&fixture.workspace, &["add", "."]);
    git(
        &fixture.workspace,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "large"],
    );
}

fn files_plan(files: &[&str]) -> String {
    json!({"tasks": [
        {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
         "dependencies": [], "policy": {"files": files, "check": check("print(1)")}},
    ]})
    .to_string()
}

#[test]
fn plan_approving_a_file_over_the_bound_is_rejected_at_lint_with_no_repairs() {
    let fixture = Fixture::new();
    commit_files(&fixture, &[("calc.py", 156_697)]);
    let server = Server::new(|request, _| {
        if planner(request) {
            return files_plan(&["calc.py"]);
        }
        good_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Make the answer 42", "fixture", 3, &control)
        .unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "too-large-lint",
        finished,
    );
    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Failed);
    assert_eq!(status.repairs, 0);
    let report = autopilot.report().unwrap();
    assert!(
        report.contains("calc.py is 154 KiB; workers can edit files up to 64 KiB. Split the file, or edit it by hand."),
        "{report}"
    );
    assert!(!report.contains("larger model"), "{report}");
    assert!(!report.contains("--max-repairs"), "{report}");
    assert_eq!(fixture.store.snapshot().unwrap().tasks.len(), 0);
}

fn oversized_approved_set(max_repairs: u32) {
    let fixture = Fixture::new();
    // Each file fits the per-file bound; together they pass the 96 KiB context bound,
    // which the planning lint does not see.
    commit_files(&fixture, &[("a.py", 50_000), ("b.py", 50_000)]);
    let server = Server::new(|request, _| {
        if planner(request) {
            return files_plan(&["a.py", "b.py"]);
        }
        good_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Make the answer 42", "fixture", max_repairs, &control)
        .unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "too-large-worker",
        finished,
    );
    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Failed);
    assert_eq!(status.repairs, 0);
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks.len(), 1, "no repair task is created");
    let evidence: alfredo_tui::worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    assert_eq!(
        evidence.failure_code.as_deref(),
        Some(alfredo_tui::worker::FILE_TOO_LARGE)
    );
    let report = autopilot.report().unwrap();
    assert!(report.contains("Approved files total"), "{report}");
    assert!(report.contains("Repairs   0"), "{report}");
    assert!(
        report.contains("Next      Approve fewer or smaller files, or edit them by hand."),
        "{report}"
    );
    assert!(
        autopilot.notice().contains("0 repairs"),
        "{}",
        autopilot.notice()
    );
    assert!(!report.contains("larger model"), "{report}");
    assert!(!report.contains("repair budget exhausted"), "{report}");
    assert!(
        !server.prompts().iter().any(|p| p.contains("Repair")),
        "{:?}",
        server.prompts()
    );
}

#[test]
fn oversized_approved_set_stops_autopilot_without_repairs_or_model_advice() {
    oversized_approved_set(3);
}

#[test]
fn oversized_approved_set_stops_cleanly_with_no_repair_budget() {
    oversized_approved_set(0);
}

fn model_unreachable(max_repairs: u32) {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            return files_plan(&["calc.py"]);
        }
        DROP_CONNECTION.into()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Make the answer 42", "fixture", max_repairs, &control)
        .unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "unreachable",
        finished,
    );
    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Failed);
    assert_eq!(status.repairs, 0);
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks.len(), 1, "no repair task is created");
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Failed);
    let evidence: alfredo_tui::worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    assert_eq!(
        evidence.failure_code.as_deref(),
        Some(alfredo_tui::worker::MODEL_UNREACHABLE)
    );
    let report = autopilot.report().unwrap().to_string();
    assert!(
        report.contains(&format!("Cannot reach Ollama at {}", server.endpoint)),
        "{report}"
    );
    assert!(report.contains("Repairs   0"), "{report}");
    assert!(
        report.contains("Next      Start Ollama, then /repair 1 retry"),
        "{report}"
    );
    assert!(!report.contains("larger model"), "{report}");
    assert!(!report.contains("--max-repairs"), "{report}");
    assert!(!report.contains("repair budget exhausted"), "{report}");
    assert!(
        autopilot.notice().contains("0 repairs") || !autopilot.notice().contains("repairs"),
        "{}",
        autopilot.notice()
    );
    assert_eq!(
        server.count(),
        2,
        "one planner request and one worker request, no repair attempts"
    );
}

#[test]
fn dropped_ollama_connection_stops_autopilot_without_repairs_and_says_to_start_ollama() {
    model_unreachable(3);
}

#[test]
fn dropped_ollama_connection_stops_cleanly_with_no_repair_budget() {
    model_unreachable(0);
}

#[test]
fn malformed_model_output_is_still_repaired_and_carries_no_failure_code() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [
                {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
            ]})
            .to_string();
        }
        if worker_prompt(request).contains("Repair") {
            good_calc()
        } else {
            "this is prose, not file edits".into()
        }
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Make the answer 42", "fixture", 2, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "repaired", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Done);
    assert_eq!(autopilot.status(&control).unwrap().repairs, 1);
    let evidence: alfredo_tui::worker::Evidence =
        serde_json::from_str(&fixture.store.evidence(1).unwrap()).unwrap();
    assert_eq!(evidence.failure_code, None);
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
fn pause_because_carries_the_specific_reason_in_notice_saved_state_and_status_panel() {
    let fixture = Fixture::new();
    let server = Server::new(|_, _| String::new());
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Answer only", "fixture", 2, &control)
        .unwrap();
    let reason = "Autopilot paused: command not recorded: Command history capacity reached; start another conversation";
    autopilot.pause_because(&mut control, reason.into());
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Paused);
    assert!(!control.dispatch.enabled);
    assert_eq!(autopilot.notice(), reason);
    let saved = fs::read_dir(fixture.directory())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("autopilot-") && name.ends_with(".json"))
        })
        .expect("saved autopilot state");
    let saved: Value = serde_json::from_slice(&fs::read(saved).unwrap()).unwrap();
    assert_eq!(saved["paused"], true);
    assert_eq!(saved["notice"], reason);
    autopilot
        .command(&runtime, &mut control, "/autopilot", "fixture", 2)
        .unwrap();
    let panel = control.autopilot_report.clone().unwrap();
    assert!(
        panel.contains("Command history capacity reached"),
        "{panel}"
    );
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

/// A planner reply held until released; later requests answer immediately.
fn slow_planning(release: std::sync::mpsc::Receiver<()>) -> Server {
    let gate = Mutex::new(release);
    Server::new(move |request, _| {
        if planner(request) {
            let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(10));
            return one_task_plan();
        }
        good_calc()
    })
}

fn planning_in_flight(control: &TaskControl, server: &Server) -> bool {
    server.count() == 1 && control.planner.active()
}

#[test]
fn stop_while_planning_cancels_the_planner_and_leaves_no_late_draft_so_go_starts() {
    let fixture = Fixture::new();
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let server = slow_planning(gate);
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("First", "fixture", 2, &control).unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "planning",
        |_, c| planning_in_flight(c, &server),
    );
    let notice = autopilot.stop(&runtime, &mut control);
    assert!(notice.contains("planning cancelled"), "{notice}");
    assert!(!control.planner.active());
    let _ = release.send(());
    // The late reply must not surface as a draft.
    for _ in 0..100 {
        control.poll();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(control.planner.checkpoint().is_none());
    assert!(control.planner.draft.is_none());
    autopilot
        .start("Second goal", "fixture", 2, &control)
        .expect("/go after a stopped planning phase starts");
}

#[test]
fn stop_while_planning_then_resume_plans_again_and_finishes() {
    let fixture = Fixture::new();
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let server = slow_planning(gate);
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("First", "fixture", 2, &control).unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "planning",
        |_, c| planning_in_flight(c, &server),
    );
    autopilot.stop(&runtime, &mut control);
    assert!(autopilot.resume(&control).unwrap().contains("resumed"));
    drop(release);
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Done);
}

#[test]
fn go_after_stop_with_the_autopilots_finished_draft_open_names_resume_and_plan_cancel() {
    let fixture = Fixture::new();
    let server = Server::new(move |request, _| {
        if planner(request) {
            return one_task_plan();
        }
        good_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("First", "fixture", 2, &control).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(submission) = autopilot.tick(&runtime, &mut control) {
            let _ = control.dispatch_prepared(&runtime, &submission.intent);
        }
        control.poll();
        if control.planner.checkpoint().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "{}", control.planner.notice);
        thread::sleep(Duration::from_millis(3));
    }
    autopilot.stop(&runtime, &mut control);
    let error = autopilot
        .start("Second", "fixture", 2, &control)
        .unwrap_err();
    assert!(
        error.contains("stopped autopilot's plan draft") && error.contains("/resume"),
        "{error}"
    );
}

#[test]
fn stop_leaves_a_users_own_plan_generation_running() {
    let fixture = Fixture::new();
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let server = slow_planning(gate);
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    control
        .command(&runtime, "/plan Make answer return 42", "fixture")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(server.count() == 1 && control.planner.active()) {
        control.poll();
        assert!(Instant::now() < deadline, "{}", control.planner.notice);
        thread::sleep(Duration::from_millis(3));
    }
    autopilot.stop(&runtime, &mut control);
    assert!(control.planner.active());
    let _ = release.send(());
    while control.planner.checkpoint().is_none() {
        control.poll();
        assert!(Instant::now() < deadline, "{}", control.planner.notice);
        thread::sleep(Duration::from_millis(3));
    }
}

#[test]
fn go_never_discards_a_users_own_plan_draft() {
    let fixture = Fixture::new();
    let server = Server::new(move |_, _| one_task_plan());
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    control
        .command(&runtime, "/plan Make answer return 42", "fixture")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.planner.checkpoint().is_none() {
        control.poll();
        assert!(Instant::now() < deadline, "{}", control.planner.notice);
        thread::sleep(Duration::from_millis(5));
    }
    let error = autopilot
        .start("Other", "fixture", 2, &control)
        .unwrap_err();
    assert!(error.contains("/plan-cancel"), "{error}");
    assert!(control.planner.checkpoint().is_some());
}

/// /stop cancels the running worker; /resume must run the same task again, not
/// repair it, and the cancel must not read as a failure or spend repair budget.
fn stopped_then_resumed(
    second_answer: fn() -> String,
    max_repairs: u32,
) -> (Fixture, Server, TaskControl, Autopilot, Runtime) {
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
        second_answer()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Answer", "fixture", max_repairs, &control)
        .unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "worker request",
        |_, _| server.count() == 2,
    );
    autopilot.stop(&runtime, &mut control);
    drop(release);
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "cancelled",
        |_, c| {
            c.workers.is_empty()
                && c.snapshot
                    .as_ref()
                    .is_some_and(|s| s.tasks[0].status == TaskStatus::Cancelled)
        },
    );
    // Paused after the cancel: not a failure, not a repair.
    let paused = autopilot.status(&control).unwrap();
    assert_eq!(paused.state, RunState::Paused);
    assert_eq!((paused.failed, paused.repairs), (0, 0), "{paused:?}");
    let message = autopilot.resume(&control).unwrap();
    assert!(message.contains("resumed"), "{message}");
    (fixture, server, control, autopilot, runtime)
}

#[test]
fn stop_then_resume_runs_the_cancelled_task_again_without_a_repair_or_failure() {
    let (fixture, _server, mut control, mut autopilot, runtime) =
        stopped_then_resumed(good_calc, 2);
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Done);
    assert_eq!(
        (status.done, status.total, status.failed, status.repairs),
        (1, 1, 0, 0)
    );
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks.len(), 1, "no Repair task is created");
    assert!(snapshot.tasks.iter().all(|task| task.repair_of.is_none()));
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Accepted);
    assert_eq!(starts(&snapshot, 1), 2, "a second run of the same task");
    assert!(snapshot.receipts.iter().any(|receipt| matches!(
        receipt.request.action,
        alfredo_tui::tasks::Action::Requeue { task: 1 }
    )));
    assert!(!snapshot.receipts.iter().any(|receipt| matches!(
        receipt.request.action,
        alfredo_tui::tasks::Action::Repair { .. }
    )));
}

#[test]
fn requeued_run_that_fails_its_check_still_spends_the_repair_budget() {
    let (fixture, _server, mut control, mut autopilot, runtime) = stopped_then_resumed(bad_calc, 1);
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    let snapshot = fixture.store.snapshot().unwrap();
    let repairs: Vec<_> = snapshot
        .tasks
        .iter()
        .filter(|task| task.repair_of.is_some())
        .collect();
    assert_eq!(
        repairs.len(),
        1,
        "budget of one repair, spent on the real failure"
    );
    assert_eq!(repairs[0].repair_of, Some(1));
    assert_eq!(
        starts(&snapshot, 1),
        2,
        "cancelled run, then the requeued run"
    );
    let status = autopilot.status(&control).unwrap();
    assert_eq!((status.failed, status.repairs), (1, 1), "{status:?}");
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
        max_repairs: 3,
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
        max_repairs: 3,
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

#[test]
fn worker_that_echoes_the_file_unchanged_is_never_accepted_even_when_the_check_passes() {
    let fixture = Fixture::new();
    let head = fixture.head();
    let server = Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [{"title": "Make answer return 42", "acceptance": ["answer() is 42"],
                "model": "fixture", "dependencies": [], "policy": {"files": ["calc.py"],
                "check": check("from calc import answer; answer()")}}]})
            .to_string();
        }
        files("calc.py", "def answer():\n    return 0\n")
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Make answer return 42", "fixture", 1, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failed", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Failed);
    let snapshot = fixture.store.snapshot().unwrap();
    assert!(snapshot
        .tasks
        .iter()
        .all(|task| !matches!(task.status, TaskStatus::ReviewReady | TaskStatus::Accepted)));
    assert_eq!(decisions(&snapshot, 1), 0, "nothing was reviewed");
    let repair = snapshot
        .tasks
        .iter()
        .find(|t| t.repair_of == Some(1))
        .unwrap();
    assert!(
        repair.title.contains("autopilot: No files changed:") && repair.title.contains("calc.py"),
        "{}",
        repair.title
    );
    let last = server.prompts().pop().unwrap();
    assert!(last.contains("previous attempt changed nothing"), "{last}");
    let report = autopilot.report().unwrap().to_string();
    assert!(report.contains("repair budget exhausted (1/1)"), "{report}");
    assert!(report.contains("No files changed:"), "{report}");
    assert!(
        report.contains("Next: Try a larger model, /repair 2 <hint>"),
        "{report}"
    );
    assert_eq!(fixture.head(), head);
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
    // The follow-up must really edit the accepted file: an unchanged answer fails (#350).
    let server = Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [
                {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
            ]})
            .to_string();
        }
        if worker_prompt(request).contains("keep answer at 42") {
            return files("calc.py", "def answer():\n    return 42  # kept\n");
        }
        good_calc()
    });
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

fn followup_server() -> Server {
    Server::new(|request, _| {
        if planner(request) {
            return json!({"tasks": [
                {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
                 "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
            ]})
            .to_string();
        }
        if worker_prompt(request).starts_with("Implement this task: Write notes") {
            files("notes.txt", "follow-up\n")
        } else {
            good_calc()
        }
    })
}

/// The owner's follow-up task: proposed, permitted and approved through the store.
fn owner_followup(fixture: &Fixture) -> u64 {
    let snapshot = fixture.store.snapshot().unwrap();
    let id = snapshot.tasks.iter().map(|task| task.id).max().unwrap() + 1;
    let mut revision = snapshot.revision;
    for (name, action) in [
        (
            "propose",
            Action::Propose {
                title: "Write notes".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        ),
        (
            "permit",
            Action::Permit {
                task: id,
                policy: alfredo_tui::tasks::WorkPolicy {
                    files: vec!["notes.txt".into()],
                    check: vec![
                        "/usr/bin/python3".into(),
                        "-B".into(),
                        "-c".into(),
                        "from pathlib import Path; assert Path('notes.txt').read_text()".into(),
                    ],
                },
            },
        ),
        ("approve", Action::Approve { task: id }),
    ] {
        fixture
            .store
            .transact(Request {
                correlation: format!("followup-{name}"),
                expected_revision: revision,
                action,
            })
            .unwrap();
        revision = fixture.store.snapshot().unwrap().revision;
    }
    id
}

/// The terminal adopts a follow-up it already read from the store.
fn observe(control: &mut TaskControl, runtime: &Runtime, task: u64) {
    control.refresh(runtime);
    let deadline = Instant::now() + Duration::from_secs(30);
    while control.pending
        || !control
            .snapshot
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .any(|t| t.id == task)
    {
        control.poll();
        assert!(Instant::now() < deadline, "refresh: {}", control.notice);
        thread::sleep(Duration::from_millis(1));
    }
}

fn integrating(autopilot: &Autopilot, control: &TaskControl) -> bool {
    autopilot
        .status(control)
        .is_some_and(|status| status.state == RunState::Finishing)
}

#[test]
fn follow_up_adopted_while_the_branch_is_built_is_not_lost() {
    let fixture = Fixture::new();
    let server = followup_server();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    // Stops after the integration job spawned, before autopilot ever polls it.
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "integrating",
        integrating,
    );
    let follow = owner_followup(&fixture);
    observe(&mut control, &runtime, follow);
    assert!(autopilot.adopt(follow));
    drive(&mut autopilot, &mut control, &runtime, "done", |a, c| {
        finished(a, c) && c.workers.is_empty()
    });
    let status = autopilot.status(&control).unwrap();
    assert_eq!(status.state, RunState::Done, "{status:?}");
    assert_eq!((status.done, status.total), (2, 2), "{status:?}");
    let branch = status.branch.expect("integration branch");
    assert!(branch.ends_with("-2"), "new integration round: {branch}");
    assert_eq!(
        git(
            &fixture.workspace,
            &["show", &format!("{branch}:notes.txt")]
        ),
        "follow-up"
    );
    assert_eq!(
        git(&fixture.workspace, &["show", &format!("{branch}:calc.py")]),
        "def answer():\n    return 42"
    );
    assert_eq!(starts(&fixture.store.snapshot().unwrap(), follow), 1);
}

#[test]
fn follow_up_adopted_while_integrating_survives_a_restart_before_the_job_completes() {
    let fixture = Fixture::new();
    let server = followup_server();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "integrating",
        integrating,
    );
    let follow = owner_followup(&fixture);
    assert!(autopilot.adopt(follow));
    drop(autopilot);
    drop(control);
    let mut control = self::control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Paused);
    autopilot.resume(&control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "done", |a, c| {
        finished(a, c) && c.workers.is_empty()
    });
    let status = autopilot.status(&control).unwrap();
    assert_eq!((status.done, status.total), (2, 2), "{status:?}");
    let branch = status.branch.expect("integration branch");
    assert_eq!(
        git(
            &fixture.workspace,
            &["show", &format!("{branch}:notes.txt")]
        ),
        "follow-up"
    );
}

fn peeked(fixture: &Fixture) -> Option<(&'static str, Option<(u32, u32)>)> {
    // A read-only peek polled from inside the drive loop: use the lock-free path so
    // the poll never competes with the workers for the store lock (#442).
    let directory = fixture.store.state_directory();
    let peek = autopilot::peek(&autopilot::state_path(directory, "default")).unwrap()?;
    Some((peek.word, peek.counts))
}

#[test]
fn saved_state_carries_done_and_total_for_read_only_peeks_of_other_missions() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            return two_task_plan();
        }
        if worker_prompt(request).starts_with("Implement this task: Add app") {
            app()
        } else {
            good_calc()
        }
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    assert_eq!(peeked(&fixture), None);
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    // No plan yet: no counts, the state word alone.
    assert_eq!(peeked(&fixture).map(|peek| peek.1), Some(None));
    let seen = std::cell::RefCell::new(vec![]);
    drive(&mut autopilot, &mut control, &runtime, "done", |_, _| {
        let now = peeked(&fixture).and_then(|peek| peek.1);
        if now.is_some() && seen.borrow().last() != Some(&now) {
            seen.borrow_mut().push(now);
        }
        now == Some((2, 2)) && peeked(&fixture).is_some_and(|peek| peek.0 == "done")
    });
    let seen = seen.into_inner();
    assert!(seen.contains(&Some((0, 2))), "{seen:?}");
    assert!(seen.contains(&Some((1, 2))), "{seen:?}");
    assert_eq!(seen.last(), Some(&Some((2, 2))), "{seen:?}");
}

#[test]
fn missing_model_404_stops_planning_after_one_request_and_names_the_model() {
    let fixture = Fixture::new();
    let server = ollama_fixture::serve(ollama_fixture::reserve(), |request, _| {
        match request.path.as_str() {
            "GET /api/tags" => ollama_fixture::running(&["qwen2.5-coder:14b"]),
            _ => ollama_fixture::Reply::Json(
                404,
                json!({"error": "model 'qwen3:14b' not found"}).to_string(),
            ),
        }
    });
    let runtime = Runtime::new().unwrap();
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(
        Ollama::new(&server.endpoint, Duration::from_secs(5))
            .unwrap()
            .with_retry_backoff(Duration::from_millis(5)),
    );
    control.refresh(&runtime);
    let deadline = Instant::now() + Duration::from_secs(5);
    while control.pending || control.scope_status.revision.is_none() {
        control.poll();
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(2));
    }
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Improve calc", "qwen3:14b", 2, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failure", finished);
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Failed);
    assert_eq!(server.count("POST /api/chat"), 1, "{:?}", server.paths());
    let notice = autopilot.notice();
    assert!(!notice.contains("Planning failed 3 times"), "{notice}");
    assert!(notice.contains("ollama pull qwen3:14b"), "{notice}");
}

fn one_task_plan() -> String {
    json!({"tasks": [
        {"title": "Make answer return 42", "acceptance": ["answer() returns 42"], "model": "fixture",
         "dependencies": [], "policy": {"files": ["calc.py"], "check": check("from calc import answer; assert answer() == 42")}},
    ]})
    .to_string()
}

#[test]
fn failed_summary_names_the_last_failure_reason_and_a_next_step() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        if planner(request) {
            one_task_plan()
        } else {
            "I would rather talk about it than write code.".into()
        }
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failed", finished);
    let report = autopilot.report().unwrap().to_string();
    assert!(report.contains("repair budget exhausted (1/1)"), "{report}");
    assert!(report.contains("Model returned no FILE blocks"), "{report}");
    assert!(
        report.contains("Try a larger model, /repair 2 <hint>, or raise --max-repairs"),
        "{report}"
    );
}

/// Run one failing check with a budget of one repair and return the report.
fn failed_check_report(check_command: Value) -> String {
    let fixture = Fixture::new();
    let server = Server::new(move |request, _| {
        if planner(request) {
            return json!({"tasks": [{"title": "Make answer return 42", "acceptance": ["answer() is 42"],
                "model": "fixture", "dependencies": [], "policy": {"files": ["calc.py"],
                "check": check_command.clone()}}]})
            .to_string();
        }
        bad_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.start("Answer", "fixture", 1, &control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failed", finished);
    autopilot.report().unwrap().to_string()
}

#[test]
fn last_failure_of_a_traceback_check_names_the_assertion_error() {
    let report = failed_check_report(check("from calc import answer; assert answer() == 42"));
    let line = report
        .lines()
        .find(|line| line.contains("Last failure:"))
        .unwrap_or_else(|| panic!("{report}"));
    assert!(line.contains("Check failed (exit 1)"), "{line}");
    assert!(line.ends_with("AssertionError"), "{line}");
}

#[test]
fn last_failure_of_a_pytest_style_check_shows_the_failed_line() {
    let report = failed_check_report(check(
        "import sys; print('FAILED tests/x.py::t - assert 1 == 2', file=sys.stderr); \
         print('1 failed in 0.1s', file=sys.stderr); sys.exit(1)",
    ));
    let line = report
        .lines()
        .find(|line| line.contains("Last failure:"))
        .unwrap_or_else(|| panic!("{report}"));
    assert!(
        line.ends_with("FAILED tests/x.py::t - assert 1 == 2"),
        "{line}"
    );
}

fn last_failure_text(report: &str) -> String {
    let line = report
        .lines()
        .find(|line| line.contains("Last failure:"))
        .unwrap_or_else(|| panic!("{report}"));
    line.split_once("Last failure:")
        .unwrap()
        .1
        .trim()
        .to_string()
}

#[test]
fn last_failure_keeps_the_no_change_prefix_and_is_bounded() {
    let report = failed_check_report(check(
        "import sys; sys.stderr.write('AssertionError: ' + 'x' * 400 + '\\n'); sys.exit(1)",
    ));
    let line = last_failure_text(&report);
    assert!(
        line.starts_with("No change from previous attempt"),
        "{line}"
    );
    assert!(line.chars().count() <= 160, "{line}");
    assert!(
        line.contains("… AssertionError: xxx"),
        "head cut first: {line}"
    );
    assert!(line.ends_with('…'), "{line}");
}

#[test]
fn last_failure_without_an_error_line_ends_with_the_last_output_line() {
    let report = failed_check_report(check(
        "import sys; print('starting'); print('boom'); sys.exit(1)",
    ));
    let line = last_failure_text(&report);
    assert!(line.contains("Check failed (exit 1)"), "{line}");
    assert!(line.ends_with("boom"), "{line}");
}

const KEY_LINE: &str = "AssertionError: answer was 41 not 42";

/// Run the failing key-line check with `repairs` repairs and draw the summary at
/// `width` columns; returns the report and the summary pane's rows (side pane cut off).
fn failed_key_line_summary(repairs: u32, width: u16) -> (String, Vec<String>) {
    let fixture = Fixture::new();
    let server = Server::new(move |request, _| {
        if planner(request) {
            return json!({"tasks": [{"title": "Make answer return 42", "acceptance": ["answer() is 42"],
                "model": "fixture", "dependencies": [], "policy": {"files": ["calc.py"],
                "check": check("import sys; sys.exit('AssertionError: answer was 41 not 42')")}}]})
            .to_string();
        }
        bad_calc()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Answer", "fixture", repairs, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failed", finished);
    let report = autopilot.report().unwrap().to_string();
    let app = alfredo_tui::model::App::new("fixture".into());
    control.set_visible(true);
    control.autopilot_report = Some(report.clone());
    (report, summary_pane_rows(&app, &control, width, 40))
}

/// The rows of the summary panel: everything right of the side pane, border and padding off.
fn summary_pane_rows(
    app: &alfredo_tui::model::App,
    control: &TaskControl,
    width: u16,
    height: u16,
) -> Vec<String> {
    let start = alfredo_tui::ui::pane_width(width) as usize + 2;
    render_rows(app, control, width, height)
        .iter()
        .map(|row| {
            row.chars()
                .skip(start)
                .collect::<String>()
                .trim_end_matches(['│', ' '])
                .to_string()
        })
        .collect()
}

/// The words of the panel rows from the one starting with `from`, joined across wraps.
fn wrapped_text(rows: &[String], from: &str) -> String {
    let at = rows
        .iter()
        .position(|row| row.trim_start().starts_with(from))
        .unwrap_or_else(|| panic!("{from}: {}", rows.join("\n")));
    let mut text = rows[at].trim_start().to_string();
    for row in &rows[at + 1..] {
        // Continuation rows hang under the label; a new row starts shallower.
        if !row.starts_with("        ") {
            break;
        }
        text.push(' ');
        text.push_str(row.trim());
    }
    text
}

#[test]
fn last_failure_row_wraps_so_the_whole_key_line_is_on_screen_at_100_columns() {
    let (report, rows) = failed_key_line_summary(0, 100);
    let screen = rows.join("\n");
    assert!(report.contains(KEY_LINE), "{report}");
    let text = wrapped_text(&rows, "Last failure:");
    assert!(text.ends_with(KEY_LINE), "{text}\n{screen}");
    assert!(
        rows.iter()
            .all(|row| !row.contains('…') || !row.contains("Last failure")),
        "{screen}"
    );
    // Continuation rows hang under the row, not at the pane's left edge.
    let at = rows
        .iter()
        .position(|row| row.contains("Last failure:"))
        .unwrap();
    assert!(rows[at + 1].starts_with("        "), "{screen}");
    assert!(!rows[at + 1].contains("Last failure"), "{screen}");
}

#[test]
fn no_change_prefix_does_not_push_the_key_line_off_screen_at_100_columns() {
    let (report, rows) = failed_key_line_summary(1, 100);
    let screen = rows.join("\n");
    assert!(
        report.contains("No change from previous attempt · Check failed"),
        "{report}"
    );
    let text = wrapped_text(&rows, "Last failure:");
    assert!(text.contains("No change from previous attempt"), "{text}");
    assert!(text.ends_with(KEY_LINE), "{text}\n{screen}");
}

#[test]
fn next_and_detail_rows_wrap_instead_of_clipping_at_100_columns() {
    let detail = "repair budget exhausted (2/2): the worker kept returning the same file and the check kept failing with the same message each time";
    let next = "Next: Try a larger model, /repair 4 <hint>, or raise --max-repairs after reading the retained run evidence for every attempt";
    let report = format!(
        "✗ Autopilot failed\nShip it\n\nTasks     0/1 accepted\nRepairs   2\nBranch    none\n\n✗ #1  Make answer return 42\n      {detail}\n      {next}"
    );
    let fixture = Fixture::new();
    let server = Server::new(|_, _| String::new());
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    control.set_visible(true);
    control.autopilot_report = Some(report);
    let app = alfredo_tui::model::App::new("fixture".into());
    let rows = summary_pane_rows(&app, &control, 100, 30);
    let screen = rows.join("\n");
    assert_eq!(
        wrapped_text(&rows, "repair budget exhausted"),
        detail,
        "{screen}"
    );
    assert_eq!(wrapped_text(&rows, "Next:"), next, "{screen}");
    assert!(rows.iter().all(|row| !row.ends_with('…')), "{screen}");
}

#[test]
fn narrow_summary_wraps_wide_characters_without_panicking() {
    let report = "✗ Autopilot failed\nShip it\n\nTasks     0/1 accepted\n\n✗ #1  界界\n      界界界界界界界界界界界界界界界界界界界界界界界界 failure 界界界界界界界界界界界界界\n      Next: 界界界界界界界界界界界界界界界界界界界界界界界界界界界界";
    let fixture = Fixture::new();
    let server = Server::new(|_, _| String::new());
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    control.set_visible(true);
    control.autopilot_report = Some(report.to_string());
    let app = alfredo_tui::model::App::new("fixture".into());
    for width in [20, 30, 41] {
        let rows = render_rows(&app, &control, width, 20);
        assert_eq!(rows.len(), 20);
    }
}

#[test]
fn planner_prose_is_reported_in_plain_words_without_serde_detail() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        assert!(planner(request));
        "I think we should just refactor everything lol".into()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot
        .start("Improve calc", "fixture", 2, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failure", finished);
    let notice = autopilot.notice().to_string();
    assert!(
        notice.contains(
            "The model did not return a valid plan (expected JSON). Try a larger model or /plan to inspect its answer"
        ),
        "{notice}"
    );
    assert!(!notice.contains("Invalid plan JSON"), "{notice}");
    assert!(!notice.contains("line 1 column 1"), "{notice}");
    // The retry feedback to the model keeps the parser detail.
    let prompts = server.prompts();
    assert_eq!(prompts.len(), 3, "{prompts:?}");
    assert!(
        prompts[1].contains("attempt 1: Invalid plan JSON"),
        "{}",
        prompts[1]
    );
    assert!(!prompts[1].contains("did not return a valid plan"));
    let report = autopilot.report().unwrap();
    assert!(report.contains("did not return a valid plan"), "{report}");
    // Failure leaves no stale draft panel in front of the summary.
    assert!(!control.planner.visible);
    assert!(control.autopilot_report.is_some());
}

#[test]
fn autopilot_command_after_a_planning_failure_shows_the_summary_not_the_stale_draft() {
    let fixture = Fixture::new();
    fixture.store.select_mission(true).unwrap();
    let server = Server::new(|request, _| {
        assert!(planner(request));
        "I think we should just refactor everything lol".into()
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
    work.autopilot
        .start("Improve calc", "fixture", 2, &work.tasks)
        .unwrap();
    work.sync_autopilot();
    drive(
        &mut work.autopilot,
        &mut work.tasks,
        &runtime,
        "failed",
        finished,
    );
    work.sync_autopilot();
    // /autopilot shows the summary, not the stale "Plan draft" panel.
    work.autopilot
        .command(&runtime, &mut work.tasks, "/autopilot", "fixture", 2)
        .unwrap();
    let screen = render_rows(&work.app, &work.tasks, 120, 30).join("\n");
    assert!(screen.contains("Autopilot failed"), "{screen}");
    assert!(!screen.contains("Plan draft"), "{screen}");
    assert!(screen.contains("did not return a valid plan"), "{screen}");
}

#[test]
fn repeated_plan_failures_collapse_to_one_counted_chat_line() {
    use alfredo_tui::{
        command_intent::Intent, console_command::CommandState, planner::Planner,
        planner_command::Outcome,
    };
    let fixture = Fixture::new();
    let control = TaskControl::new(fixture.store.clone());
    let planner = Planner::default();
    let mut app = alfredo_tui::model::App::new("fixture".into());
    let snapshot = fixture.store.snapshot().unwrap();
    for generation in ["g1", "g2", "g3"] {
        let request = planner
            .prepare_command(generation, "/plan Improve calc", "fixture", &snapshot)
            .unwrap()
            .unwrap();
        let id = app.sessions[0]
            .submit_command("Autopilot · plan".into(), Intent::Planner { request })
            .unwrap();
        app.sessions[0].set_command_state(
            &id,
            CommandState::Planner {
                outcome: Outcome::failed("Invalid plan JSON: expected value at line 1 column 1"),
            },
        );
    }
    let chat = render_rows(&app, &control, 160, 40).join("\n");
    assert_eq!(chat.matches("plan failed:").count(), 1, "{chat}");
    assert!(chat.contains("did not return a valid plan"), "{chat}");
    assert!(chat.contains("×3"), "{chat}");
    assert!(!chat.contains("Invalid plan JSON"), "{chat}");
}

#[test]
fn manual_repeated_plan_failures_are_not_collapsed() {
    use alfredo_tui::{
        command_intent::Intent, console_command::CommandState, planner::Planner,
        planner_command::Outcome,
    };
    let fixture = Fixture::new();
    let control = TaskControl::new(fixture.store.clone());
    let planner = Planner::default();
    let mut app = alfredo_tui::model::App::new("fixture".into());
    let snapshot = fixture.store.snapshot().unwrap();
    for generation in ["m1", "m2"] {
        let request = planner
            .prepare_command(generation, "/plan Improve calc", "fixture", &snapshot)
            .unwrap()
            .unwrap();
        let id = app.sessions[0]
            .submit_command("/plan Improve calc".into(), Intent::Planner { request })
            .unwrap();
        app.sessions[0].set_command_state(
            &id,
            CommandState::Planner {
                outcome: Outcome::failed("Invalid plan JSON: expected value"),
            },
        );
    }
    let chat = render_rows(&app, &control, 160, 40).join("\n");
    assert!(!chat.contains("×2"), "{chat}");
    assert_eq!(chat.matches("/plan Improve calc").count(), 2, "{chat}");
}

const START_NOTICE: &str = "Autopilot started · plan → approve → dispatch → review/repair (max 1 repairs per task) · F5 pauses";

#[test]
fn clean_working_tree_gets_the_unchanged_start_notice_and_no_warning() {
    let fixture = Fixture::new();
    // An ignored file is not an uncommitted change.
    fs::write(fixture.workspace.join(".gitignore"), "*.log\n").unwrap();
    git(&fixture.workspace, &["add", ".gitignore"]);
    git(
        &fixture.workspace,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "ignore"],
    );
    fs::write(fixture.workspace.join("debug.log"), "noise\n").unwrap();
    let server = one_good_task();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    let notice = autopilot.start("Answer", "fixture", 1, &control).unwrap();
    assert_eq!(notice, START_NOTICE);
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    assert!(
        !autopilot.report().unwrap().contains("uncommitted"),
        "{}",
        autopilot.report().unwrap()
    );
}

#[test]
fn dirty_working_tree_warns_at_start_and_in_the_summary_without_touching_it() {
    let fixture = Fixture::new();
    fs::write(fixture.workspace.join(".gitignore"), "*.log\n").unwrap();
    git(&fixture.workspace, &["add", ".gitignore"]);
    git(
        &fixture.workspace,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "ignore"],
    );
    let head = fixture.head();
    fs::write(
        fixture.workspace.join("calc.py"),
        "def answer():\n    return 1\n",
    )
    .unwrap();
    fs::write(fixture.workspace.join("new.txt"), "mine\n").unwrap();
    fs::write(fixture.workspace.join("debug.log"), "ignored\n").unwrap();
    let before = fixture.status();
    let server = one_good_task();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    let warning = format!(
        "2 uncommitted changes are not visible to workers (based on HEAD {})",
        &head[..7]
    );
    // Informational only: /go still starts.
    let notice = autopilot.start("Answer", "fixture", 1, &control).unwrap();
    assert_eq!(notice, format!("{START_NOTICE} · {warning}"));
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    let report = autopilot.report().unwrap();
    assert!(
        report
            .lines()
            .any(|line| line == format!("Warning   {warning}")),
        "{report}"
    );
    // Read-only: the user's tree and index are exactly as they were.
    assert_eq!(fixture.status(), before);
    assert_eq!(fixture.head(), head);
    // The warning survives a restart (persisted state).
    let reopened = Autopilot::open(&fixture.directory(), "default").unwrap();
    assert!(reopened.report().unwrap().contains(&warning));
}

#[derive(Clone, Copy, PartialEq)]
enum Loop {
    None,
    Running,
    Paused,
    Finished,
    Failed,
}

/// /stop, /pause and /resume must tell one story in every autopilot state:
/// "/resume continues" only where /resume really continues the loop.
#[test]
fn stop_pause_and_resume_agree_across_autopilot_states() {
    for state in [
        Loop::None,
        Loop::Running,
        Loop::Paused,
        Loop::Finished,
        Loop::Failed,
    ] {
        let fixture = Fixture::new();
        let server = match state {
            Loop::Failed => Server::new(|_, _| {
                json!({"tasks": [{"title": "Bad", "acceptance": ["x"], "model": "fixture", "dependencies": [2],
                    "policy": {"files": ["calc.py"], "check": ["true"]}}]})
                .to_string()
            }),
            _ => one_good_task(),
        };
        let runtime = Runtime::new().unwrap();
        let mut control = control(&fixture, &server, &runtime);
        let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
        if state != Loop::None {
            autopilot.start("Answer", "fixture", 2, &control).unwrap();
        }
        if matches!(state, Loop::Finished | Loop::Failed) {
            drive(&mut autopilot, &mut control, &runtime, "end", finished);
        }
        if state == Loop::Failed {
            assert_eq!(autopilot.status(&control).unwrap().state, RunState::Failed);
        }
        if state == Loop::Finished {
            assert_eq!(autopilot.status(&control).unwrap().state, RunState::Done);
        }
        let say = |autopilot: &mut Autopilot, control: &mut TaskControl, text: &str| {
            autopilot
                .command(&runtime, control, text, "fixture", 2)
                .unwrap_or_else(|error| error)
        };
        if state == Loop::Paused {
            say(&mut autopilot, &mut control, "/pause");
        }
        let pause = say(&mut autopilot, &mut control, "/pause");
        if state == Loop::Running {
            say(&mut autopilot, &mut control, "/resume");
        }
        let stop = say(&mut autopilot, &mut control, "/stop");
        let resume = say(&mut autopilot, &mut control, "/resume");
        let active = matches!(state, Loop::Running | Loop::Paused);
        if active {
            assert!(pause.starts_with("Autopilot paused"), "{pause}");
            assert_eq!(
                stop,
                "Autopilot stopped · cancellation requested for 0 worker(s) · /resume continues"
            );
            assert_eq!(resume, "Autopilot resumed");
        } else {
            assert_eq!(pause, "No active autopilot");
            assert_eq!(stop, "No active autopilot");
            assert_eq!(
                resume,
                if state == Loop::None {
                    "No autopilot goal; use /go GOAL"
                } else {
                    "Autopilot already finished; /go GOAL starts a new loop"
                }
            );
        }
    }
}

/// Without an active autopilot, /stop still cancels running workers (here a
/// worker started by an autopilot that this instance knows nothing about, so it
/// is indistinguishable from a manual one) and must not claim autopilot stopped.
#[test]
fn stop_without_autopilot_still_cancels_running_workers() {
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
    let mut starter = Autopilot::open(&fixture.directory(), "default").unwrap();
    starter.start("Answer", "fixture", 2, &control).unwrap();
    drive(
        &mut starter,
        &mut control,
        &runtime,
        "worker request",
        |_, _| server.count() == 2,
    );
    let other = Fixture::new();
    let mut idle = Autopilot::open(&other.directory(), "default").unwrap();
    assert!(!control.workers.is_empty());
    let notice = idle.stop(&runtime, &mut control);
    assert_eq!(
        notice,
        "No active autopilot · cancellation requested for 1 worker(s)"
    );
    assert!(control
        .workers
        .values()
        .all(|flag| flag.load(Ordering::SeqCst)));
    drop(release);
    drive(&mut starter, &mut control, &runtime, "cancelled", |_, c| {
        c.workers.is_empty()
    });
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks[0].status,
        TaskStatus::Cancelled
    );
}

fn plan_server() -> Server {
    Server::new(|request, _| {
        if planner(request) {
            return two_task_plan();
        }
        let prompt = worker_prompt(request);
        if prompt.starts_with("Implement this task: Make answer") {
            good_calc()
        } else if prompt.starts_with("Implement this task: Add app") {
            app()
        } else {
            panic!("Unexpected prompt {prompt}")
        }
    })
}

fn planner_requests(server: &Server) -> usize {
    server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| planner(request))
        .count()
}

fn open_draft(control: &mut TaskControl, runtime: &Runtime) {
    control
        .command(
            runtime,
            "/plan Make answer return 42 and add app",
            "fixture",
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.planner.checkpoint().is_none() {
        control.poll();
        assert!(Instant::now() < deadline, "{}", control.planner.notice);
        thread::sleep(Duration::from_millis(5));
    }
}

fn assert_ran_and_integrated(autopilot: &Autopilot, control: &TaskControl, fixture: &Fixture) {
    let status = autopilot.status(control).unwrap();
    assert_eq!(status.state, RunState::Done, "{status:?}");
    assert_eq!((status.done, status.total, status.failed), (2, 2, 0));
    assert!(status.branch.as_deref().unwrap().starts_with("alfredo/go-"));
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks.len(), 2, "no duplicate tasks");
    assert!(snapshot
        .tasks
        .iter()
        .all(|task| task.status == TaskStatus::Accepted));
}

#[test]
fn bare_go_saves_the_open_draft_unchanged_and_runs_it_without_a_planner_request() {
    let fixture = Fixture::new();
    let server = plan_server();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    open_draft(&mut control, &runtime);
    assert_eq!(planner_requests(&server), 1);

    let error = autopilot
        .command(&runtime, &mut control, "/go Other goal", "fixture", 2)
        .unwrap_err();
    assert!(error.contains("/go with no goal"), "{error}");
    assert!(control.planner.checkpoint().is_some());
    assert!(autopilot.status(&control).is_none());

    let notice = autopilot
        .command(&runtime, &mut control, "/go", "fixture", 2)
        .unwrap();
    assert!(notice.contains("reviewed plan"), "{notice}");
    assert_eq!(
        autopilot.status(&control).unwrap().goal,
        "Make answer return 42 and add app"
    );
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "draft run",
        finished,
    );
    assert_ran_and_integrated(&autopilot, &control, &fixture);
    assert_eq!(planner_requests(&server), 1, "no second planner request");
}

#[test]
fn bare_go_runs_saved_proposed_tasks_without_replanning_or_orphaning_them() {
    let fixture = Fixture::new();
    let server = plan_server();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    open_draft(&mut control, &runtime);
    control.command(&runtime, "/plan-save", "fixture").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.planner.checkpoint().is_some()
        || control.snapshot.as_ref().is_none_or(|s| s.tasks.len() < 2)
    {
        control.poll();
        control.refresh(&runtime);
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(5));
    }

    autopilot
        .command(&runtime, &mut control, "/go", "fixture", 2)
        .unwrap();
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "saved run",
        finished,
    );
    assert_ran_and_integrated(&autopilot, &control, &fixture);
    assert_eq!(planner_requests(&server), 1);
    // Nothing is left to adopt once the saved tasks have run.
    let error = autopilot
        .command(&runtime, &mut control, "/go", "fixture", 2)
        .unwrap_err();
    assert!(
        error.starts_with("Usage: /go GOAL, or /go to run the open plan"),
        "{error}"
    );
}

#[test]
fn bare_go_with_nothing_to_run_says_usage() {
    let fixture = Fixture::new();
    let server = plan_server();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    let error = autopilot
        .command(&runtime, &mut control, "/go", "fixture", 2)
        .unwrap_err();
    assert!(
        error.starts_with("Usage: /go GOAL, or /go to run the open plan"),
        "{error}"
    );
    assert!(autopilot.status(&control).is_none());
}

#[test]
fn bare_go_refuses_a_plan_that_fails_unattended_lint_and_leaves_it_untouched() {
    let fixture = Fixture::new();
    let server = Server::new(|_, _| {
        // Both tasks write calc.py with no dependency between them.
        json!({"tasks": [
            {"title": "One", "acceptance": ["a"], "model": "fixture", "dependencies": [],
             "policy": {"files": ["calc.py"], "check": check("import calc")}},
            {"title": "Two", "acceptance": ["b"], "model": "fixture", "dependencies": [],
             "policy": {"files": ["calc.py"], "check": check("import calc")}},
        ]})
        .to_string()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    open_draft(&mut control, &runtime);
    let before = control.planner.checkpoint().unwrap();
    let error = autopilot
        .command(&runtime, &mut control, "/go", "fixture", 2)
        .unwrap_err();
    assert!(error.contains("both write calc.py"), "{error}");
    assert!(error.contains("nothing changed"), "{error}");
    assert_eq!(control.planner.checkpoint().unwrap().plan, before.plan);
    assert!(autopilot.status(&control).is_none());

    // The same plan saved by hand is refused the same way and stays proposed.
    control.command(&runtime, "/plan-save", "fixture").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.snapshot.as_ref().is_none_or(|s| s.tasks.len() < 2) {
        control.poll();
        control.refresh(&runtime);
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(5));
    }
    let error = autopilot
        .command(&runtime, &mut control, "/go", "fixture", 2)
        .unwrap_err();
    assert!(error.contains("both write calc.py"), "{error}");
    let snapshot = fixture.store.snapshot().unwrap();
    assert!(snapshot
        .tasks
        .iter()
        .all(|task| task.status == TaskStatus::Proposed));
    assert!(autopilot.status(&control).is_none());
}

#[test]
fn go_goal_with_an_open_draft_names_bare_go_as_the_way_to_run_it() {
    let fixture = Fixture::new();
    let server = plan_server();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    open_draft(&mut control, &runtime);
    let error = autopilot
        .start("Another goal", "fixture", 2, &control)
        .unwrap_err();
    assert!(
        error.contains("/go with no goal runs it") && error.contains("/plan-cancel"),
        "{error}"
    );
    assert!(control.planner.checkpoint().is_some());
    assert!(autopilot.status(&control).is_none());
}

#[test]
fn bare_go_refuses_saved_tasks_that_depend_on_unaccepted_work_outside_the_run() {
    let fixture = Fixture::new();
    let server = plan_server();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    open_draft(&mut control, &runtime);
    control.command(&runtime, "/plan-save", "fixture").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.snapshot.as_ref().is_none_or(|s| s.tasks.len() < 2) {
        control.poll();
        control.refresh(&runtime);
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(5));
    }
    while control
        .command(&runtime, "/cancel-task 1", "fixture")
        .is_err()
    {
        control.poll();
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(5));
    }
    while control.snapshot.as_ref().is_none_or(|s| {
        s.tasks
            .iter()
            .any(|task| task.id == 1 && task.status != TaskStatus::Cancelled)
    }) {
        control.poll();
        control.refresh(&runtime);
        assert!(Instant::now() < deadline, "{}", control.notice);
        thread::sleep(Duration::from_millis(5));
    }
    let error = autopilot
        .command(&runtime, &mut control, "/go", "fixture", 2)
        .unwrap_err();
    assert!(
        error.contains("#2 depends on #1") && error.contains("nothing changed"),
        "{error}"
    );
    assert!(autopilot.status(&control).is_none());
    let snapshot = fixture.store.snapshot().unwrap();
    assert_eq!(snapshot.tasks[1].status, TaskStatus::Proposed);
}

fn hook_script(fixture: &Fixture, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = fixture.root.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn wait_for_lines(path: &Path, count: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = fs::read_to_string(path).unwrap_or_default();
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        if lines.len() >= count || Instant::now() > deadline {
            return lines;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn on_finish_hook_runs_once_with_the_run_summary_when_the_run_is_done() {
    let fixture = Fixture::new();
    let server = one_good_task();
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let out = fixture.root.join("hook.out");
    let hook = hook_script(
        &fixture,
        "hook.sh",
        &format!(
            "echo \"$ALFREDO_STATE|$ALFREDO_BRANCH|$ALFREDO_GOAL|$ALFREDO_ACCEPTED|$ALFREDO_TOTAL|$ALFREDO_REPAIRS\" >> {}",
            out.display()
        ),
    );
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.set_on_finish(Some(hook.clone()));
    autopilot
        .start("Answer 42", "fixture", 1, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "done", finished);
    let branch = autopilot.status(&control).unwrap().branch.unwrap();
    let lines = wait_for_lines(&out, 1);
    assert_eq!(lines, [format!("done|{branch}|Answer 42|1|1|0")]);
    // Further ticks, and a restart that re-renders the finished run, do not re-fire.
    for _ in 0..20 {
        autopilot.tick(&runtime, &mut control);
        thread::sleep(Duration::from_millis(5));
    }
    let mut restarted = Autopilot::open(&fixture.directory(), "default").unwrap();
    restarted.set_on_finish(Some(hook));
    for _ in 0..20 {
        restarted.tick(&runtime, &mut control);
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(300));
    assert_eq!(wait_for_lines(&out, 2).len(), 1);
}

#[test]
fn on_finish_hook_failure_shows_a_notice_and_leaves_the_failed_run_unchanged() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        assert!(planner(request));
        json!({"tasks": [{"title": "Bad", "acceptance": ["x"], "model": "fixture", "dependencies": [2],
            "policy": {"files": ["calc.py"], "check": ["true"]}}]})
        .to_string()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let out = fixture.root.join("hook.out");
    let hook = hook_script(
        &fixture,
        "hook.sh",
        &format!(
            "echo \"$ALFREDO_STATE|$ALFREDO_ACCEPTED|$ALFREDO_TOTAL\" >> {}\nexit 3",
            out.display()
        ),
    );
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.set_on_finish(Some(hook));
    autopilot
        .start("Improve calc", "fixture", 2, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failure", finished);
    assert_eq!(wait_for_lines(&out, 1), ["failed|0|0"]);
    // The result notice comes first, then the hook's own notice.
    let mut notices = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !notices.iter().any(|n: &String| n.contains("--on-finish")) {
        notices.extend(autopilot.take_finished_notice());
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        notices
            .iter()
            .any(|n| n.contains("--on-finish hook failed")),
        "{notices:?}"
    );
    assert_eq!(autopilot.status(&control).unwrap().state, RunState::Failed);
}

#[test]
fn finish_hooks_waits_for_a_slow_hook_and_returns_the_failure() {
    let fixture = Fixture::new();
    let server = Server::new(|request, _| {
        assert!(planner(request));
        json!({"tasks": [{"title": "Bad", "acceptance": ["x"], "model": "fixture", "dependencies": [2],
            "policy": {"files": ["calc.py"], "check": ["true"]}}]})
        .to_string()
    });
    let runtime = Runtime::new().unwrap();
    let mut control = control(&fixture, &server, &runtime);
    let out = fixture.root.join("slow.out");
    let hook = hook_script(
        &fixture,
        "slow.sh",
        &format!("sleep 1\necho late > {}\nexit 4", out.display()),
    );
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.set_on_finish(Some(hook));
    autopilot
        .start("Improve calc", "fixture", 2, &control)
        .unwrap();
    drive(&mut autopilot, &mut control, &runtime, "failure", finished);
    let failures = autopilot.finish_hooks(Duration::from_secs(15));
    assert!(
        out.exists(),
        "the hook completed before finish_hooks returned"
    );
    assert_eq!(failures, ["--on-finish hook failed (exit status: 4)"]);
    assert!(autopilot.finish_hooks(Duration::from_secs(1)).is_empty());
}

#[test]
fn on_finish_hook_is_not_run_by_stop() {
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
    let out = fixture.root.join("hook.out");
    let hook = hook_script(
        &fixture,
        "hook.sh",
        &format!("echo fired >> {}", out.display()),
    );
    let mut autopilot = Autopilot::open(&fixture.directory(), "default").unwrap();
    autopilot.set_on_finish(Some(hook));
    autopilot.start("Answer", "fixture", 2, &control).unwrap();
    drive(&mut autopilot, &mut control, &runtime, "worker", |_, _| {
        server.count() == 2
    });
    autopilot.stop(&runtime, &mut control);
    drop(release);
    drive(
        &mut autopilot,
        &mut control,
        &runtime,
        "cancelled",
        |_, c| c.workers.is_empty(),
    );
    thread::sleep(Duration::from_millis(300));
    assert!(!out.exists());
}
