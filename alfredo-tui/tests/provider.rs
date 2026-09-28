use alfredo_tui::{
    model::{Event, Message, Update},
    provider::Ollama,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};
use tokio::sync::mpsc;

fn server(body: Vec<Vec<u8>>, pause: Duration) -> (String, thread::JoinHandle<()>) {
    let (endpoint, handle, _) = server_capture(body, pause);
    (endpoint, handle)
}
fn server_capture(
    body: Vec<Vec<u8>>,
    pause: Duration,
) -> (
    String,
    thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<serde_json::Value>,
) {
    let (sender, receiver) = std::sync::mpsc::channel();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let headers = String::from_utf8(request).unwrap();
        assert!(headers.starts_with("POST /api/chat HTTP/1.1"));
        let length: usize = headers
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .unwrap()
            .parse()
            .unwrap();
        let mut request = vec![0; length];
        stream.read_exact(&mut request).unwrap();
        let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
        assert_eq!(request["stream"], true);
        assert_eq!(request["model"], "fixture");
        let _ = sender.send(request);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n").unwrap();
        for bytes in body {
            if stream.write_all(&bytes).is_err() {
                break;
            }
            let _ = stream.flush();
            thread::sleep(pause);
        }
    });
    (endpoint, handle, receiver)
}

async fn collect(body: Vec<Vec<u8>>, pause: Duration, timeout: Duration) -> Vec<Event> {
    let (endpoint, server) = server(body, pause);
    let provider = Ollama::new(&endpoint, timeout).unwrap();
    let (sender, mut receiver) = mpsc::channel(128);
    provider
        .chat(
            4,
            7,
            "fixture".into(),
            vec![Message {
                role: "user".into(),
                content: "hello".into(),
            }],
            sender,
        )
        .await;
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        assert_eq!((event.session, event.attempt), (4, 7));
        events.push(event);
    }
    server.join().unwrap();
    events
}

#[tokio::test]
async fn fragmented_unicode_and_final_frame_without_newline_complete() {
    let body = "{\"message\":{\"content\":\"🦀 hello\"},\"done\":false}\n{\"done\":true}";
    let chunks = body.as_bytes().chunks(1).map(|b| b.to_vec()).collect();
    let events = collect(chunks, Duration::ZERO, Duration::from_secs(3)).await;
    assert!(matches!(&events[0].update, Update::Admitted));
    assert!(matches!(&events[1].update, Update::Token(text) if text == "🦀 hello"));
    assert!(matches!(events.last().unwrap().update, Update::Done));
}

#[tokio::test]
async fn eof_without_done_is_failure_after_partial_output() {
    let events = collect(
        vec![b"{\"message\":{\"content\":\"partial\"}}\n".to_vec()],
        Duration::ZERO,
        Duration::from_secs(3),
    )
    .await;
    assert!(matches!(&events[0].update, Update::Admitted));
    assert!(matches!(&events[1].update, Update::Token(text) if text == "partial"));
    assert!(
        matches!(&events[2].update, Update::Failed(error) if error.contains("before completion"))
    );
}

#[tokio::test]
async fn malformed_oversized_and_midstream_errors_are_never_completion() {
    for body in [
        b"not json\n".to_vec(),
        b"{}\n".to_vec(),
        vec![b'x'; 65537],
        b"{\"error\":\"model unloaded\"}\n".to_vec(),
        vec![255, b'\n'],
    ] {
        let events = collect(vec![body], Duration::ZERO, Duration::from_secs(3)).await;
        assert!(matches!(events.last().unwrap().update, Update::Failed(_)));
        assert!(!events.iter().any(|e| matches!(e.update, Update::Done)));
    }
}

#[tokio::test]
async fn idle_stream_hits_deadline_without_blocking_other_sessions() {
    let slow = collect(
        vec![b"{\"message\":{\"content\":\"partial\"}}\n".to_vec()],
        Duration::from_millis(250),
        Duration::from_millis(80),
    );
    let fast = collect(
        vec![b"{\"done\":true}\n".to_vec()],
        Duration::ZERO,
        Duration::from_secs(3),
    );
    let (slow, fast) = tokio::join!(slow, fast);
    assert!(
        matches!(&slow.last().unwrap().update, Update::Failed(error) if error.contains("stalled"))
    );
    assert!(matches!(fast.last().unwrap().update, Update::Done));
}

#[tokio::test]
async fn aborting_a_stalled_request_finishes_promptly() {
    let (endpoint, server) = server(vec![b"\n".to_vec()], Duration::from_millis(250));
    let provider = Ollama::new(&endpoint, Duration::from_secs(60)).unwrap();
    let (sender, _receiver) = mpsc::channel(8);
    let job = tokio::spawn(async move {
        provider.chat(0, 1, "fixture".into(), vec![], sender).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    job.abort();
    let result = tokio::time::timeout(Duration::from_millis(100), job)
        .await
        .unwrap();
    assert!(result.unwrap_err().is_cancelled());
    server.join().unwrap();
}

#[test]
fn invalid_provider_origins_are_rejected() {
    for url in [
        "file:///tmp/model",
        "http://user:secret@localhost",
        "http://localhost/path",
        "http://localhost?token=secret",
    ] {
        assert!(Ollama::new(url, Duration::from_secs(1)).is_err());
    }
}

#[tokio::test]
async fn completion_metrics_are_advisory_bounded_and_emitted_before_done() {
    let events=collect(vec![
        b"{\"message\":{\"content\":\"A\"},\"done\":false,\"load_duration\":999}\n".to_vec(),
        b"{\"message\":{\"content\":\"B\"},\"done\":true,\"total_duration\":3500000000,\"load_duration\":500000000,\"prompt_eval_duration\":1000000000,\"eval_duration\":2000000000,\"prompt_eval_count\":100,\"eval_count\":40}\n".to_vec(),
    ],Duration::ZERO,Duration::from_secs(3)).await;
    assert!(matches!(events.last().unwrap().update, Update::Done));
    let Update::Metrics(metrics) = &events[events.len() - 2].update else {
        panic!("Missing completion timings")
    };
    assert_eq!(metrics.load_duration, Some(500_000_000));
    assert!(metrics.summary().contains("load 0.50s"));
    assert!(metrics.summary().contains("20.0 generated tokens/s"));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.update, Update::Metrics(_)))
            .count(),
        1
    );
    for raw in [
        r#"{"message":{"content":"ok"},"done":true,"load_duration":"bad","eval_duration":-1,"total_duration":18446744073709551615,"eval_count":99999999}"#,
        r#"{"message":{"content":"ok"},"done":true,"eval_duration":0,"eval_count":40}"#,
    ] {
        let events = collect(
            vec![format!("{raw}\n").into_bytes()],
            Duration::ZERO,
            Duration::from_secs(3),
        )
        .await;
        assert!(matches!(events.last().unwrap().update, Update::Done));
        for event in events {
            if let Update::Metrics(metrics) = event.update {
                assert!(!metrics.summary().contains("tokens/s"));
            }
        }
    }
}

#[test]
fn timing_observations_do_not_restore_or_cross_attempts() {
    use alfredo_tui::{metrics::Metrics, model::Session};
    let mut session = Session::new("fixture".into());
    session.insert("hello");
    session.begin().unwrap();
    let metrics = Metrics {
        load_duration: Some(100_000_000),
        ..Default::default()
    };
    session.apply(1, Update::Metrics(metrics.clone()));
    session.apply(1, Update::Done);
    let restored: Session = serde_json::from_slice(&serde_json::to_vec(&session).unwrap()).unwrap();
    assert!(restored.metrics.is_none());
    session.insert("next");
    session.begin().unwrap();
    assert!(session.metrics.is_none());
    session.apply(1, Update::Metrics(metrics));
    assert!(session.metrics.is_none());
}

#[tokio::test]
async fn thinking_only_frames_report_progress_without_exposing_reasoning_as_answer() {
    let events = collect(vec![
        b"{\"message\":{\"thinking\":\"PRIVATE_REASONING_SENTINEL\"}}\n".to_vec(),
        b"{\"message\":{\"thinking\":\"more private text\",\"content\":\"READY\"},\"done\":true}\n".to_vec(),
    ], Duration::ZERO, Duration::from_secs(3)).await;
    assert!(matches!(events[0].update, Update::Admitted));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.update, Update::Thinking))
            .count(),
        1
    );
    let text: String = events
        .iter()
        .filter_map(|event| match &event.update {
            Update::Token(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "READY");
    assert!(matches!(events.last().unwrap().update, Update::Done));
    assert!(!format!("{events:?}").contains("PRIVATE_REASONING_SENTINEL"));
}

#[tokio::test]
async fn thinking_bytes_share_the_bounded_output_budget() {
    let frame = format!(
        "{}\n",
        serde_json::json!({"message":{"thinking":"x".repeat(40 * 1024)}})
    )
    .into_bytes();
    let events = collect(vec![frame; 4], Duration::ZERO, Duration::from_secs(3)).await;
    assert!(
        matches!(&events.last().unwrap().update, Update::Failed(error) if error.contains("128 KiB"))
    );
    assert!(!events
        .iter()
        .any(|event| matches!(event.update, Update::Token(_) | Update::Done)));
}

#[tokio::test]
async fn generation_limit_retains_text_and_metrics_without_successful_completion() {
    for message in [
        serde_json::json!({"content":"partial answer"}),
        serde_json::json!({"content":"{\"files\":[]}"}),
        serde_json::json!({"content":"", "thinking":"not an answer"}),
    ] {
        let frame = serde_json::json!({"message":message, "done":true,
            "done_reason":"length", "eval_count":128, "eval_duration":1_000_000_000_u64});
        let data = serde_json::to_vec(&frame).unwrap();
        let events = collect(
            vec![
                data[..data.len() / 2].to_vec(),
                data[data.len() / 2..].to_vec(),
            ],
            Duration::ZERO,
            Duration::from_secs(3),
        )
        .await;
        assert!(
            matches!(&events.last().unwrap().update, Update::Failed(error) if error.contains("generation limit"))
        );
        assert!(!events
            .iter()
            .any(|event| matches!(event.update, Update::Done)));
        assert!(events
            .iter()
            .any(|event| matches!(event.update, Update::Metrics(_))));
        let content: String = events
            .iter()
            .filter_map(|event| match &event.update {
                Update::Token(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(content, message["content"].as_str().unwrap());
    }
    let events = collect(
        vec![
            b"{\"message\":{\"content\":\"complete\"},\"done\":true,\"done_reason\":\"stop\"}\n"
                .to_vec(),
        ],
        Duration::ZERO,
        Duration::from_secs(3),
    )
    .await;
    assert!(matches!(events.last().unwrap().update, Update::Done));
}

#[tokio::test]
async fn structured_thinking_policy_is_sent_only_for_schema_requests() {
    for (schema, policy, expected) in [
        (true, None, Some(false)),
        (true, Some(Some(true)), Some(true)),
        (true, Some(None), None),
        (false, Some(Some(false)), None),
        (false, Some(Some(true)), None),
    ] {
        let (endpoint, server, request) = server_capture(
            vec![b"{\"message\":{\"content\":\"{}\"},\"done\":true}\n".to_vec()],
            Duration::ZERO,
        );
        let mut provider = Ollama::new(&endpoint, Duration::from_secs(3)).unwrap();
        if let Some(policy) = policy {
            provider = provider.with_structured_thinking(policy);
        }
        if schema {
            provider = provider.with_json_schema(serde_json::json!({"type":"object"}));
        }
        let (sender, mut events) = mpsc::channel(128);
        provider
            .chat(
                0,
                1,
                "fixture".into(),
                vec![Message {
                    role: "user".into(),
                    content: "fixture".into(),
                }],
                sender,
            )
            .await;
        let mut done = false;
        while let Some(event) = events.recv().await {
            done |= matches!(event.update, Update::Done);
        }
        assert!(done);
        let generation = provider.structured_generation();
        let request = request.recv().unwrap();
        if schema {
            assert_eq!(request["options"]["num_predict"], generation.num_predict);
            assert_eq!(request["options"]["temperature"], generation.temperature);
            let requested = match generation.thinking {
                alfredo_tui::provider::RequestedThinking::Auto => None,
                alfredo_tui::provider::RequestedThinking::On => Some(true),
                alfredo_tui::provider::RequestedThinking::Off => Some(false),
            };
            assert_eq!(requested, expected);
        }
        assert_eq!(
            request.get("think").and_then(serde_json::Value::as_bool),
            expected
        );
        assert_eq!(request.get("format").is_some(), schema);
        assert_eq!(request["options"]["num_predict"], 4096);
        server.join().unwrap();
    }
}
