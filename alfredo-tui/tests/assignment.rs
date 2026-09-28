use alfredo_tui::{
    assignment,
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
            "alfredo-assign-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("workspace")).unwrap();
        let store =
            TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
        let fixture = Self { root, store };
        fixture.action(Action::Propose {
            title: "Implement calculation".into(),
            model: "original".into(),
            dependencies: vec![],
        });
        fixture.action(Action::Permit {
            task: 1,
            policy: WorkPolicy {
                files: vec!["calc.py".into()],
                check: vec!["true".into()],
            },
        });
        fixture.action(Action::Approve { task: 1 });
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
    fn request(&self) -> Request {
        Request {
            correlation: "assign".into(),
            expected_revision: self.store.snapshot().unwrap().revision,
            action: Action::Assign {
                task: 1,
                model: "replacement".into(),
            },
        }
    }
    fn path(&self) -> std::path::PathBuf {
        self.store
            .conversation_directory()
            .unwrap()
            .join("tasks.json")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn catalog(
    body: &'static str,
    count: usize,
    mut before_response: impl FnMut() + Send + 'static,
) -> (Ollama, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Ollama::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        Duration::from_secs(3),
    )
    .unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        for _ in 0..count {
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut header = vec![];
            let mut byte = [0];
            while !header.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            assert!(header.starts_with(b"GET /api/tags HTTP/1.1"));
            before_response();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
        }
    });
    (provider, server)
}
#[tokio::test]
async fn assignment_resets_approval_preserves_policy_and_replays_offline_after_restart() {
    let fixture = Fixture::new();
    let before = fixture.store.snapshot().unwrap();
    let request = fixture.request();
    let (provider, server) = catalog(r#"{"models":[{"name":"replacement"}]}"#, 1, || {});
    let (snapshot, notice) = assignment::assign(fixture.store.clone(), request.clone(), provider)
        .await
        .unwrap();
    server.join().unwrap();
    assert!(notice.contains("replacement"));
    assert_eq!(snapshot.tasks[0].model, "replacement");
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Proposed);
    assert_eq!(snapshot.tasks[0].policy, before.tasks[0].policy);
    assert!(snapshot.tasks[0].run.is_none());
    assert!(fixture
        .store
        .transact(Request {
            correlation: "start-too-soon".into(),
            expected_revision: snapshot.revision,
            action: Action::Start {
                task: 1,
                baseline: "0".repeat(40),
                inputs: vec![]
            }
        })
        .is_err());
    let restored = TaskStore::new(
        &fixture.root.join("state"),
        &fixture.root.join("workspace"),
        "mission",
    )
    .unwrap();
    let offline = Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap();
    let replay = assignment::assign(restored, request.clone(), offline.clone())
        .await
        .unwrap()
        .0;
    assert_eq!(replay.revision, snapshot.revision);
    let mut changed = request;
    if let Action::Assign { model, .. } = &mut changed.action {
        *model = "other".into();
    }
    assert!(
        assignment::assign(fixture.store.clone(), changed, offline.clone())
            .await
            .unwrap_err()
            .contains("Correlation")
    );
    fixture.action(Action::Approve { task: 1 });
    fixture.action(Action::Start {
        task: 1,
        baseline: "0".repeat(40),
        inputs: vec![],
    });
    let bytes = fs::read(fixture.path()).unwrap();
    let mut running = fixture.request();
    running.correlation = "after-start".into();
    assert!(assignment::assign(fixture.store.clone(), running, offline)
        .await
        .unwrap_err()
        .contains("before execution"));
    assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
}
#[tokio::test]
async fn concurrent_state_change_during_catalog_query_refuses_assignment() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let store = fixture.store.clone();
    let (provider, server) = catalog(r#"{"models":[{"name":"replacement"}]}"#, 1, move || {
        store
            .transact(Request {
                correlation: "cancel-during-catalog".into(),
                expected_revision: 3,
                action: Action::Cancel { task: 1 },
            })
            .unwrap();
    });
    assert!(assignment::assign(fixture.store.clone(), request, provider)
        .await
        .unwrap_err()
        .contains("changed"));
    server.join().unwrap();
    let saved = fixture.store.snapshot().unwrap();
    assert_eq!(saved.tasks[0].status, TaskStatus::Cancelled);
    assert_eq!(saved.tasks[0].model, "original");
    assert_eq!(saved.revision, 4);
}
#[test]
fn terminal_retry_cannot_bypass_missing_model_and_legacy_state_rejects_assignment() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (provider, server) = catalog(r#"{"models":[{"name":"original"}]}"#, 2, || {});
    let mut control = TaskControl::new(fixture.store.clone());
    control.set_provider(provider);
    control.refresh(&runtime);
    let wait = |control: &mut TaskControl| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while control.pending {
            control.poll();
            assert!(Instant::now() < deadline, "{}", control.notice);
            thread::sleep(Duration::from_millis(2));
        }
    };
    wait(&mut control);
    let bytes = fs::read(fixture.path()).unwrap();
    for command in ["/assign 1 replacement", "/retry-task"] {
        control.command(&runtime, command, "original").unwrap();
        wait(&mut control);
        assert!(
            control.notice.contains("not installed"),
            "{}",
            control.notice
        );
        assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
    }
    server.join().unwrap();
    fixture.store.transact(fixture.request()).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.path()).unwrap()).unwrap();
    value["schema_version"] = 6.into();
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(fixture.path(), &bytes).unwrap();
    assert!(fixture.store.snapshot().unwrap_err().contains("schema v7"));
    assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
}
