//! Issue #233: provider tool calls and the capped call -> result -> continue loop.
mod ollama_fixture;

use alfredo_tui::{
    chat_tools::{self, MAX_STEPS, STEP_CAP_NOTICE},
    model::{Event, Message, Session, Status, ToolCall, Update},
    provider::Ollama,
};
use ollama_fixture::{done, Reply};
use serde_json::json;
use std::{fs, path::PathBuf, time::Duration};
use tokio::sync::mpsc;

fn tool_call(name: &str, arguments: serde_json::Value) -> Reply {
    Reply::Stream(
        vec![
            format!(
                "{}\n",
                json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":name,"arguments":arguments}}]},"done":false})
            )
            .into_bytes(),
            format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )
}

fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("alfredo-tool-loop-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("README.md"),
        "# Demo\nREADME_BODY_MARKER explains the demo.\n",
    )
    .unwrap();
    root
}

async fn run(endpoint: &str, root: &std::path::Path, messages: Vec<Message>) -> Vec<Event> {
    let provider = Ollama::new(endpoint, Duration::from_secs(5)).unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    chat_tools::run_turn(
        &provider,
        root,
        0,
        1,
        "fixture".into(),
        messages,
        sender,
        None,
    )
    .await;
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn tool_call_frames_parse_into_events_and_requests_carry_tools() {
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| tool_call("list_files", json!({})));
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5))
        .unwrap()
        .with_tools(chat_tools::definitions());
    let (sender, mut receiver) = mpsc::channel(64);
    provider
        .chat(
            0,
            1,
            "fixture".into(),
            vec![Message {
                role: "user".into(),
                content: "list".into(),
            }],
            sender,
        )
        .await;
    let mut calls = Vec::new();
    let mut finished = false;
    while let Some(event) = receiver.recv().await {
        match event.update {
            Update::ToolCalls(found) => calls.extend(found),
            Update::Done => finished = true,
            Update::Failed(error) => panic!("{error}"),
            _ => {}
        }
    }
    assert!(finished);
    assert_eq!(
        calls,
        [ToolCall {
            name: "list_files".into(),
            arguments: json!({})
        }]
    );
    let body = &fixture.requests.lock().unwrap()[0].body;
    assert_eq!(body["tools"], json!(chat_tools::definitions()));
}

/// Issue #253: a chat round writes straight to the caller's sender (no forwarding
/// hop in `run_turn`); a tool-less round delivers `Done` on that same path, and a
/// round that asks for tools returns its calls and withholds `Done`.
#[tokio::test]
async fn chat_round_writes_to_the_callers_sender_without_a_forwarding_hop() {
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| done("hello"));
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let (sender, mut receiver) = mpsc::channel(64);
    let calls = provider
        .chat_round(0, 1, "fixture".into(), vec![], sender)
        .await;
    assert_eq!(calls, Some(vec![]));
    // The sender was moved in and dropped on return, so everything the round
    // produced is already queued on the caller's channel, `Done` last.
    let mut updates = Vec::new();
    while let Some(event) = receiver.recv().await {
        updates.push(event.update);
    }
    assert!(matches!(updates.last(), Some(Update::Done)));
    assert_eq!(
        updates.iter().filter(|u| matches!(u, Update::Done)).count(),
        1
    );
    assert!(updates
        .iter()
        .any(|u| matches!(u, Update::Token(t) if t == "hello")));

    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| tool_call("list_files", json!({})));
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let (sender, mut receiver) = mpsc::channel(64);
    let calls = provider
        .chat_round(0, 1, "fixture".into(), vec![], sender)
        .await;
    assert_eq!(
        calls,
        Some(vec![ToolCall {
            name: "list_files".into(),
            arguments: json!({})
        }])
    );
    while let Some(event) = receiver.recv().await {
        assert!(
            !matches!(event.update, Update::Done | Update::ToolCalls(_)),
            "a tool round must not emit Done or ToolCalls"
        );
    }
}

#[tokio::test]
async fn plain_providers_send_no_tools() {
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| done("ok"));
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let (sender, mut receiver) = mpsc::channel(64);
    provider
        .chat(
            0,
            1,
            "fixture".into(),
            vec![Message {
                role: "user".into(),
                content: "x".into(),
            }],
            sender,
        )
        .await;
    while receiver.recv().await.is_some() {}
    assert!(fixture.requests.lock().unwrap()[0]
        .body
        .get("tools")
        .is_none());
}

#[tokio::test]
async fn summarise_readme_is_answered_from_a_read_file_round_trip() {
    let root = workspace("demo");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |request, index| {
        if index == 0 {
            tool_call("read_file", json!({"path": "README.md"}))
        } else {
            let messages = request.body["messages"].as_array().unwrap();
            let seen = messages.iter().any(|m| {
                m["role"] == "tool"
                    && m["content"]
                        .as_str()
                        .unwrap()
                        .contains("README_BODY_MARKER")
            });
            done(if seen {
                "README summary: a demo."
            } else {
                "NO_TOOL_RESULT"
            })
        }
    });
    let mut session = Session::new("fixture".into());
    session.insert("summarise README");
    let messages = session.begin().unwrap();
    let events = run(&fixture.endpoint, &root, messages).await;
    for event in events {
        session.apply(event.attempt, event.update);
    }
    assert_eq!(session.status, Status::Complete);
    assert_eq!(session.messages[1].content, "README summary: a demo.");
    let steps = session.tool_steps(1);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].calls[0].name, "read_file");
    assert!(steps[0].results[0].contains("README_BODY_MARKER"));
    let requests = fixture.requests.lock().unwrap();
    let chats: Vec<_> = requests
        .iter()
        .filter(|r| r.path == "POST /api/chat")
        .collect();
    assert_eq!(chats.len(), 2);
    let second = chats[1].body["messages"].as_array().unwrap();
    let call = &second[second.len() - 2];
    assert_eq!(call["role"], "assistant");
    assert_eq!(call["tool_calls"][0]["function"]["name"], "read_file");
    assert_eq!(second[second.len() - 1]["role"], "tool");
    assert_eq!(second[second.len() - 1]["tool_name"], "read_file");
    // A follow-up turn still carries the earlier tool exchange as context.
    session.insert("and again?");
    let follow = session.begin().unwrap();
    assert!(follow
        .iter()
        .any(|m| m.role == "tool" && m.content.contains("README_BODY_MARKER")));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn the_loop_stops_at_the_step_cap_with_a_visible_notice() {
    let root = workspace("cap");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| tool_call("list_files", json!({})));
    let mut session = Session::new("fixture".into());
    session.insert("loop forever");
    let messages = session.begin().unwrap();
    let events = run(&fixture.endpoint, &root, messages).await;
    for event in events {
        session.apply(event.attempt, event.update);
    }
    assert_eq!(session.status, Status::Complete);
    assert_eq!(session.tool_steps(1).len(), MAX_STEPS);
    assert!(session.messages[1].content.contains(STEP_CAP_NOTICE));
    assert_eq!(fixture.count("POST /api/chat"), MAX_STEPS + 1);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn string_arguments_are_stored_and_replayed_as_objects() {
    let root = workspace("strargs");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, index| {
        if index == 0 {
            tool_call("read_file", json!("{\"path\":\"README.md\"}"))
        } else {
            done("ok")
        }
    });
    let mut session = Session::new("fixture".into());
    session.insert("go");
    let messages = session.begin().unwrap();
    for event in run(&fixture.endpoint, &root, messages).await {
        session.apply(event.attempt, event.update);
    }
    assert_eq!(
        session.tool_steps(1)[0].calls[0].arguments,
        json!({"path": "README.md"})
    );
    let requests = fixture.requests.lock().unwrap();
    let second = requests[1].body["messages"].as_array().unwrap();
    let call = &second[second.len() - 2];
    assert_eq!(
        call["tool_calls"][0]["function"]["arguments"],
        json!({"path": "README.md"})
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn calls_past_the_per_round_limit_get_error_results() {
    let root = workspace("many");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, index| {
        if index == 0 {
            let calls: Vec<_> = (0..20)
                .map(|_| json!({"function":{"name":"list_files","arguments":{}}}))
                .collect();
            Reply::Stream(
                vec![
                    format!(
                        "{}\n",
                        json!({"message":{"content":"","tool_calls":calls},"done":false})
                    )
                    .into_bytes(),
                    format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
                ],
                Duration::ZERO,
            )
        } else {
            done("ok")
        }
    });
    let mut session = Session::new("fixture".into());
    session.insert("go");
    let messages = session.begin().unwrap();
    for event in run(&fixture.endpoint, &root, messages).await {
        session.apply(event.attempt, event.update);
    }
    assert_eq!(session.status, Status::Complete);
    let step = &session.tool_steps(1)[0];
    assert_eq!(step.calls.len(), 20);
    assert_eq!(step.results.len(), 20);
    assert!(
        step.results[19].starts_with("error: "),
        "{}",
        step.results[19]
    );
    assert!(!step.results[0].starts_with("error: "));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn the_loop_stops_when_a_step_would_pass_the_conversation_limit() {
    let root = workspace("limit");
    fs::write(root.join("big.txt"), "b".repeat(15_000)).unwrap();
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| {
        tool_call("read_file", json!({"path": "big.txt"}))
    });
    let mut session = Session::new("fixture".into());
    session.insert(&"p".repeat(16_000));
    let mut messages = session.begin().unwrap();
    // Earlier history held by the caller: close to the 128 KiB limit.
    messages.insert(
        0,
        Message {
            role: "user".into(),
            content: "h".repeat(120_000),
        },
    );
    for event in run(&fixture.endpoint, &root, messages).await {
        session.apply(event.attempt, event.update);
    }
    assert!(
        matches!(session.status, Status::Failed(_)),
        "{:?}",
        session.status
    );
    assert_eq!(fixture.count("POST /api/chat"), 1);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn num_ctx_accounting_counts_tool_messages() {
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| done("ok"));
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5))
        .unwrap()
        .with_num_ctx(Some(2048));
    let messages = vec![
        Message {
            role: "user".into(),
            content: "read it".into(),
        },
        Message {
            role: alfredo_tui::model::TOOL_CALLS_ROLE.into(),
            content: json!([{"name":"read_file","arguments":{"path":"a"}}]).to_string(),
        },
        Message {
            role: "tool".into(),
            content: "t".repeat(9000),
        },
    ];
    let (sender, mut receiver) = mpsc::channel(64);
    provider
        .chat(0, 1, "fixture".into(), messages, sender)
        .await;
    let mut failure = None;
    while let Some(event) = receiver.recv().await {
        if let Update::Failed(error) = event.update {
            failure = Some(error);
        }
    }
    assert!(failure.unwrap().contains("--num-ctx 2048"));
    assert_eq!(fixture.count("POST /api/chat"), 0);
}

#[tokio::test]
async fn a_failed_model_request_ends_the_turn_without_more_steps() {
    let root = workspace("fail");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, index| {
        if index == 0 {
            tool_call("list_files", json!({}))
        } else {
            Reply::Json(400, "{}".into())
        }
    });
    let mut session = Session::new("fixture".into());
    session.insert("go");
    let messages = session.begin().unwrap();
    for event in run(&fixture.endpoint, &root, messages).await {
        session.apply(event.attempt, event.update);
    }
    assert!(matches!(session.status, Status::Failed(_)));
    assert_eq!(fixture.count("POST /api/chat"), 2);
    let _ = fs::remove_dir_all(root);
}
