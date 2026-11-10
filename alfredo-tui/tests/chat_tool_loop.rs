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

const TEXT_CALL_NOTICE: &str = "The model wrote a tool call as text; nothing ran. Ctrl+R retries.";

/// Run one turn whose only reply is `text`; the notices, the reply and whether any tool
/// event or file write happened (issue #660).
async fn text_reply(tag: &str, text: &str) -> (Vec<String>, String, bool, usize) {
    let root = workspace(tag);
    let addr = ollama_fixture::reserve();
    let reply = text.to_owned();
    let fixture = ollama_fixture::serve(addr, move |_, _| done(&reply));
    let mut session = Session::new("fixture".into());
    session.insert("hi");
    let messages = session.begin().unwrap();
    let events = run(&fixture.endpoint, &root, messages).await;
    let mut notices = Vec::new();
    let mut ran = false;
    for event in events {
        match &event.update {
            Update::Notice(text) => notices.push(text.clone()),
            Update::ToolStep(_) | Update::ToolResult { .. } | Update::RuleRunning { .. } => {
                ran = true;
            }
            _ => {}
        }
        session.apply(event.attempt, event.update);
    }
    assert_eq!(session.status, Status::Complete);
    let wrote = root.join("pwned.txt").exists();
    let requests = fixture.count("POST /api/chat");
    let _ = fs::remove_dir_all(root);
    (
        notices,
        session.messages[1].content.clone(),
        ran || wrote,
        requests,
    )
}

#[tokio::test]
async fn a_whole_reply_json_tool_call_gets_the_notice_and_nothing_runs() {
    let call = r#"{"name": "write_file", "arguments": {"path": "pwned.txt", "content": "x"}}"#;
    let (notices, reply, ran, requests) = text_reply("text-json", &format!("\n {call}\n")).await;
    assert_eq!(notices, [TEXT_CALL_NOTICE]);
    assert_eq!(reply, format!("\n {call}\n"), "the reply stays as written");
    assert!(!ran, "a text call must never run a tool");
    assert_eq!(requests, 1, "no re-prompt");
}

#[tokio::test]
async fn tool_call_blocks_naming_registered_tools_get_the_notice_and_nothing_runs() {
    let one = r#"<tool_call>{"name":"write_file","arguments":{"path":"pwned.txt","content":"x"}}</tool_call>"#;
    let two = format!(
        "{one}\n<tool_call>\n{{\"name\":\"read_file\",\"arguments\":{{\"path\":\"README.md\"}}}}\n</tool_call>"
    );
    for (tag, text) in [("block-one", one.to_owned()), ("block-two", two)] {
        let (notices, _, ran, requests) = text_reply(tag, &text).await;
        assert_eq!(notices, [TEXT_CALL_NOTICE], "{text}");
        assert!(!ran);
        assert_eq!(requests, 1);
    }
}

#[tokio::test]
async fn prose_fences_unknown_tools_and_partial_blocks_leave_the_reply_unchanged() {
    let cases = [
        r#"Use {"name":"read_file","arguments":{"path":"a.txt"}} to read it."#,
        "```json\n{\"name\":\"read_file\",\"arguments\":{\"path\":\"a.txt\"}}\n```",
        r#"{"name":"launch_missiles","arguments":{}}"#,
        r#"<tool_call>{"name":"launch_missiles","arguments":{}}</tool_call>"#,
        r#"<tool_call>{"name":"read_file","arguments":{"path":"a"}}</tool_call> done"#,
        r#"Sure. <tool_call>{"name":"read_file","arguments":{"path":"a"}}</tool_call>"#,
        r#"<tool_call>{"name":"read_file","arguments":{"path":"a"}}"#,
        r#"{"name":"read_file","arguments":"a.txt"}"#,
        r#"{"name":"read_file"}"#,
        r#"[{"name":"read_file","arguments":{"path":"a.txt"}}]"#,
        "Plain answer.",
    ];
    for (index, text) in cases.iter().enumerate() {
        let (notices, reply, ran, _) = text_reply(&format!("plain-{index}"), text).await;
        assert!(notices.is_empty(), "{text}: {notices:?}");
        assert_eq!(reply, *text);
        assert!(!ran);
    }
}

#[tokio::test]
async fn a_configured_cap_stops_the_loop_after_that_many_rounds() {
    let root = workspace("cap-configured");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| tool_call("list_files", json!({})));
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5))
        .unwrap()
        .with_max_turns(Some(3));
    let mut session = Session::new("fixture".into());
    session.insert("loop forever");
    let messages = session.begin().unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    chat_tools::run_turn(
        &provider,
        &root,
        0,
        1,
        "fixture".into(),
        messages,
        sender,
        None,
    )
    .await;
    while let Ok(event) = receiver.try_recv() {
        session.apply(event.attempt, event.update);
    }
    assert_eq!(session.tool_steps(1).len(), 3);
    assert!(session.messages[1]
        .content
        .contains("[Stopped: tool step limit reached after 3 rounds; ask again to continue]"));
    assert_eq!(fixture.count("POST /api/chat"), 4);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn max_turns_accepts_one_to_thirty_two_and_names_the_range_otherwise() {
    assert_eq!(chat_tools::parse_max_turns(""), Ok(None));
    assert_eq!(chat_tools::parse_max_turns("1"), Ok(Some(1)));
    assert_eq!(chat_tools::parse_max_turns(" 32 "), Ok(Some(32)));
    for bad in ["0", "33", "-1", "many", "3.5"] {
        let error = chat_tools::parse_max_turns(bad).unwrap_err();
        assert!(error.contains("1 to 32") && error.contains(bad), "{error}");
    }
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

/// Issue #651: a long non-ASCII tool name is cut by bytes, so it stays a valid step.
#[tokio::test]
async fn a_long_non_ascii_tool_name_gets_an_unknown_tool_result_and_the_turn_completes() {
    let root = workspace("cjk-name");
    let addr = ollama_fixture::reserve();
    let name = "读".repeat(40);
    let fixture = ollama_fixture::serve(addr, {
        let name = name.clone();
        move |_, index| {
            if index == 0 {
                tool_call(&name, json!({}))
            } else {
                done("ok")
            }
        }
    });
    let mut session = Session::new("fixture".into());
    session.insert("go");
    let messages = session.begin().unwrap();
    finish(run(&fixture.endpoint, &root, messages).await, &mut session);
    assert_eq!(session.status, Status::Complete);
    let steps = session.tool_steps(1);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].results.len(), 1);
    assert!(
        steps[0].results[0].starts_with("error: unknown tool"),
        "{}",
        steps[0].results[0]
    );
    assert!(steps[0].calls[0].name.len() <= 64);
    let requests = fixture.requests.lock().unwrap();
    let second = serde_json::to_string(&requests[1].body["messages"]).unwrap();
    assert!(second.contains("unknown tool"), "{second}");
    let _ = fs::remove_dir_all(root);
}

/// Issue #651: whichever byte of a name falls on the 64-byte limit, the turn completes
/// with the cut name recorded (3-byte and 4-byte chars, limit inside or at a char edge).
#[tokio::test]
async fn names_whose_64th_byte_falls_inside_a_char_are_cut_and_the_turn_completes() {
    let names = [
        "读".repeat(21) + "读读",
        format!("{}读读", "x".repeat(62)),
        format!("a{}", "读".repeat(30)),
        "😀".repeat(20),
        format!("a{}", "😀".repeat(20)),
        format!("{}😀😀", "y".repeat(61)),
    ];
    for name in names {
        let root = workspace("cut-name");
        let addr = ollama_fixture::reserve();
        let sent = name.clone();
        let fixture = ollama_fixture::serve(addr, move |_, index| {
            if index == 0 {
                tool_call(&sent, json!({}))
            } else {
                done("ok")
            }
        });
        let mut session = Session::new("fixture".into());
        session.insert("go");
        let messages = session.begin().unwrap();
        finish(run(&fixture.endpoint, &root, messages).await, &mut session);
        assert_eq!(session.status, Status::Complete, "{name}");
        let steps = session.tool_steps(1);
        assert_eq!(steps.len(), 1, "{name}");
        let recorded = &steps[0].calls[0].name;
        assert!(recorded.len() <= 64, "{name}: {} bytes", recorded.len());
        assert!(!recorded.is_empty() && name.starts_with(recorded.as_str()));
        assert!(steps[0].results[0].starts_with("error: unknown tool"));
        let _ = fs::remove_dir_all(root);
    }
}

/// Issue #651: the name the loop records is the model's name without control characters,
/// cut to exactly 64 bytes, or `unnamed` when nothing is left.
#[tokio::test]
async fn recorded_tool_names_drop_control_chars_and_stop_at_64_bytes() {
    let cases = [
        ("a".repeat(100), "a".repeat(64)),
        ("a\u{7}b\n读".to_string(), "ab读".to_string()),
        ("\u{7}\u{1}".to_string(), "unnamed".to_string()),
        (format!("\u{7}{}", "读".repeat(30)), "读".repeat(21)),
        ("😀".repeat(16) + "x", "😀".repeat(16)),
    ];
    for (sent, expected) in cases {
        let root = workspace("clean-name");
        let addr = ollama_fixture::reserve();
        let name = sent.clone();
        let fixture = ollama_fixture::serve(addr, move |_, index| {
            if index == 0 {
                tool_call(&name, json!({}))
            } else {
                done("ok")
            }
        });
        let mut session = Session::new("fixture".into());
        session.insert("go");
        let messages = session.begin().unwrap();
        finish(run(&fixture.endpoint, &root, messages).await, &mut session);
        assert_eq!(session.status, Status::Complete, "{sent:?}");
        let steps = session.tool_steps(1);
        assert_eq!(steps.len(), 1, "{sent:?}");
        assert_eq!(steps[0].calls[0].name, expected, "{sent:?}");
        assert!(steps[0].results[0].starts_with("error: unknown tool"));
        let _ = fs::remove_dir_all(root);
    }
}

/// Issue #651: a saved or replayed step still needs a non-empty, control-free name of at
/// most 64 bytes; each rule alone rejects it.
#[test]
fn a_tool_result_whose_name_breaks_one_validity_rule_fails_the_turn() {
    let rejected = [
        "n".repeat(65),
        "读".repeat(22),
        String::new(),
        "bad\u{7}name".to_string(),
    ];
    for name in rejected {
        let mut session = Session::new("fixture".into());
        session.insert("go");
        session.begin().unwrap();
        session.apply(
            1,
            Update::ToolResult {
                call: ToolCall {
                    name: name.clone(),
                    arguments: json!({}),
                },
                result: "ok".into(),
                new_round: true,
            },
        );
        assert!(
            matches!(session.status, Status::Failed(_)),
            "{name:?}: {:?}",
            session.status
        );
        assert!(session.tool_steps(1).is_empty(), "{name:?}");
    }
    let mut session = Session::new("fixture".into());
    session.insert("go");
    session.begin().unwrap();
    session.apply(
        1,
        Update::ToolResult {
            call: ToolCall {
                name: "n".repeat(64),
                arguments: json!({}),
            },
            result: "ok".into(),
            new_round: true,
        },
    );
    assert!(!matches!(session.status, Status::Failed(_)));
    assert_eq!(session.tool_steps(1).len(), 1);
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

/// Like `run`, with the provider's `--num-ctx` set.
async fn run_ctx(
    endpoint: &str,
    root: &std::path::Path,
    messages: Vec<Message>,
    num_ctx: Option<u32>,
) -> Vec<Event> {
    let provider = Ollama::new(endpoint, Duration::from_secs(5))
        .unwrap()
        .with_num_ctx(num_ctx);
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

/// A model that keeps reading `path` while it is offered tools and answers once it is not.
fn reader(path: &'static str) -> impl Fn(&ollama_fixture::Request, usize) -> Reply {
    move |request, _| {
        if request.body.get("tools").is_some() {
            tool_call("read_file", json!({"path": path}))
        } else {
            done("final answer")
        }
    }
}

fn finish(events: Vec<Event>, session: &mut Session) {
    for event in events {
        session.apply(event.attempt, event.update);
    }
}

#[tokio::test]
async fn a_16kb_read_completes_under_num_ctx_4096() {
    let root = workspace("ctx4096");
    fs::write(root.join("big.rs"), "fn x() {}\n".repeat(1_700)).unwrap();
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |request, index| {
        if index == 0 {
            tool_call("read_file", json!({"path": "big.rs"}))
        } else {
            assert!(request.body.get("tools").is_some());
            done("final answer")
        }
    });
    let mut session = Session::new("fixture".into());
    session.insert("read big.rs");
    let messages = session.begin().unwrap();
    finish(
        run_ctx(&fixture.endpoint, &root, messages, Some(4096)).await,
        &mut session,
    );
    assert_eq!(session.status, Status::Complete, "{:?}", session.status);
    assert!(session.messages[1].content.contains("final answer"));
    let result = &session.tool_steps(1)[0].results[0];
    assert!(
        result.contains("[truncated at line"),
        "{}",
        &result[result.len() - 80..]
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn budgeted_read_file_ends_on_a_whole_line_with_the_continuation_hint() {
    let root = workspace("ctx4096hint");
    let body: String = (1..=8000)
        .map(|n| format!("line {n:05} some filler text here\n"))
        .collect();
    fs::write(root.join("big.txt"), body).unwrap();
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, index| {
        if index == 0 {
            tool_call("read_file", json!({"path": "big.txt"}))
        } else {
            done("final answer")
        }
    });
    let mut session = Session::new("fixture".into());
    session.insert("read big.txt");
    let messages = session.begin().unwrap();
    finish(
        run_ctx(&fixture.endpoint, &root, messages, Some(4096)).await,
        &mut session,
    );
    let result = &session.tool_steps(1)[0].results[0];
    let (shown, footer) = result.rsplit_once('\n').unwrap();
    let last: usize = shown
        .rsplit('\n')
        .next()
        .unwrap()
        .split('\t')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        footer,
        format!(
            "[truncated at line {last} of 8000; call read_file with start_line={} to continue]",
            last + 1
        )
    );
    assert!(shown.ends_with("some filler text here"), "{shown}");
    assert!(
        last > 5 && result.len() <= 4096 * 3 / 2,
        "{} bytes",
        result.len()
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn budgeted_grep_is_cut_on_a_newline() {
    let root = workspace("ctx4096grep");
    let body: String = (1..=400)
        .map(|n| format!("needle line {n:05} some filler text here\n"))
        .collect();
    fs::write(root.join("big.txt"), body).unwrap();
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, index| {
        if index == 0 {
            tool_call("grep", json!({"pattern": "needle"}))
        } else {
            done("final answer")
        }
    });
    let mut session = Session::new("fixture".into());
    session.insert("grep needle");
    let messages = session.begin().unwrap();
    finish(
        run_ctx(&fixture.endpoint, &root, messages, Some(4096)).await,
        &mut session,
    );
    let result = &session.tool_steps(1)[0].results[0];
    let (shown, footer) = result.rsplit_once('\n').unwrap();
    assert!(footer.starts_with("[truncated: "), "{footer}");
    assert!(shown.ends_with("some filler text here"), "{shown}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn repeated_reads_under_num_ctx_8192_end_with_an_answer() {
    let root = workspace("bigloop");
    let lines: String = (1..=5_000).map(|n| format!("line {n}\n")).collect();
    fs::write(root.join("big.txt"), lines).unwrap();
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, reader("big.txt"));
    let mut session = Session::new("fixture".into());
    session.insert("read big.txt again and again");
    let messages = session.begin().unwrap();
    finish(
        run_ctx(&fixture.endpoint, &root, messages, Some(8192)).await,
        &mut session,
    );
    assert_eq!(session.status, Status::Complete, "{:?}", session.status);
    assert!(session.messages[1].content.contains("final answer"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn eight_large_reads_at_the_default_window_end_with_an_answer() {
    let root = workspace("eight");
    let lines: String = (1..=20_000).map(|n| format!("line {n}\n")).collect();
    fs::write(root.join("huge.txt"), lines).unwrap();
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, reader("huge.txt"));
    let mut session = Session::new("fixture".into());
    session.insert("read huge.txt");
    let messages = session.begin().unwrap();
    finish(run(&fixture.endpoint, &root, messages).await, &mut session);
    assert_eq!(session.status, Status::Complete, "{:?}", session.status);
    assert!(session.messages[1].content.contains("final answer"));
    // The squeezed round is followed by one request without tools, never by a failure.
    let requests = fixture.requests.lock().unwrap();
    assert!(requests.len() <= MAX_STEPS + 1);
    assert!(requests.last().unwrap().body.get("tools").is_none());
    drop(requests);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_result_with_no_room_left_becomes_a_note_and_the_model_answers() {
    let root = workspace("note");
    fs::write(root.join("big.txt"), "b".repeat(15_000)).unwrap();
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, reader("big.txt"));
    let mut session = Session::new("fixture".into());
    session.insert("go");
    let mut messages = session.begin().unwrap();
    messages.insert(
        0,
        Message {
            role: "user".into(),
            content: "h".repeat(127_000),
        },
    );
    finish(run(&fixture.endpoint, &root, messages).await, &mut session);
    assert_eq!(session.status, Status::Complete, "{:?}", session.status);
    let result = &session.tool_steps(1)[0].results[0];
    assert!(result.contains("near its size limit"), "{result}");
    assert!(session.messages[1].content.contains("final answer"));
    assert_eq!(fixture.count("POST /api/chat"), 2);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn num_ctx_estimate_counts_the_tool_definitions() {
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| done("ok"));
    let tools = chat_tools::definitions();
    let tool_bytes = serde_json::to_string(&tools).unwrap().len();
    assert!(tool_bytes > 3_000, "{tool_bytes}");
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5))
        .unwrap()
        .with_num_ctx(Some(2048))
        .with_tools(tools);
    // Messages alone (about 1,000 tokens) fit; with the tools they pass the window.
    let content = "m".repeat(2_900);
    let messages = vec![Message {
        role: "user".into(),
        content,
    }];
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
    let failure = failure.expect("refused once the tools are counted");
    assert!(failure.contains("--num-ctx 2048"), "{failure}");
    assert!(failure.contains("/compact"), "{failure}");
    assert_eq!(fixture.count("POST /api/chat"), 0);
}

#[tokio::test]
async fn a_window_too_small_for_the_tools_runs_the_turn_as_plain_chat() {
    let root = workspace("tiny");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| done("plain answer"));
    let mut session = Session::new("fixture".into());
    session.insert("hi");
    let messages = session.begin().unwrap();
    finish(
        run_ctx(&fixture.endpoint, &root, messages, Some(2048)).await,
        &mut session,
    );
    assert_eq!(session.status, Status::Complete, "{:?}", session.status);
    assert!(session.messages[1].content.contains("plain answer"));
    assert!(session.messages[1]
        .content
        .contains("too small for the file tools"));
    assert!(fixture.requests.lock().unwrap()[0]
        .body
        .get("tools")
        .is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_works_from_a_failed_tool_turn() {
    let mut session = Session::new("fixture".into());
    for n in 0..3 {
        session.insert(&format!("question {n}"));
        session.begin().unwrap();
        let attempt = session.attempt;
        session.apply(attempt, Update::Token(format!("answer {n}")));
        session.apply(attempt, Update::Done);
    }
    session.insert("read big.rs");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(
        attempt,
        Update::ToolStep(alfredo_tui::model::ToolStep {
            calls: vec![ToolCall {
                name: "read_file".into(),
                arguments: json!({"path": "big.rs"}),
            }],
            results: vec!["x".repeat(16_000)],
        }),
    );
    session.apply(
        attempt,
        Update::Failed("prompt ≈5471 tokens > --num-ctx 4096; run /compact".into()),
    );
    assert!(matches!(session.status, Status::Failed(_)));
    let plan = session
        .start_compaction(Some(4096), true)
        .expect("compactable");
    let attempt = session.attempt;
    session
        .finish_compaction(attempt, &plan, Ok("summary".into()))
        .expect("compacted");
    assert!(session.compaction().is_some());
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

async fn run_as(
    endpoint: &str,
    root: &std::path::Path,
    model: &str,
    messages: Vec<Message>,
) -> Vec<Event> {
    let provider = Ollama::new(endpoint, Duration::from_secs(5)).unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    chat_tools::run_turn(&provider, root, 0, 1, model.into(), messages, sender, None).await;
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    events
}

fn begin(session: &mut Session, text: &str) -> Vec<Message> {
    session.insert(text);
    let mut messages = session.begin().unwrap();
    messages.insert(0, alfredo_tui::chat_context::system_message(None, |_| None));
    messages
}

#[tokio::test]
async fn a_model_without_tool_support_falls_back_to_plain_chat_once() {
    let root = workspace("notools");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |request, _| {
        if request.body.get("tools").is_some() {
            Reply::Json(
                400,
                r#"{"error":"registry.ollama.ai/library/gemma:2b does not support tools"}"#.into(),
            )
        } else {
            done("plain hello")
        }
    });
    let model = "notools-model";
    let mut session = Session::new(model.into());
    let messages = begin(&mut session, "hi");
    let events = run_as(&fixture.endpoint, &root, model, messages).await;
    let notices: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.update {
            Update::Notice(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        notices,
        [format!(
            "Model {model} has no tool support; chat is running without file tools"
        )]
    );
    for event in events {
        session.apply(event.attempt, event.update);
    }
    let reply = &session.messages.last().unwrap().content;
    assert!(reply.contains("plain hello"), "{reply}");
    // Issue #281: the notice is a notice, never assistant text.
    assert!(!reply.contains("no tool support"), "{reply}");
    assert_eq!(reply.trim(), "plain hello");
    assert!(!matches!(session.status, Status::Failed(_)));
    let requests = fixture.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].body.get("tools").is_some());
    assert!(requests[1].body.get("tools").is_none());
    let system = requests[1].body["messages"][0]["content"].as_str().unwrap();
    assert!(!system.contains("read_file"), "{system}");
    assert!(system.contains("no tools"), "{system}");

    // The next request's history carries no notice (issue #281).
    session.insert("second one");
    let history = session.begin().unwrap();
    assert!(
        history
            .iter()
            .all(|m| !m.content.contains("no tool support")),
        "{history:?}"
    );
    assert_eq!(history.len(), 3, "{history:?}");

    // A later turn skips tools without probing and shows no second notice.
    let mut later = Session::new(model.into());
    let messages = begin(&mut later, "again");
    for event in run_as(&fixture.endpoint, &root, model, messages).await {
        later.apply(event.attempt, event.update);
    }
    let reply = &later.messages.last().unwrap().content;
    assert!(!reply.contains("no tool support"), "{reply}");
    let requests = fixture.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].body.get("tools").is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_notice_reaches_the_footer_and_never_the_transcript() {
    let mut app = alfredo_tui::model::App::new("m".into());
    app.sessions[0].insert("hi");
    app.sessions[0].begin().unwrap();
    let attempt = app.sessions[0].attempt;
    app.apply(Event {
        session: 0,
        attempt,
        update: Update::Notice("Model m has no tool support".into()),
    });
    assert_eq!(app.notice, "Model m has no tool support");
    assert!(app.sessions[0].messages.last().unwrap().content.is_empty());
    app.notice.clear();
    app.apply(Event {
        session: 0,
        attempt: attempt + 1,
        update: Update::Notice("stale".into()),
    });
    assert!(app.notice.is_empty());
}

#[tokio::test]
async fn another_400_shows_ollamas_error_text_and_is_not_retried_without_tools() {
    let root = workspace("other400");
    let addr = ollama_fixture::reserve();
    let long = "x".repeat(400);
    let body = json!({"error": format!("context\u{7} length exceeded {long}")}).to_string();
    let fixture = ollama_fixture::serve(addr, move |_, _| Reply::Json(400, body.clone()));
    let mut session = Session::new("other400-model".into());
    let messages = begin(&mut session, "hi");
    for event in run_as(&fixture.endpoint, &root, "other400-model", messages).await {
        session.apply(event.attempt, event.update);
    }
    let Status::Failed(error) = &session.status else {
        panic!("{:?}", session.status);
    };
    assert!(
        error.contains("HTTP 400: context length exceeded"),
        "{error}"
    );
    assert!(!error.chars().any(|c| c.is_control()), "{error:?}");
    assert!(error.chars().count() < 260, "{error}");
    assert_eq!(fixture.count("POST /api/chat"), 1);
    let _ = fs::remove_dir_all(root);
}

/// A reply asking for several tools at once.
fn tool_calls(calls: &[(&str, serde_json::Value)]) -> Reply {
    let calls: Vec<_> = calls
        .iter()
        .map(|(name, arguments)| json!({"function":{"name":name,"arguments":arguments}}))
        .collect();
    Reply::Stream(
        vec![
            format!(
                "{}\n",
                json!({"message":{"role":"assistant","content":"","tool_calls":calls},"done":false})
            )
            .into_bytes(),
            format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )
}

/// Issue #257: a round whose read time is spent skips the reads not reached.
#[tokio::test]
async fn a_read_call_past_the_round_time_limit_is_not_run() {
    let root = workspace("round-limit");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, index| {
        if index == 0 {
            tool_call("read_file", json!({"path": "README.md"}))
        } else {
            done("ok")
        }
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let mut session = Session::new("fixture".into());
    session.insert("read");
    let messages = session.begin().unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    chat_tools::run_turn_limited(
        &provider,
        &root,
        0,
        1,
        "fixture".into(),
        messages,
        sender,
        None,
        Duration::ZERO,
    )
    .await;
    while let Ok(event) = receiver.try_recv() {
        session.apply(event.attempt, event.update);
    }
    let steps = session.tool_steps(1);
    assert_eq!(
        steps[0].results[0],
        "error: not run; round time limit reached"
    );
    let _ = fs::remove_dir_all(root);
}

/// Issue #257: owner think time on an approval card does not use up the read budget.
#[tokio::test]
async fn a_slow_approval_does_not_exhaust_the_round_read_budget() {
    use alfredo_tui::chat_tools::approval::{self, Decision, Gate};
    let root = workspace("slow-approval");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, index| {
        if index == 0 {
            tool_calls(&[
                ("write_file", json!({"path": "new.txt", "content": "one"})),
                ("read_file", json!({"path": "README.md"})),
            ])
        } else {
            done("ok")
        }
    });
    let (gate_sender, mut cards) = mpsc::channel::<approval::Request>(4);
    tokio::spawn(async move {
        while let Some(request) = cards.recv().await {
            // Far longer than the 300 ms round limit used below.
            tokio::time::sleep(Duration::from_millis(900)).await;
            let _ = request.reply.send(Decision::Approve);
        }
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let mut session = Session::new("fixture".into());
    session.insert("write then read");
    let messages = session.begin().unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    chat_tools::run_turn_limited(
        &provider,
        &root,
        0,
        1,
        "fixture".into(),
        messages,
        sender,
        Some(Gate::new(gate_sender, None)),
        Duration::from_millis(300),
    )
    .await;
    while let Ok(event) = receiver.try_recv() {
        session.apply(event.attempt, event.update);
    }
    let steps = session.tool_steps(1);
    assert!(
        steps[0].results[1].contains("README_BODY_MARKER"),
        "{:?}",
        steps[0].results
    );
    assert_eq!(fs::read_to_string(root.join("new.txt")).unwrap(), "one");
    let _ = fs::remove_dir_all(root);
}

/// Issue #271: a two-call round whose first call was approved and applied and whose
/// second card was cancelled keeps the applied call in the transcript and in the
/// next request; the unanswered call is in neither.
#[tokio::test]
async fn an_applied_call_survives_a_cancelled_round_and_reaches_the_next_request() {
    use alfredo_tui::chat_tools::approval::{self, Decision, Gate};
    let root = workspace("cancelled-round");
    let addr = ollama_fixture::reserve();
    let fixture = ollama_fixture::serve(addr, |_, _| {
        tool_calls(&[
            ("write_file", json!({"path": "big.txt", "content": "one"})),
            (
                "write_file",
                json!({"path": "second.txt", "content": "two"}),
            ),
        ])
    });
    // Approve the first card; leave the second unanswered, as when Esc is pressed.
    let (gate_sender, mut cards) = mpsc::channel::<approval::Request>(4);
    tokio::spawn(async move {
        let mut asked = 0;
        let mut held = Vec::new();
        while let Some(request) = cards.recv().await {
            asked += 1;
            if asked == 1 {
                let _ = request.reply.send(Decision::Approve);
            } else {
                held.push(request);
            }
        }
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let mut session = Session::new("fixture".into());
    session.insert("make files");
    let messages = session.begin().unwrap();
    let (sender, mut receiver) = mpsc::channel::<Event>(256);
    let worker_root = root.clone();
    let job = tokio::spawn(async move {
        chat_tools::run_turn(
            &provider,
            &worker_root,
            0,
            1,
            "fixture".into(),
            messages,
            sender,
            Some(Gate::new(gate_sender, None)),
        )
        .await;
    });
    // Apply events until the applied call shows in the session; the turn is then
    // waiting on the second card.
    let seen = tokio::time::timeout(Duration::from_secs(3), async {
        while !session
            .tool_steps(1)
            .iter()
            .any(|step| step.calls.iter().any(|call| call.name == "write_file"))
        {
            let Some(event) = receiver.recv().await else {
                return false;
            };
            session.apply(event.attempt, event.update);
        }
        true
    })
    .await;
    assert_eq!(seen, Ok(true), "the applied call never reached the session");
    job.abort();
    session.cancel();
    assert_eq!(fs::read_to_string(root.join("big.txt")).unwrap(), "one");
    assert!(!root.join("second.txt").exists());
    let steps = session.tool_steps(1);
    assert_eq!(steps.iter().map(|s| s.calls.len()).sum::<usize>(), 1);
    assert_eq!(steps[0].calls[0].arguments["path"], "big.txt");

    session.insert("hello");
    let request = session.begin().unwrap();
    let text: Vec<String> = request
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect();
    assert!(
        text.iter().any(|m| m.starts_with("user: make files")),
        "{text:?}"
    );
    let applied = text
        .iter()
        .filter(|m| m.contains("big.txt"))
        .collect::<Vec<_>>();
    assert!(!applied.is_empty(), "applied step missing: {text:?}");
    assert!(
        !text.iter().any(|m| m.contains("second.txt")),
        "unanswered call leaked: {text:?}"
    );
    assert_eq!(text.last().unwrap(), "user: hello");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_text_tool_call_reply_can_be_retried_with_ctrl_r_but_a_plain_one_cannot() {
    for (text, retryable) in [
        (r#"{"name":"read_file","arguments":{"path":"a.txt"}}"#, true),
        ("Plain answer.", false),
    ] {
        let root = workspace("text-retry");
        let addr = ollama_fixture::reserve();
        let reply = text.to_owned();
        let fixture = ollama_fixture::serve(addr, move |_, _| done(&reply));
        let mut session = Session::new("fixture".into());
        session.insert("hi");
        let messages = session.begin().unwrap();
        for event in run(&fixture.endpoint, &root, messages).await {
            session.apply(event.attempt, event.update);
        }
        assert_eq!(session.ends_in_text_tool_call(), retryable, "{text}");
        assert_eq!(session.retry().is_ok(), retryable, "{text}");
        let _ = fs::remove_dir_all(root);
    }
}
