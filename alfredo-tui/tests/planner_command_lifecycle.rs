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

/// Issue #751: tab, vertical tab, form feed and NEL join like line breaks.
#[test]
fn planner_command_normalise_goal_joins_tabs_and_other_separators_like_line_breaks() {
    use alfredo_tui::planner_command::normalise_goal;
    for (pasted, one_line) in [
        ("a\tb\n\t- c", "a b - c"),
        ("a\tb", "a b"),
        ("a\x0bb", "a b"),
        ("a\x0cb", "a b"),
        ("a\u{85}b", "a b"),
        ("Fix:\n\t- x", "Fix: - x"),
        ("a\t\t\x0b\x0cb", "a b"),
    ] {
        assert_eq!(normalise_goal(pasted), one_line, "{pasted:?}");
    }
    // ESC, NUL and DEL are not separators: they stay, to be refused.
    for kept in ["a\x1bb", "a\0b", "a\x7fb"] {
        assert_eq!(normalise_goal(kept), kept);
    }
}

fn prompt_of(request: &alfredo_tui::planner_command::Request) -> &str {
    use alfredo_tui::planner_command::Operation;
    match &request.operation {
        Operation::Generate { prompt, .. } | Operation::Revise { prompt, .. } => prompt,
        other => panic!("not a prompt operation: {other:?}"),
    }
}

/// A planner holding one finished draft, so `/plan-revise` has a base.
fn drafted(fixture: &Fixture, runtime: &tokio::runtime::Runtime) -> alfredo_tui::planner::Planner {
    let mut planner = alfredo_tui::planner::Planner::default();
    let snapshot = fixture.store.snapshot().unwrap();
    let request = planner
        .prepare_command("first", "/plan Implement calculation", "fixture", &snapshot)
        .unwrap()
        .unwrap();
    let (provider, job, _) = server_capture(
        serde_json::json!({"tasks": plan().tasks}).to_string(),
        true,
        Duration::ZERO,
    );
    planner
        .dispatch_command(runtime, provider, &request, fixture.store.clone())
        .unwrap();
    wait(&mut planner);
    job.join().unwrap();
    assert!(planner.checkpoint().is_some());
    planner
}

#[test]
fn prepare_command_joins_a_pasted_multi_line_plan_request_with_single_spaces() {
    let fixture = Fixture::new();
    let planner = alfredo_tui::planner::Planner::default();
    let snapshot = fixture.store.snapshot().unwrap();
    for (pasted, joined) in [
        ("/plan first line\rsecond line", "first line second line"),
        ("/plan first line\nsecond line", "first line second line"),
        ("/plan first line\r\nsecond line", "first line second line"),
        ("/plan Fix:\n\t- x\n\t- y", "Fix: - x - y"),
        ("/plan a\tb", "a b"),
        ("/plan  \n lead\n\n", "lead"),
    ] {
        let request = planner
            .prepare_command("c", pasted, "fixture", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(prompt_of(&request), joined, "{pasted:?}");
    }
}

#[test]
fn prepare_command_takes_plan_followed_directly_by_any_whitespace() {
    let fixture = Fixture::new();
    let planner = alfredo_tui::planner::Planner::default();
    let snapshot = fixture.store.snapshot().unwrap();
    for text in [
        "/plan\nFix the bug",
        "/plan\r\nFix the bug",
        "/plan\tFix the bug",
        "/plan Fix the bug",
    ] {
        let request = planner
            .prepare_command("c", text, "fixture", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(prompt_of(&request), "Fix the bug", "{text:?}");
    }
    // A longer command word is not the /plan prefix.
    assert!(planner
        .prepare_command("c", "/planet Fix", "fixture", &snapshot)
        .unwrap()
        .is_none());
    assert!(planner
        .prepare_command("c", "/plan", "fixture", &snapshot)
        .unwrap()
        .is_none());
}

#[test]
fn prepare_command_refuses_a_plan_request_with_the_goal_refusal_wording() {
    let fixture = Fixture::new();
    let planner = alfredo_tui::planner::Planner::default();
    let snapshot = fixture.store.snapshot().unwrap();
    let long = format!("/plan {}", "y".repeat(9000));
    assert_eq!(
        planner.prepare_command("c", &long, "fixture", &snapshot),
        Err("Request is 9000 bytes; the limit is 8192 bytes".to_string())
    );
    let multi = format!("/plan {}\n{}", "y".repeat(4100), "z".repeat(4100));
    assert_eq!(
        planner.prepare_command("c", &multi, "fixture", &snapshot),
        Err("Request is 8201 bytes; the limit is 8192 bytes".to_string())
    );
    let at_limit = format!("/plan {}", "y".repeat(8192));
    assert!(planner
        .prepare_command("c", &at_limit, "fixture", &snapshot)
        .unwrap()
        .is_some());
    for control in ["\x1b", "\0", "\x7f"] {
        let error = planner
            .prepare_command(
                "c",
                &format!("/plan Fix\n{control}bug"),
                "fixture",
                &snapshot,
            )
            .unwrap_err();
        assert!(
            error.starts_with("Request has a control character")
                && error.contains("line breaks are fine but other control characters are not"),
            "{error}"
        );
    }
}

#[test]
fn prepare_command_joins_a_pasted_multi_line_plan_revise_request() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let planner = drafted(&fixture, &runtime);
    let snapshot = fixture.store.snapshot().unwrap();
    for (pasted, joined) in [
        ("/plan-revise a\nb", "a b"),
        ("/plan-revise a\r\n\tb", "a b"),
        ("/plan-revise\nMake it smaller", "Make it smaller"),
        ("/plan-revise\tMake it smaller", "Make it smaller"),
    ] {
        let request = planner
            .prepare_command("r", pasted, "fixture", &snapshot)
            .unwrap()
            .unwrap();
        assert!(matches!(
            request.operation,
            alfredo_tui::planner_command::Operation::Revise { .. }
        ));
        assert_eq!(prompt_of(&request), joined, "{pasted:?}");
    }
    let long = format!("/plan-revise {}", "y".repeat(9000));
    assert_eq!(
        planner.prepare_command("r", &long, "fixture", &snapshot),
        Err("Request is 9000 bytes; the limit is 8192 bytes".to_string())
    );
    let error = planner
        .prepare_command("r", "/plan-revise a\x1bb", "fixture", &snapshot)
        .unwrap_err();
    assert!(
        error.starts_with("Request has a control character"),
        "{error}"
    );
}

/// A planner holding a finished draft whose prompt is `goal`.
fn drafted_from(
    fixture: &Fixture,
    runtime: &tokio::runtime::Runtime,
    goal: &str,
) -> alfredo_tui::planner::Planner {
    let mut planner = alfredo_tui::planner::Planner::default();
    let snapshot = fixture.store.snapshot().unwrap();
    let request = planner
        .prepare_command("first", &format!("/plan {goal}"), "fixture", &snapshot)
        .unwrap()
        .unwrap();
    let (provider, job, _) = server_capture(
        serde_json::json!({"tasks": plan().tasks}).to_string(),
        true,
        Duration::ZERO,
    );
    planner
        .dispatch_command(runtime, provider, &request, fixture.store.clone())
        .unwrap();
    wait(&mut planner);
    job.join().unwrap();
    planner
}

/// The bytes ` | Revision request: ` adds between the draft prompt and the request.
const REVISION_SEPARATOR: usize = " | Revision request: ".len();

fn overflow_message(combined: usize) -> String {
    format!("Revision would make the plan prompt {combined} bytes; the limit is 8192. Use a shorter request, or /plan a fresh one")
}

/// Issue #764.
#[test]
fn plan_revise_refuses_a_request_that_overflows_the_combined_prompt() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let planner = drafted_from(&fixture, &runtime, &"spec ".repeat(1200));
    let saved = planner.checkpoint().unwrap();
    let draft_len = saved.plan.prompt.len();
    assert!(draft_len >= 5999, "{draft_len}");
    let request = "more ".repeat(700);
    let request = request.trim();
    let snapshot = fixture.store.snapshot().unwrap();
    let expected = overflow_message(draft_len + REVISION_SEPARATOR + request.len());
    assert_eq!(
        planner.prepare_command(
            "r",
            &format!("/plan-revise {request}"),
            "fixture",
            &snapshot
        ),
        Err(expected.clone())
    );
    assert!(expected.contains("8192"));
    assert_eq!(planner.checkpoint().unwrap(), saved);

    // A request that skipped prepare_command is refused by dispatch before any planner request.
    let short = planner
        .prepare_command("s", "/plan-revise short", "fixture", &snapshot)
        .unwrap()
        .unwrap();
    let alfredo_tui::planner_command::Operation::Revise {
        revision,
        base_sha256,
        ..
    } = short.operation
    else {
        panic!("not a revise")
    };
    let forged = alfredo_tui::planner_command::Request {
        correlation: "r".into(),
        operation: alfredo_tui::planner_command::Operation::Revise {
            prompt: request.into(),
            revision,
            base_sha256,
        },
    };
    let mut planner = planner;
    let (provider, _job, received) = server_capture(
        serde_json::json!({"tasks": plan().tasks}).to_string(),
        true,
        Duration::ZERO,
    );
    assert_eq!(
        planner.dispatch_command(&runtime, provider, &forged, fixture.store.clone()),
        Err(expected)
    );
    assert!(received.recv_timeout(Duration::from_millis(300)).is_err());
    assert!(!planner.active());
    assert_eq!(planner.checkpoint().unwrap(), saved);
}

#[test]
fn plan_revise_accepts_a_combined_prompt_of_exactly_8192_bytes_and_refuses_one_more() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut planner = drafted_from(&fixture, &runtime, &"spec ".repeat(1200));
    let draft_len = planner.checkpoint().unwrap().plan.prompt.len();
    let room = 8192 - draft_len - REVISION_SEPARATOR;
    let snapshot = fixture.store.snapshot().unwrap();
    let over = "r".repeat(room + 1);
    assert_eq!(
        planner.prepare_command("r", &format!("/plan-revise {over}"), "fixture", &snapshot),
        Err(overflow_message(8193))
    );
    let exact = "r".repeat(room);
    let request = planner
        .prepare_command("r", &format!("/plan-revise {exact}"), "fixture", &snapshot)
        .unwrap()
        .unwrap();
    let (provider, job, received) = server_capture(
        serde_json::json!({"tasks": plan().tasks}).to_string(),
        true,
        Duration::ZERO,
    );
    planner
        .dispatch_command(&runtime, provider, &request, fixture.store.clone())
        .unwrap();
    wait(&mut planner);
    job.join().unwrap();
    received.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(planner.checkpoint().unwrap().plan.prompt.len(), 8192);
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
