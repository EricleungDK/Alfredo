use alfredo_tui::{
    conversations::{ConversationStore, Snapshot},
    model::App,
    tasks::{Action, Request, TaskStore},
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    state: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        Self::build(true)
    }
    fn build(commit: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo doctor {} {}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--allow-empty",
                "-qm",
                "baseline",
            ],
        ]
        .into_iter()
        .take(if commit { 2 } else { 1 })
        {
            assert!(Command::new("git")
                .arg("-C")
                .arg(&workspace)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        Self {
            state: root.join("state"),
            root,
            workspace,
        }
    }
    fn command(&self, endpoint: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"));
        command
            .args([
                "--doctor",
                "--model",
                "initial-model",
                "--endpoint",
                endpoint,
                "--workspace",
            ])
            .arg(&self.workspace)
            .arg("--state-dir")
            .arg(&self.state);
        command
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn catalog(model: &str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let body = serde_json::json!({"models":[{"name":model}]}).to_string();
    let job = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        assert!(
            request.starts_with(b"GET /api/tags HTTP/1.1"),
            "Diagnostics must not request inference"
        );
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    (endpoint, job)
}

#[test]
fn doctor_uses_restored_model_without_terminal_or_mutating_saved_state() {
    let fixture = Fixture::new();
    let tasks = TaskStore::new(&fixture.state, &fixture.workspace, "default").unwrap();
    tasks
        .transact(Request {
            correlation: "proposal".into(),
            expected_revision: 0,
            action: Action::Propose {
                title: "saved task".into(),
                model: "worker".into(),
                dependencies: vec![],
            },
        })
        .unwrap();
    let store = ConversationStore::open(&tasks, "default").unwrap();
    store
        .save(&Snapshot::capture(
            &App::new("restored-model".into()),
            "default",
        ))
        .unwrap();
    drop(store);
    let directory = tasks.conversation_directory().unwrap();
    let original: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    for (model, expected_code) in [("restored-model", 0), ("different-model", 2)] {
        let (endpoint, server) = catalog(model);
        let output = fixture.command(&endpoint).output().unwrap();
        server.join().unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(output.status.code(), Some(expected_code), "{text}");
        assert!(text.contains("restored-model"));
        assert!(!text.contains("Interactive terminal required") && !text.contains('\u{1b}'));
        for (path, bytes) in &original {
            assert_eq!(&fs::read(path).unwrap(), bytes);
        }
    }
}

#[test]
fn doctor_reports_storage_and_server_failures_together_without_erasing_state() {
    let fixture = Fixture::new();
    fs::write(&fixture.state, "keep original bytes").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let output = fixture.command(&endpoint).output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        text.contains("FAIL storage") && text.contains("FAIL model server"),
        "{text}"
    );
    assert!(text.contains("--endpoint") && text.contains("--state-dir"));
    assert_eq!(
        fs::read_to_string(&fixture.state).unwrap(),
        "keep original bytes"
    );
}

#[test]
fn doctor_names_the_fix_for_a_repository_without_commits() {
    let fixture = Fixture::build(false);
    let (endpoint, server) = catalog("initial-model");
    let output = fixture.command(&endpoint).output().unwrap();
    server.join().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(
        text.contains("FAIL worker workspace: This repository has no commits yet; make an initial commit (git commit --allow-empty -m init)"),
        "{text}"
    );
    assert!(!text.contains("ambiguous argument"), "{text}");
}
