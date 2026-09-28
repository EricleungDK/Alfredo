//! Saved controller requests bind process ownership; only canonical Finish proves worker outcomes.
use alfredo_tui::{
    command_intent::Intent,
    console_command::CommandState,
    control_command::{Operation, Outcome},
    conversations::{Autosave, ConversationStore},
    model::App,
    provider::Ollama,
    task_control::TaskControl,
    tasks::{Action, Request, TaskStatus, TaskStore, WorkPolicy},
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
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
            "alfredo-control-command-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
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
        fs::write(workspace.join("calc.py"), "def answer():\n    return 0\n").unwrap();
        for args in [
            vec!["add", "calc.py"],
            vec!["-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture"],
        ] {
            assert!(Command::new("git")
                .arg("-C")
                .arg(&workspace)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        let store = TaskStore::new(&root.join("state"), &workspace, "mission").unwrap();
        for action in [
            Action::Propose {
                title: "Return 42".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
            Action::Permit {
                task: 1,
                policy: WorkPolicy {
                    files: vec!["calc.py".into()],
                    check: vec![
                        "/usr/bin/python3".into(),
                        "-B".into(),
                        "-c".into(),
                        "from calc import answer; assert answer()==42".into(),
                    ],
                },
            },
            Action::Approve { task: 1 },
        ] {
            let revision = store.snapshot().unwrap().revision;
            store
                .transact(Request {
                    correlation: format!("setup-{revision}"),
                    expected_revision: revision,
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
fn server(
    success: bool,
) -> (
    Ollama,
    thread::JoinHandle<()>,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Ollama::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        Duration::from_secs(5),
    )
    .unwrap();
    let (seen, received) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let job = thread::spawn(move || {
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
                Err(error) => panic!("No worker HTTP request: {error}"),
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
        stream.read_exact(&mut body).unwrap();
        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(request["model"], "fixture");
        seen.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
        let plan = serde_json::json!({"files":[{"path":"calc.py","content":if success { "def answer():\n    return 42\n" } else { "def answer():\n    return 9\n" }}]});
        let response = format!(
            "{}\n",
            serde_json::json!({"message":{"content":plan.to_string()},"done":true})
        );
        let _=stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes());
    });
    (provider, job, received, release)
}

fn control(fixture: &Fixture, provider: Ollama) -> TaskControl {
    let mut control = TaskControl::new(fixture.store.clone());
    control.snapshot = Some(fixture.store.snapshot().unwrap());
    control.observe_scope(fixture.store.understanding().snapshot().unwrap());
    control.set_provider(provider);
    control
}
fn wait_intent(control: &mut TaskControl, intent: &Intent) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while control.intent_pending(intent) {
        control.poll();
        assert!(
            Instant::now() < deadline,
            "Controller acknowledgment timed out"
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn wait_workers(control: &mut TaskControl) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !control.workers.is_empty() {
        control.poll();
        assert!(Instant::now() < deadline, "Worker completion timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
fn prepared(control: &mut TaskControl, text: &str) -> Intent {
    control
        .prepare_command(text, "fixture")
        .unwrap()
        .expect("Saved control intent")
}
fn unavailable_provider() -> Ollama {
    Ollama::new("http://127.0.0.1:9", Duration::from_secs(1)).unwrap()
}

#[test]
fn worker_cancel_waits_for_saved_intent_and_exact_owner_then_finishes_at_its_origin() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (provider, job, seen, release) = server(true);
    let mut control = control(&fixture, provider);
    let run = prepared(&mut control, "/run 1");
    control.dispatch_prepared(&runtime, &run).unwrap();
    seen.recv_timeout(Duration::from_secs(10)).unwrap();
    control.poll();
    let started = fixture.store.snapshot().unwrap();
    let cancel = prepared(&mut control, "/cancel-task 1");
    let Intent::Control { request } = &cancel else {
        panic!("Expected control request")
    };
    assert!(
        matches!(&request.operation, Operation::CancelWorker { task: 1, start_correlation, expected_start_revision } if start_correlation == run.correlation() && *expected_start_revision == 4)
    );
    assert!(!control.workers[&1].load(Ordering::SeqCst));
    assert!(cancel.reconcile(Some(&started), None).is_none());
    assert_eq!(cancel.task_receipts(Some(&started)).len(), 1);
    let mut app = App::new("fixture".into());
    let mut autosave = Autosave::new(ConversationStore::open(&fixture.store, "default").unwrap());
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    let path = fs::read_dir(fixture.store.conversation_directory().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("conversations-")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let original = fs::read(&path).unwrap();
    let id = app.sessions[0]
        .submit_command("/cancel-task 1".into(), cancel.clone())
        .unwrap();
    app.add_session();
    app.sessions[1].insert("newer draft remains");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(autosave
        .finish(&runtime, &app, Default::default(), None)
        .is_err());
    assert!(!autosave.contains_saved_command(0, &app.sessions[0].commands()[0]));
    assert!(!control.workers[&1].load(Ordering::SeqCst));
    assert_eq!(
        serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap(),
        serde_json::to_value(&started).unwrap()
    );
    fs::remove_dir(&path).unwrap();
    fs::write(&path, original).unwrap();
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    assert!(autosave.contains_saved_command(0, &app.sessions[0].commands()[0]));
    let mut wrong_owner = cancel.clone();
    let Intent::Control { request: wrong } = &mut wrong_owner else {
        unreachable!()
    };
    wrong.controller.push_str("-other");
    assert!(control.dispatch_prepared(&runtime, &wrong_owner).is_err());
    let mut wrong_start = cancel.clone();
    let Intent::Control { request: wrong } = &mut wrong_start else {
        unreachable!()
    };
    let Operation::CancelWorker {
        start_correlation, ..
    } = &mut wrong.operation
    else {
        unreachable!()
    };
    start_correlation.push_str("-other");
    assert!(control.dispatch_prepared(&runtime, &wrong_start).is_err());
    assert!(!control.workers[&1].load(Ordering::SeqCst));
    assert!(control.take_control_events().is_empty());
    control.dispatch_prepared(&runtime, &cancel).unwrap();
    assert!(control.workers[&1].load(Ordering::SeqCst));
    let events = control.take_control_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].request, *request);
    assert_eq!(events[0].outcome, Outcome::CancellationRequested);
    app.sessions[0].set_command_state(
        &id,
        CommandState::Control {
            outcome: events[0].outcome.clone(),
        },
    );
    // A recorded local cancellation request is not a canonical result.
    assert_eq!(cancel.task_receipts(Some(&started)).len(), 1);
    release.send(()).unwrap();
    wait_workers(&mut control);
    job.join().unwrap();
    let finished = fixture.store.snapshot().unwrap();
    assert_eq!(finished.tasks[0].status, TaskStatus::Cancelled);
    assert_eq!(
        cancel.task_receipts(Some(&finished)),
        run.task_receipts(Some(&finished))
    );
    assert_eq!(cancel.task_receipts(Some(&finished)).len(), 2);
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    drop(autosave);
    let restored = ConversationStore::open(&fixture.store, "default")
        .unwrap()
        .load()
        .unwrap()
        .unwrap()
        .restore();
    assert_eq!(restored.selected, 1);
    assert_eq!(restored.sessions[1].draft, "newer draft remains");
    assert!(restored.sessions[1].commands().is_empty());
    assert!(restored
        .sessions
        .iter()
        .all(|session| session.messages.is_empty()));
    assert_eq!(restored.sessions[0].commands()[0].intent, cancel);
    assert_eq!(
        restored.sessions[0].commands()[0]
            .intent
            .task_receipts(Some(&finished))
            .len(),
        2
    );
    let mut restarted = TaskControl::new(fixture.store.clone());
    assert!(!restarted.dispatch.enabled);
    assert!(restarted.dispatch_prepared(&runtime, &cancel).is_err());
    assert!(restarted.take_control_events().is_empty());
    assert_eq!(
        serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap(),
        serde_json::to_value(&finished).unwrap()
    );
}

#[test]
fn cancellation_can_capture_a_worker_before_its_start_receipt_is_visible() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Ollama::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        Duration::from_secs(1),
    )
    .unwrap();
    let mut control = control(&fixture, provider);
    let before = fixture.store.snapshot().unwrap();
    let run = prepared(&mut control, "/run 1");
    control.dispatch_prepared(&runtime, &run).unwrap();
    let cancel = prepared(&mut control, "/cancel-task 1");
    assert_eq!(
        serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert!(cancel.task_receipts(Some(&before)).is_empty());
    control.dispatch_prepared(&runtime, &cancel).unwrap();
    assert_eq!(
        control.take_control_events()[0].outcome,
        Outcome::CancellationRequested
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !control.workers.is_empty() {
        runtime.block_on(async { tokio::time::sleep(Duration::from_millis(2)).await });
        control.poll();
        assert!(Instant::now() < deadline);
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let final_state = fixture.store.snapshot().unwrap();
    if final_state.tasks[0].run.is_some() {
        assert_eq!(final_state.tasks[0].status, TaskStatus::Cancelled);
        assert_eq!(cancel.task_receipts(Some(&final_state)).len(), 2);
    } else {
        assert_eq!(
            serde_json::to_value(&final_state).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
        assert!(cancel.task_receipts(Some(&final_state)).is_empty());
    }
}

#[test]
fn dispatch_requests_bind_epoch_scope_and_controller_and_replay_only_historical_outcomes() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = control(&fixture, unavailable_provider());
    let before = fixture.store.snapshot().unwrap();
    let on = prepared(&mut control, "/dispatch on");
    assert!(!control.dispatch.enabled);
    control.dispatch_prepared(&runtime, &on).unwrap();
    wait_intent(&mut control, &on);
    assert!(control.dispatch.enabled);
    assert_eq!(
        control.take_control_events()[0].outcome,
        Outcome::DispatchChanged { enabled: true }
    );
    assert!(control.workers.is_empty());
    assert_eq!(
        serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    let stale = prepared(&mut control, "/dispatch on");
    let off = prepared(&mut control, "/dispatch off");
    control.dispatch_prepared(&runtime, &off).unwrap();
    assert!(!control.dispatch.enabled);
    assert_eq!(
        control.take_control_events()[0].outcome,
        Outcome::DispatchChanged { enabled: false }
    );
    assert!(control.dispatch_prepared(&runtime, &stale).is_err());
    assert!(control.take_control_events().is_empty());
    control.dispatch_prepared(&runtime, &on).unwrap();
    assert!(
        !control.dispatch.enabled,
        "Replayed acknowledgment must not reactivate historical enable"
    );
    assert_eq!(
        control.take_control_events()[0].outcome,
        Outcome::DispatchChanged { enabled: true }
    );
    let mut restarted = TaskControl::new(fixture.store.clone());
    restarted.observe_scope(fixture.store.understanding().snapshot().unwrap());
    assert!(restarted.dispatch_prepared(&runtime, &on).is_err());
    assert!(!restarted.dispatch.enabled);
    let stale_scope = prepared(&mut control, "/dispatch on");
    fixture
        .store
        .understanding()
        .transact(alfredo_tui::understanding::Request {
            correlation: "changed-scope".into(),
            expected_revision: 0,
            action: alfredo_tui::understanding::Action::Draft {
                brief: alfredo_tui::understanding::Brief {
                    destination: "Different objective".into(),
                    scope: "Different work".into(),
                    constraints: "Preserve tests".into(),
                    uncertainty: "Latency".into(),
                },
            },
        })
        .unwrap();
    control.dispatch_prepared(&runtime, &stale_scope).unwrap();
    wait_intent(&mut control, &stale_scope);
    assert!(!control.dispatch.enabled);
    assert!(control.intent_error(&stale_scope).is_some());
    assert!(control.take_control_events().is_empty());
    assert_eq!(
        serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
}

#[test]
fn dispatch_off_leaves_running_worker_alive_and_invalidates_pending_enable() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (provider, job, seen, release) = server(true);
    let mut control = control(&fixture, provider);
    let on = prepared(&mut control, "/dispatch on");
    control.dispatch_prepared(&runtime, &on).unwrap();
    wait_intent(&mut control, &on);
    assert!(control.dispatch.enabled);
    control.take_control_events();
    let launch = control.prepare_dispatch().unwrap().unwrap();
    control
        .dispatch_prepared(&runtime, &Intent::DispatchRun { request: launch })
        .unwrap();
    seen.recv_timeout(Duration::from_secs(10)).unwrap();
    control.poll();
    let cancel = prepared(&mut control, "/cancel-task 1");
    assert_eq!(
        cancel
            .task_receipts(Some(&fixture.store.snapshot().unwrap()))
            .len(),
        1
    );
    let off = prepared(&mut control, "/dispatch off");
    control.dispatch_prepared(&runtime, &off).unwrap();
    assert!(!control.dispatch.enabled);
    assert!(!control.workers[&1].load(Ordering::SeqCst));
    assert!(control.prepare_dispatch().unwrap().is_none());
    control.take_control_events();
    release.send(()).unwrap();
    wait_workers(&mut control);
    job.join().unwrap();
    assert_eq!(
        fixture.store.snapshot().unwrap().tasks[0].status,
        TaskStatus::ReviewReady
    );
    assert_eq!(
        cancel
            .task_receipts(Some(&fixture.store.snapshot().unwrap()))
            .len(),
        2
    );
    assert!(control.dispatch_prepared(&runtime, &cancel).is_err());
    assert!(control.take_control_events().is_empty());
    // Queue an enable read, but disable before polling its result.
    let pending = prepared(&mut control, "/dispatch on");
    control.dispatch_prepared(&runtime, &pending).unwrap();
    let stop = prepared(&mut control, "/dispatch off");
    control.dispatch_prepared(&runtime, &stop).unwrap();
    wait_intent(&mut control, &pending);
    assert!(!control.dispatch.enabled);
    assert!(control.intent_error(&pending).is_some());
    assert!(control
        .take_control_events()
        .iter()
        .all(|event| event.request.correlation == stop.correlation()));
}

fn enable_saved_dispatch(
    control: &mut TaskControl,
    runtime: &tokio::runtime::Runtime,
    app: &mut App,
    autosave: &mut Autosave,
) -> Intent {
    let on = prepared(control, "/dispatch on");
    let id = app.sessions[0]
        .submit_command("/dispatch on".into(), on.clone())
        .unwrap();
    autosave
        .finish(runtime, app, Default::default(), None)
        .unwrap();
    assert!(autosave.contains_saved_command(0, app.sessions[0].commands().last().unwrap()));
    control.dispatch_prepared(runtime, &on).unwrap();
    wait_intent(control, &on);
    assert!(control.dispatch.enabled);
    let event = control.take_control_events().pop().unwrap();
    assert_eq!(event.request.correlation, on.correlation());
    app.sessions[0].set_command_state(
        &id,
        CommandState::Control {
            outcome: event.outcome,
        },
    );
    autosave
        .finish(runtime, app, Default::default(), None)
        .unwrap();
    on
}

#[test]
fn automatic_worker_launch_waits_for_exact_saved_origin_and_restores_without_replay() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (provider, job, seen, release) = server(true);
    let mut control = control(&fixture, provider);
    let mut app = App::new("fixture".into());
    let mut autosave = Autosave::new(ConversationStore::open(&fixture.store, "default").unwrap());
    let source = enable_saved_dispatch(&mut control, &runtime, &mut app, &mut autosave);
    app.add_session();
    app.sessions[1].insert("Reading and drafting in another session");
    let before = serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap();
    let launch = control.prepare_dispatch().unwrap().unwrap();
    let Intent::Control { request: parent } = source else {
        unreachable!()
    };
    assert_eq!(launch.source, parent);
    assert_eq!(launch.approval_revision, 3);
    assert!(control.workers.is_empty());
    assert!(control.dispatch.attempts.is_empty());
    assert_eq!(
        serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap(),
        before
    );
    assert!(matches!(seen.try_recv(), Err(mpsc::TryRecvError::Empty)));
    let intent = Intent::DispatchRun {
        request: launch.clone(),
    };
    let id = app.sessions[0]
        .submit_automatic_command("Dispatch selected task #1".into(), intent.clone())
        .unwrap();
    let entry = app.sessions[0].commands().last().unwrap();
    assert!(
        !autosave.contains_saved_command(0, entry),
        "An older successful parent save cannot release a child"
    );
    assert!(app.sessions[1].commands().is_empty());
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    assert!(!autosave.contains_saved_command(1, app.sessions[0].commands().last().unwrap()));
    assert!(autosave.contains_saved_command(0, app.sessions[0].commands().last().unwrap()));
    control.dispatch_prepared(&runtime, &intent).unwrap();
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    seen.recv_timeout(Duration::from_secs(10)).unwrap();
    let started = fixture.store.snapshot().unwrap();
    assert_eq!(intent.task_receipts(Some(&started)).len(), 1);
    assert_eq!(
        started.receipts.last().unwrap().request.correlation,
        launch.correlation
    );
    release.send(()).unwrap();
    wait_workers(&mut control);
    job.join().unwrap();
    let finished = fixture.store.snapshot().unwrap();
    assert_eq!(finished.tasks[0].status, TaskStatus::ReviewReady);
    assert_eq!(intent.task_receipts(Some(&finished)).len(), 2);
    let mut wrong_approval = intent.clone();
    let Intent::DispatchRun { request } = &mut wrong_approval else {
        unreachable!()
    };
    request.approval_revision = 2;
    wrong_approval.validate().unwrap();
    assert!(wrong_approval.reconcile(Some(&finished), None).is_none());
    assert!(wrong_approval.task_receipts(Some(&finished)).is_empty());
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    drop(autosave);
    let mut restored = ConversationStore::open(&fixture.store, "default")
        .unwrap()
        .load()
        .unwrap()
        .unwrap()
        .restore();
    assert_eq!(restored.selected, 1);
    assert!(restored.sessions[1].commands().is_empty());
    assert_eq!(
        restored.sessions[1].draft,
        "Reading and drafting in another session"
    );
    assert!(restored
        .sessions
        .iter()
        .all(|session| session.messages.is_empty()));
    assert!(matches!(
        restored.sessions[0].commands()[1].state,
        CommandState::Unknown { .. }
    ));
    assert_eq!(
        restored.sessions[0].commands()[1]
            .intent
            .task_receipts(Some(&finished))
            .len(),
        2
    );
    assert!(restored.sessions[0].retry_command(&id).is_err());
    let mut restarted = TaskControl::new(fixture.store.clone());
    restarted.snapshot = Some(finished.clone());
    restarted.observe_scope(fixture.store.understanding().snapshot().unwrap());
    assert!(restarted.prepare_dispatch().unwrap().is_none());
    assert!(restarted.dispatch_prepared(&runtime, &intent).is_err());
    assert!(restarted.workers.is_empty());
    assert!(!restarted.dispatch.enabled);
    assert_eq!(
        serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap(),
        serde_json::to_value(finished).unwrap()
    );
}

#[test]
fn failed_automatic_intent_save_stops_selection_and_off_invalidates_a_saved_launch() {
    for fail_save in [true, false] {
        let fixture = Fixture::new();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let provider = Ollama::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            Duration::from_secs(1),
        )
        .unwrap();
        let mut control = control(&fixture, provider);
        let mut app = App::new("fixture".into());
        let mut autosave =
            Autosave::new(ConversationStore::open(&fixture.store, "default").unwrap());
        enable_saved_dispatch(&mut control, &runtime, &mut app, &mut autosave);
        let before = serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap();
        let launch = control.prepare_dispatch().unwrap().unwrap();
        let intent = Intent::DispatchRun {
            request: launch.clone(),
        };
        app.sessions[0]
            .submit_automatic_command("Dispatch selected task #1".into(), intent.clone())
            .unwrap();
        assert!(control.dispatch.attempts.is_empty());
        assert!(!autosave.contains_saved_command(0, app.sessions[0].commands().last().unwrap()));
        if fail_save {
            let path = fs::read_dir(fixture.store.conversation_directory().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("conversations-")
                        && path
                            .extension()
                            .is_some_and(|extension| extension == "json")
                })
                .unwrap();
            let original = fs::read(&path).unwrap();
            fs::remove_file(&path).unwrap();
            fs::create_dir(&path).unwrap();
            assert!(autosave
                .finish(&runtime, &app, Default::default(), None)
                .is_err());
            assert!(!autosave.contains_saved_command(0, app.sessions[0].commands().last().unwrap()));
            control.refuse_dispatch(&launch, "Intent save failed; no launch dispatched");
            assert!(control.dispatch.failures.contains_key(&1));
            fs::remove_dir(&path).unwrap();
            fs::write(&path, original).unwrap();
        } else {
            autosave
                .finish(&runtime, &app, Default::default(), None)
                .unwrap();
            assert!(autosave.contains_saved_command(0, app.sessions[0].commands().last().unwrap()));
            let off = prepared(&mut control, "/dispatch off");
            control.dispatch_prepared(&runtime, &off).unwrap();
            assert!(control.dispatch_prepared(&runtime, &intent).is_err());
        }
        assert!(!control.dispatch.enabled);
        assert!(control.prepare_dispatch().unwrap().is_none());
        assert!(control.workers.is_empty());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(
            serde_json::to_value(fixture.store.snapshot().unwrap()).unwrap(),
            before
        );
    }
}
