use alfredo_tui::{
    planner::{Plan, Step},
    provider::Ollama,
    tasks::{TaskStore, WorkPolicy},
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
        usage: None,
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

fn wait(planner: &mut alfredo_tui::planner::Planner) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while planner.active() {
        planner.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
}
/// Issue #743: a pasted goal is one line downstream.
#[test]
fn planner_command_normalise_goal_joins_each_run_of_line_breaks_with_one_space() {
    use alfredo_tui::planner_command::normalise_goal;
    for (pasted, one_line) in [
        ("Fix the bug\nin calc.py", "Fix the bug in calc.py"),
        ("Fix the bug\rin calc.py", "Fix the bug in calc.py"),
        ("Fix the bug\r\n\r\n\nin calc.py", "Fix the bug in calc.py"),
        ("\r\n  Fix it \n\n", "Fix it"),
        ("Fix\n a \nb", "Fix  a  b"),
        ("one line", "one line"),
        ("\r\n\n", ""),
    ] {
        assert_eq!(normalise_goal(pasted), one_line, "{pasted:?}");
    }
}

#[test]
fn planner_command_goal_refusal_names_the_rule_that_failed() {
    use alfredo_tui::planner_command::goal_refusal;
    assert_eq!(goal_refusal("Goal", "x", 1), None);
    assert_eq!(goal_refusal("Goal", &"x".repeat(4096), 4096), None);
    assert_eq!(
        goal_refusal("Goal", "", 4096).as_deref(),
        Some("Goal is empty")
    );
    assert_eq!(
        goal_refusal("Goal", " \t ", 4096).as_deref(),
        Some("Goal is empty")
    );
    for control in ["a\x1bb", "a\0b", "a\x7fb"] {
        let refusal = goal_refusal("Goal", control, 4096).unwrap();
        assert!(
            refusal.starts_with("Goal has a control character")
                && refusal.contains("line breaks are fine but other control characters are not"),
            "{refusal}"
        );
    }
    assert_eq!(
        goal_refusal("Goal", &"x".repeat(4097), 4096).as_deref(),
        Some("Goal is 4097 bytes; the limit is 4096 bytes")
    );
    assert_eq!(
        goal_refusal("Request", &"é".repeat(5), 9).as_deref(),
        Some("Request is 10 bytes; the limit is 9 bytes")
    );
}

#[test]
fn planner_saved_intent_generates_revises_fails_and_cancels_at_exact_origin() {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::CommandState,
        conversations::{Autosave, ConversationStore},
        model::App,
        planner::Planner,
        planner_command::Outcome,
    };
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut planner = Planner::default();
    let mut app = App::new("fixture".into());
    let snapshot = fixture.store.snapshot().unwrap();
    let request = planner
        .prepare_command(
            "generation-one",
            "/plan Implement calculation",
            "fixture",
            &snapshot,
        )
        .unwrap()
        .unwrap();
    let id = app.sessions[0]
        .submit_command(
            "/plan Implement calculation".into(),
            Intent::Planner {
                request: request.clone(),
            },
        )
        .unwrap();
    app.open_chat();
    let mut autosave = Autosave::new(ConversationStore::open(&fixture.store, "default").unwrap());
    assert!(!autosave.contains_saved_command(0, &app.sessions[0].commands()[0]));
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    assert!(autosave.contains_saved_command(0, &app.sessions[0].commands()[0]));
    let (provider, job, wire) = server_capture(
        serde_json::json!({"tasks": plan().tasks}).to_string(),
        true,
        Duration::ZERO,
    );
    planner
        .dispatch_command(&runtime, provider, &request, fixture.store.clone())
        .unwrap();
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    wait(&mut planner);
    job.join().unwrap();
    wire.recv_timeout(Duration::from_secs(1)).unwrap();
    let events = planner.take_command_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].request, request);
    assert!(
        matches!(events[0].outcome, Outcome::Generated { tasks: 2, .. }),
        "{:?}",
        events
    );
    let saved = planner.checkpoint().unwrap();
    assert_eq!(saved.origin.as_ref(), Some(&request));
    assert_eq!(saved.outcome_for(&request), Some(events[0].outcome.clone()));
    assert!(fixture.store.snapshot().unwrap().tasks.is_empty());
    autosave
        .finish(&runtime, &app, Default::default(), Some(saved.clone()))
        .unwrap();
    drop(autosave);
    let store = ConversationStore::open(&fixture.store, "default").unwrap();
    let restored = store.load().unwrap().unwrap().restore();
    assert_eq!(restored.selected, 1);
    assert!(matches!(
        restored.sessions[0].commands()[0].state,
        CommandState::Planner { .. }
    ));
    assert!(restored.sessions[1].commands().is_empty());
    assert!(restored.sessions.iter().all(|s| s.messages.is_empty()));
    let mut reopened = Planner::default();
    reopened.restore(saved.clone()).unwrap();
    assert!(!reopened.active());
    assert!(reopened.take_command_events().is_empty());
    let revision = reopened
        .prepare_command(
            "revision-two",
            "/plan-revise Make checks more specific",
            "fixture",
            &snapshot,
        )
        .unwrap()
        .unwrap();
    let (provider, job, _) = server_capture("invalid JSON".into(), true, Duration::ZERO);
    reopened
        .dispatch_command(&runtime, provider, &revision, fixture.store.clone())
        .unwrap();
    wait(&mut reopened);
    job.join().unwrap();
    let failed = reopened.take_command_events();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].request, revision);
    assert!(matches!(failed[0].outcome, Outcome::Failed { .. }));
    assert_eq!(reopened.checkpoint().unwrap(), saved);
    let (provider, job, _) = server_capture(
        serde_json::json!({"tasks": plan().tasks}).to_string(),
        true,
        Duration::ZERO,
    );
    let revised = reopened
        .prepare_command(
            "revision-three",
            "/plan-revise Make checks more specific",
            "fixture",
            &snapshot,
        )
        .unwrap()
        .unwrap();
    reopened
        .dispatch_command(&runtime, provider, &revised, fixture.store.clone())
        .unwrap();
    wait(&mut reopened);
    job.join().unwrap();
    assert_eq!(reopened.take_command_events()[0].request, revised);
    let newdraft = reopened.checkpoint().unwrap();
    assert_eq!(newdraft.origin.as_ref(), Some(&revised));
    assert!(newdraft.outcome_for(&request).is_none());
    let (provider, job, wire) = server_capture(
        serde_json::json!({"tasks": plan().tasks}).to_string(),
        true,
        Duration::from_millis(100),
    );
    let pending = reopened
        .prepare_command(
            "generation-cancelled",
            "/plan-revise Another change",
            "fixture",
            &snapshot,
        )
        .unwrap()
        .unwrap();
    reopened
        .dispatch_command(&runtime, provider.clone(), &pending, fixture.store.clone())
        .unwrap();
    wire.recv_timeout(Duration::from_secs(4)).unwrap();
    let cancel = reopened
        .prepare_command("cancel-exact", "/plan-cancel", "fixture", &snapshot)
        .unwrap()
        .unwrap();
    reopened
        .dispatch_command(&runtime, provider.clone(), &cancel, fixture.store.clone())
        .unwrap();
    job.join().unwrap();
    assert!(!reopened.active());
    let events = reopened.take_command_events();
    assert!(events
        .iter()
        .any(|e| e.request == pending && e.outcome == Outcome::Stopped));
    assert!(events
        .iter()
        .any(|e| e.request == cancel && e.outcome == Outcome::Stopped));
    assert_eq!(reopened.checkpoint().unwrap(), newdraft);
    assert!(reopened
        .dispatch_command(&runtime, provider, &cancel, fixture.store.clone())
        .is_err());
    assert!(fixture.store.snapshot().unwrap().tasks.is_empty());
}
