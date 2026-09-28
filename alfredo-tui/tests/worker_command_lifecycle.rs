//! Real worker evidence establishes command Start/Finish projection; rendering never starts work.
use alfredo_tui::{
    command_intent::{Acknowledgment, Intent},
    console_command::CommandState,
    conversations::{Autosave, ConversationStore, Snapshot as ConversationSnapshot},
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
            "alfredo-worker-command-{}-{}",
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

#[test]
fn worker_command_links_start_finish_and_recovery_to_original_session_across_restart() {
    for outcome in [
        TaskStatus::ReviewReady,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ] {
        let fixture = Fixture::new();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (provider, job, seen, release) = server(outcome == TaskStatus::ReviewReady);
        let mut control = TaskControl::new(fixture.store.clone());
        control.snapshot = Some(fixture.store.snapshot().unwrap());
        control.set_provider(provider);
        control.refresh(&runtime);
        let deadline = Instant::now() + Duration::from_secs(15);
        while control.pending {
            control.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        let intent = control
            .prepare_command("/run 1", "fixture")
            .unwrap()
            .unwrap();
        let mut app = App::new("fixture".into());
        let id = app.sessions[0]
            .submit_command("/run 1".into(), intent.clone())
            .unwrap();
        app.add_session();
        assert_eq!(app.selected, 1);
        let mut autosave =
            Autosave::new(ConversationStore::open(&fixture.store, "default").unwrap());
        autosave
            .finish(&runtime, &app, Default::default(), None)
            .unwrap();
        assert!(autosave.contains_saved_command(0, &app.sessions[0].commands()[0]));
        control.dispatch_prepared(&runtime, &intent).unwrap();
        app.sessions[0].set_command_state(&id, CommandState::Submitted);
        seen.recv_timeout(Duration::from_secs(10)).unwrap();
        let started = fixture.store.snapshot().unwrap();
        let start_bytes = serde_json::to_vec(&started).unwrap();
        let start_chain = intent.task_receipts(Some(&started));
        assert_eq!(start_chain.len(), 1);
        assert!(
            fixture.store.recover(1).is_err(),
            "Live owner cannot be recovered as completed"
        );
        assert_eq!(
            intent
                .task_receipts(Some(&fixture.store.snapshot().unwrap()))
                .len(),
            1
        );
        if outcome == TaskStatus::Cancelled {
            control
                .command(&runtime, "/cancel-task 1", "fixture")
                .unwrap();
        }
        release.send(()).unwrap();
        while !control.workers.is_empty() {
            control.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        job.join().unwrap();
        let finished = fixture.store.snapshot().unwrap();
        assert_eq!(finished.tasks[0].status, outcome);
        let chain = intent.task_receipts(Some(&finished));
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0], start_chain[0]);
        let run = finished.tasks[0].run.as_ref().unwrap();
        let Acknowledgment::Task {
            revision,
            correlation,
            task,
        } = &chain[1]
        else {
            panic!("Expected task result");
        };
        assert_eq!(*task, 1);
        assert_eq!(correlation, &format!("finish:{}", run.id));
        assert!(
            matches!(&finished.receipts[*revision as usize-1].request.action,Action::Finish { run: id,status,.. } if id==&run.id && status==&outcome)
        );
        assert!(app.sessions[1].commands().is_empty());
        assert_eq!(app.sessions[0].commands()[0].intent, intent);
        autosave
            .finish(&runtime, &app, Default::default(), None)
            .unwrap();
        drop(autosave);
        let saved = ConversationStore::open(&fixture.store, "default").unwrap();
        let restored = saved.load().unwrap().unwrap().restore();
        assert_eq!(restored.selected, 1);
        assert!(restored.sessions[1].commands().is_empty());
        assert!(matches!(
            restored.sessions[0].commands()[0].state,
            CommandState::Unknown { .. }
        ));
        assert_eq!(
            restored.sessions[0].commands()[0]
                .intent
                .task_receipts(Some(&finished)),
            chain
        );
        let Intent::Run {
            expected_revision,
            task,
            ..
        } = intent.clone()
        else {
            unreachable!()
        };
        assert!(Intent::Run {
            correlation: "wrong-start".into(),
            expected_revision,
            task
        }
        .task_receipts(Some(&finished))
        .is_empty());
        let mut corrupted = finished.clone();
        corrupted.tasks[0]
            .run
            .as_mut()
            .unwrap()
            .id
            .push_str("-different");
        assert!(intent.task_receipts(Some(&corrupted)).len() < 2);
        if outcome == TaskStatus::ReviewReady {
            // Simulate missing result publication using the real earlier canonical
            // Start snapshot, retaining the independently produced evidence.
            let path = fixture
                .store
                .conversation_directory()
                .unwrap()
                .join("tasks.json");
            fs::write(&path, &start_bytes).unwrap();
            assert_eq!(
                intent
                    .task_receipts(Some(&fixture.store.snapshot().unwrap()))
                    .len(),
                1
            );
            let (recovered, _) = fixture.store.recover(1).unwrap();
            assert_eq!(intent.task_receipts(Some(&recovered)), chain);
        }
        assert_eq!(restored.sessions[0].commands()[0].intent, intent);
        assert_eq!(
            ConversationSnapshot::capture(&restored, "default").sessions[0]
                .messages
                .len(),
            0
        );
    }
}
