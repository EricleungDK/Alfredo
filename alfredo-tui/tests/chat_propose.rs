//! Issue #239: chat proposes governed work (/plan or /go) instead of doing it. The
//! proposal ends the turn and prefills the prompt; nothing starts without the owner.
mod ollama_fixture;

use alfredo_tui::{
    chat_tools::{self, propose, Tier},
    model::{Message, Session, Status, ToolCall, ToolStep},
    provider::Ollama,
};
use ollama_fixture::{done, Reply};
use serde_json::json;
use std::{fs, path::PathBuf, time::Duration};
use tokio::sync::mpsc;

fn call(arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        name: "propose_plan".into(),
        arguments,
    }
}

fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("alfredo-propose-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn tool_call(arguments: serde_json::Value) -> Reply {
    Reply::Stream(
        vec![
            format!(
                "{}\n",
                json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"propose_plan","arguments":arguments}}]},"done":false})
            )
            .into_bytes(),
            format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )
}

#[test]
fn propose_plan_is_registered_in_the_propose_tier() {
    let tool = chat_tools::TOOLS
        .iter()
        .find(|tool| tool.name == "propose_plan")
        .expect("registered");
    assert_eq!(tool.tier, Tier::Propose);
    let definition = chat_tools::definitions()
        .into_iter()
        .find(|d| d["function"]["name"] == "propose_plan")
        .unwrap();
    assert_eq!(
        definition["function"]["parameters"]["required"],
        json!(["goal", "mode", "reason"])
    );
}

#[test]
fn a_valid_proposal_names_the_command_and_starts_nothing() {
    let root = workspace("valid");
    let result = chat_tools::dispatch(
        &root,
        &call(json!({"goal": "Add login\n  with tests", "mode": "go", "reason": "multi-file"})),
    );
    assert!(
        result.starts_with("proposed: /go Add login with tests\n"),
        "{result}"
    );
    assert!(result.contains("nothing starts"), "{result}");
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn bad_proposals_are_errors() {
    let root = workspace("bad");
    for arguments in [
        json!({"goal": "x", "mode": "run", "reason": "r"}),
        json!({"goal": "  \n ", "mode": "plan", "reason": "r"}),
        json!({"mode": "plan", "reason": "r"}),
        json!({"goal": "x", "reason": "r"}),
    ] {
        let result = chat_tools::dispatch(&root, &call(arguments.clone()));
        assert!(result.starts_with("error: "), "{arguments}: {result}");
    }
}

#[test]
fn long_goals_are_cut_and_control_characters_dropped() {
    let root = workspace("long");
    let goal = format!("a\u{1b}[31m{}", "b".repeat(5000));
    let result = chat_tools::dispatch(
        &root,
        &call(json!({"goal": goal, "mode": "plan", "reason": "r"})),
    );
    let command = propose::prefill(&ToolStep {
        calls: vec![call(json!({}))],
        results: vec![result],
    })
    .unwrap();
    assert!(!command.contains('\u{1b}'));
    assert!(command.len() <= "/plan ".len() + propose::MAX_GOAL);
}

#[test]
fn only_a_successful_propose_call_prefills() {
    let error = ToolStep {
        calls: vec![call(json!({}))],
        results: vec!["error: goal is required".into()],
    };
    assert_eq!(propose::prefill(&error), None);
    let other = ToolStep {
        calls: vec![ToolCall {
            name: "read_file".into(),
            arguments: json!({}),
        }],
        results: vec!["proposed: /go rm -rf\n".into()],
    };
    assert_eq!(propose::prefill(&other), None);
}

async fn turn(session: &mut Session, prompt: &str) -> usize {
    let root = workspace("loop");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, index| {
        if index == 0 {
            tool_call(
                json!({"goal": "Split ui.rs into modules", "mode": "plan", "reason": "broad refactor"}),
            )
        } else {
            done("SHOULD_NOT_CONTINUE")
        }
    });
    session.insert(prompt);
    let messages: Vec<Message> = session.begin().unwrap();
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    let attempt = session.attempt;
    chat_tools::run_turn(
        &provider,
        &root,
        0,
        attempt,
        "fixture".into(),
        messages,
        sender,
        None,
    )
    .await;
    while let Some(event) = receiver.recv().await {
        session.apply(event.attempt, event.update);
    }
    fixture.count("POST /api/chat")
}

#[tokio::test]
async fn a_proposal_ends_the_turn_and_prefills_the_prompt() {
    let mut session = Session::new("fixture".into());
    let chats = turn(&mut session, "refactor the whole ui").await;
    assert_eq!(chats, 1, "the turn ends after the proposal");
    assert_eq!(session.status, Status::Complete);
    assert_eq!(session.draft, "/plan Split ui.rs into modules");
    assert_eq!(session.cursor(), session.draft.len());
    let reply = &session.messages[1].content;
    assert!(reply.contains("/plan Split ui.rs into modules"), "{reply}");
    assert!(!reply.contains("SHOULD_NOT_CONTINUE"));
    assert_eq!(session.messages.len(), 2, "nothing was sent or started");
}

#[test]
fn format_and_bidi_characters_are_dropped_from_the_goal() {
    let root = workspace("bidi");
    let goal = "safe\u{202E}txt.exe\u{200B}\u{2066}x\u{2069}\u{FEFF}\u{200F}\u{00AD}";
    let result = chat_tools::dispatch(
        &root,
        &call(json!({"goal": goal, "mode": "plan", "reason": "r\u{202E}"})),
    );
    assert!(
        result.starts_with("proposed: /plan safetxt.exex\n"),
        "{result:?}"
    );
    assert!(!result.contains('\u{202E}'));
}

#[tokio::test]
async fn other_calls_beside_a_proposal_are_not_run() {
    let root = workspace("mixed");
    fs::write(root.join("a.txt"), "old\n").unwrap();
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| {
        Reply::Stream(
            vec![
                format!(
                    "{}\n",
                    json!({"message":{"role":"assistant","content":"","tool_calls":[
                        {"function":{"name":"write_file","arguments":{"path":"a.txt","content":"new\n"}}},
                        {"function":{"name":"propose_plan","arguments":{"goal":"Big","mode":"go","reason":"r"}}},
                        {"function":{"name":"read_file","arguments":{"path":"a.txt"}}},
                        {"function":{"name":"run_command","arguments":{"command":"echo new > a.txt","escalate":true}}}
                    ]},"done":false})
                )
                .into_bytes(),
                format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
            ],
            Duration::ZERO,
        )
    });
    let mut session = Session::new("fixture".into());
    session.insert("go big");
    let messages = session.begin().unwrap();
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    let (gate_sender, mut requests) = mpsc::channel(4);
    let gate = chat_tools::approval::Gate::new(gate_sender, None);
    let attempt = session.attempt;
    let asked = tokio::spawn(async move { requests.recv().await.is_some() });
    chat_tools::run_turn(
        &provider,
        &root,
        0,
        attempt,
        "fixture".into(),
        messages,
        sender,
        Some(gate),
    )
    .await;
    while let Some(event) = receiver.recv().await {
        session.apply(event.attempt, event.update);
    }
    assert!(!asked.await.unwrap(), "no approval card was raised");
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "old\n");
    let results = &session.tool_steps(1)[0].results;
    assert_eq!(results.len(), 4);
    assert!(results[0].starts_with("error: not run: proposal ends the turn"));
    assert!(results[1].starts_with("proposed: /go Big"));
    assert!(results[2].starts_with("error: not run: proposal ends the turn"));
    assert!(results[3].starts_with("error: not run: proposal ends the turn"));
    assert_eq!(session.draft, "/go Big");
}

#[tokio::test]
async fn a_proposal_never_overwrites_text_the_owner_typed() {
    let mut session = Session::new("fixture".into());
    let root = workspace("typed");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| {
        tool_call(json!({"goal": "Big change", "mode": "go", "reason": "risky"}))
    });
    session.insert("do the big change");
    let messages = session.begin().unwrap();
    session.insert("my own follow-up");
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    let attempt = session.attempt;
    chat_tools::run_turn(
        &provider,
        &root,
        0,
        attempt,
        "fixture".into(),
        messages,
        sender,
        None,
    )
    .await;
    while let Some(event) = receiver.recv().await {
        session.apply(event.attempt, event.update);
    }
    assert_eq!(session.status, Status::Complete);
    assert_eq!(session.draft, "my own follow-up");
    assert!(session.messages[1].content.contains("/go Big change"));
}
