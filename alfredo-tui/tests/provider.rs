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

/// Serves `body`, then holds the connection open until released (bounded for safety).
fn server_capture_held(
    body: Vec<Vec<u8>>,
    held: std::sync::mpsc::Receiver<()>,
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
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let headers = String::from_utf8(request).unwrap();
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
        let _ = sender.send(serde_json::from_slice(&request).unwrap());
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n").unwrap();
        for bytes in body {
            stream.write_all(&bytes).unwrap();
            let _ = stream.flush();
        }
        let _ = held.recv_timeout(Duration::from_secs(120));
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
    // The stalled server keeps its connection open until the client has reported
    // its outcome, so only the idle deadline can end the slow stream.
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (endpoint, server, _) = server_capture_held(
        vec![b"{\"message\":{\"content\":\"partial\"}}\n".to_vec()],
        held,
    );
    let slow = async move {
        // Generous deadline: on a loaded host a short one can expire before the
        // first frame arrives, which is a different (retryable) failure.
        let provider = Ollama::new(&endpoint, Duration::from_secs(1)).unwrap();
        let (sender, mut receiver) = mpsc::channel(128);
        provider.chat(4, 7, "fixture".into(), vec![], sender).await;
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            events.push(event);
        }
        release.send(()).unwrap();
        events
    };
    let fast = collect(
        vec![b"{\"done\":true}\n".to_vec()],
        Duration::ZERO,
        Duration::from_secs(3),
    );
    let (slow, fast) = tokio::join!(slow, fast);
    tokio::task::spawn_blocking(move || server.join().unwrap())
        .await
        .unwrap();
    assert!(
        matches!(&slow.last().unwrap().update, Update::Failed(error) if error.contains("stalled"))
    );
    assert!(matches!(fast.last().unwrap().update, Update::Done));
}

#[tokio::test]
async fn aborting_a_stalled_request_finishes_promptly() {
    let (endpoint, server, received) =
        server_capture(vec![b"\n".to_vec()], Duration::from_millis(250));
    let provider = Ollama::new(&endpoint, Duration::from_secs(60)).unwrap();
    let (sender, _receiver) = mpsc::channel(8);
    let job = tokio::spawn(async move {
        provider.chat(0, 1, "fixture".into(), vec![], sender).await;
    });
    // Abort only once the request is in flight; a fixed sleep raced the send.
    tokio::task::spawn_blocking(move || received.recv().unwrap())
        .await
        .unwrap();
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
            matches!(&events.last().unwrap().update, Update::Failed(error) if error.starts_with("Model output hit the 4096-token limit"))
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
async fn repair_sampling_is_sent_bounded_and_recorded_as_requested_generation() {
    for (temperature, limit, wire_temperature, wire_limit, summary) in [
        (
            0.3,
            8192,
            serde_json::json!(0.3),
            8192,
            "token limit 8192 · temperature 0.3",
        ),
        (
            5.0,
            999_999,
            serde_json::json!(0.8),
            8192,
            "token limit 8192 · temperature 0.8",
        ),
        (
            0.0,
            4096,
            serde_json::json!(0),
            4096,
            "token limit 4096 · temperature 0",
        ),
    ] {
        let (endpoint, server, request) = server_capture(
            vec![b"{\"message\":{\"content\":\"{}\"},\"done\":true}\n".to_vec()],
            Duration::ZERO,
        );
        let provider = Ollama::new(&endpoint, Duration::from_secs(3))
            .unwrap()
            .with_json_schema(serde_json::json!({"type":"object"}))
            .with_sampling(temperature, limit);
        let generation = provider.structured_generation();
        assert!(generation.valid());
        assert!(
            generation.summary().ends_with(summary),
            "{}",
            generation.summary()
        );
        let (sender, mut events) = mpsc::channel(128);
        provider
            .chat(0, 1, "fixture".into(), prompt(), sender)
            .await;
        while events.recv().await.is_some() {}
        let request = request.recv().unwrap();
        assert_eq!(request["options"]["temperature"], wire_temperature);
        assert_eq!(request["options"]["num_predict"], wire_limit);
        server.join().unwrap();
    }
    // Evidence saved before fractional temperatures still reads as the same number.
    let legacy: alfredo_tui::provider::Generation =
        serde_json::from_str(r#"{"thinking":"off","num_predict":4096,"temperature":0}"#).unwrap();
    assert!(legacy
        .summary()
        .ends_with("token limit 4096 · temperature 0"));
}

#[tokio::test]
async fn structured_text_requests_send_thinking_policy_and_sampling_without_schema() {
    for (policy, expected) in [
        (Some(false), Some(false)),
        (Some(true), Some(true)),
        (None, None),
    ] {
        let (endpoint, server, request) = server_capture(
            vec![b"{\"message\":{\"content\":\"text\"},\"done\":true}\n".to_vec()],
            Duration::ZERO,
        );
        let provider = Ollama::new(&endpoint, Duration::from_secs(3))
            .unwrap()
            .with_structured_thinking(policy)
            .with_structured_text()
            .with_sampling(0.3, 8192);
        let (sender, mut events) = mpsc::channel(128);
        provider
            .chat(0, 1, "fixture".into(), prompt(), sender)
            .await;
        while events.recv().await.is_some() {}
        let request = request.recv().unwrap();
        assert!(request.get("format").is_none());
        assert_eq!(
            request.get("think").and_then(serde_json::Value::as_bool),
            expected
        );
        assert_eq!(request["options"]["temperature"], 0.3);
        assert_eq!(request["options"]["num_predict"], 8192);
        server.join().unwrap();
    }
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

mod ollama_fixture;
use alfredo_tui::model::Retry;
use ollama_fixture::{done, reserve, serve, Reply};

fn prompt() -> Vec<Message> {
    vec![Message {
        role: "user".into(),
        content: "hello".into(),
    }]
}

async fn run(provider: &Ollama) -> Vec<Event> {
    let (sender, mut receiver) = mpsc::channel(128);
    provider
        .chat(4, 7, "fixture".into(), prompt(), sender)
        .await;
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        assert_eq!((event.session, event.attempt), (4, 7));
        events.push(event);
    }
    events
}

fn retries(events: &[Event]) -> Vec<Retry> {
    events
        .iter()
        .filter_map(|event| match &event.update {
            Update::Retrying(retry) => Some(retry.clone()),
            _ => None,
        })
        .collect()
}

fn retrying(endpoint: &str, limit: u32, backoff: Duration) -> Ollama {
    Ollama::new(endpoint, Duration::from_secs(3))
        .unwrap()
        .with_connect_retries(limit)
        .unwrap()
        .with_retry_backoff(backoff)
}

#[test]
fn keep_alive_values_are_validated_and_default_omits_the_field() {
    use alfredo_tui::provider::parse_keep_alive;
    assert_eq!(parse_keep_alive("default").unwrap(), None);
    for valid in ["30m", "1h30m", "0", "-1", "300", "2.5h", "45s"] {
        assert_eq!(parse_keep_alive(valid).unwrap().as_deref(), Some(valid));
    }
    let long = "9".repeat(40);
    for invalid in ["", "bogus", "30 m", "m", "1d", "--1", "1h-2m", &long] {
        assert!(parse_keep_alive(invalid).is_err(), "{invalid}");
    }
}

#[tokio::test]
async fn keep_alive_is_sent_on_chat_and_omitted_when_unset() {
    for (keep_alive, expected) in [
        (None, None),
        (Some("30m"), Some(serde_json::json!("30m"))),
        (Some("-1"), Some(serde_json::json!(-1))),
        (Some("300"), Some(serde_json::json!(300))),
    ] {
        let fixture = serve(reserve(), |_, _| done("ok"));
        let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(3))
            .unwrap()
            .with_keep_alive(keep_alive.map(|value| {
                alfredo_tui::provider::parse_keep_alive(value)
                    .unwrap()
                    .unwrap()
            }));
        let events = run(&provider).await;
        assert!(matches!(events.last().unwrap().update, Update::Done));
        let request = fixture.requests.lock().unwrap()[0].body.clone();
        assert_eq!(request.get("keep_alive").cloned(), expected);
    }
}

#[tokio::test]
async fn refused_connection_before_content_retries_with_backoff_until_server_starts() {
    let addr = reserve();
    let provider = retrying(
        &ollama_fixture::endpoint(addr),
        5,
        Duration::from_millis(100),
    );
    // The server starts only once the first refusal has been reported, so the
    // retry path is exercised regardless of scheduling.
    let (sender, mut receiver) = mpsc::channel(128);
    let chat = provider.chat(4, 7, "fixture".into(), prompt(), sender);
    let observe = async {
        let mut events = Vec::new();
        let mut fixture = None;
        while let Some(event) = receiver.recv().await {
            assert_eq!((event.session, event.attempt), (4, 7));
            if fixture.is_none() && matches!(event.update, Update::Retrying(_)) {
                fixture = Some(
                    tokio::task::spawn_blocking(move || serve(addr, |_, _| done("recovered")))
                        .await
                        .unwrap(),
                );
            }
            events.push(event);
        }
        (events, fixture)
    };
    let (_, (events, fixture)) = tokio::join!(chat, observe);
    let fixture = fixture.expect("first connection was refused");
    let retries = retries(&events);
    assert!(!retries.is_empty());
    assert_eq!(retries[0].retry, 1);
    assert_eq!(retries[0].limit, 5);
    assert_eq!(retries[0].delay, Duration::from_millis(100));
    assert!(retries
        .windows(2)
        .all(|pair| pair[1].delay == pair[0].delay * 2 && pair[1].retry == pair[0].retry + 1));
    assert!(matches!(events.last().unwrap().update, Update::Done));
    let text: String = events
        .iter()
        .filter_map(|event| match &event.update {
            Update::Token(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "recovered");
    assert_eq!(fixture.count("POST /api/chat"), 1);
}

#[tokio::test]
async fn slow_model_load_outlasts_the_idle_deadline_within_the_loading_deadline() {
    let fixture = serve(reserve(), |_, _| {
        thread::sleep(Duration::from_millis(300));
        done("loaded")
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_millis(50))
        .unwrap()
        .with_loading_deadline(Duration::from_secs(3));
    let events = run(&provider).await;
    assert!(matches!(events.last().unwrap().update, Update::Done));
    assert_eq!(fixture.count("POST /api/chat"), 1);
}

#[tokio::test]
async fn loading_deadline_is_final_so_a_retry_never_restarts_the_load() {
    let fixture = serve(reserve(), |_, _| {
        thread::sleep(Duration::from_millis(400));
        done("too late")
    });
    let provider = retrying(&fixture.endpoint, 3, Duration::from_millis(10))
        .with_loading_deadline(Duration::from_millis(100));
    let events = run(&provider).await;
    assert!(retries(&events).is_empty());
    assert!(matches!(
        &events.last().unwrap().update,
        Update::Failed(reason) if reason.contains("loading deadline")
    ));
    assert_eq!(fixture.count("POST /api/chat"), 1);
}

#[tokio::test]
async fn server_errors_and_resets_before_content_are_retried() {
    let fixture = serve(reserve(), |_, index| match index {
        0 => Reply::Json(503, "{\"error\":\"loading model\"}".into()),
        1 => Reply::Close,
        _ => done("third time"),
    });
    let provider = retrying(&fixture.endpoint, 3, Duration::from_millis(10));
    let events = run(&provider).await;
    assert_eq!(retries(&events).len(), 2);
    assert!(retries(&events)[0].reason.contains("503"));
    assert!(matches!(events.last().unwrap().update, Update::Done));
    assert_eq!(fixture.count("POST /api/chat"), 3);
}

#[tokio::test]
async fn error_frame_before_any_content_is_retried_until_the_model_answers() {
    let fixture = serve(reserve(), |_, index| match index {
        0 => Reply::Stream(
            vec![b"{\"error\":\"server is busy, try again\"}\n".to_vec()],
            Duration::ZERO,
        ),
        1 => Reply::Stream(
            vec![b"{\"error\":\"llama runner process has terminated\"}\n".to_vec()],
            Duration::ZERO,
        ),
        _ => done("recovered"),
    });
    let provider = retrying(&fixture.endpoint, 3, Duration::from_millis(10));
    let events = run(&provider).await;
    let seen = retries(&events);
    assert_eq!(seen.len(), 2);
    assert!(seen[0].reason.contains("server is busy"), "{seen:?}");
    assert!(matches!(events.last().unwrap().update, Update::Done));
    assert_eq!(fixture.count("POST /api/chat"), 3);
    // The same bound as every other pre-content failure.
    let persistent = serve(reserve(), |_, _| {
        Reply::Stream(vec![b"{\"error\":\"busy\"}\n".to_vec()], Duration::ZERO)
    });
    let events = run(&retrying(&persistent.endpoint, 2, Duration::from_millis(5))).await;
    assert_eq!(retries(&events).len(), 2);
    assert!(
        matches!(&events.last().unwrap().update, Update::Failed(error) if error.contains("busy"))
    );
    assert_eq!(persistent.count("POST /api/chat"), 3);
}

#[tokio::test]
async fn error_frame_after_partial_content_or_naming_a_missing_model_stays_final() {
    for frames in [
        vec![
            b"{\"message\":{\"content\":\"partial\"}}\n".to_vec(),
            b"{\"error\":\"server is busy\"}\n".to_vec(),
        ],
        vec![
            b"{\"message\":{\"thinking\":\"hmm\"}}\n".to_vec(),
            b"{\"error\":\"server is busy\"}\n".to_vec(),
        ],
        vec![b"{\"error\":\"model 'nope' not found, try pulling it first\"}\n".to_vec()],
        vec![b"{\"error\":\"Model Not Found\"}\n".to_vec()],
    ] {
        let fixture = serve(reserve(), move |_, _| {
            Reply::Stream(frames.clone(), Duration::ZERO)
        });
        let provider = retrying(&fixture.endpoint, 3, Duration::from_millis(10));
        let events = run(&provider).await;
        assert!(retries(&events).is_empty());
        assert!(
            matches!(&events.last().unwrap().update, Update::Failed(error) if error.starts_with("Ollama: "))
        );
        assert_eq!(fixture.count("POST /api/chat"), 1);
    }
}

#[tokio::test]
async fn failure_after_content_keeps_partial_and_is_never_retried() {
    for frame in [
        b"{\"message\":{\"content\":\"partial\"}}\n".to_vec(),
        b"{\"message\":{\"thinking\":\"hmm\"}}\n".to_vec(),
    ] {
        let fixture = serve(reserve(), move |_, _| {
            Reply::Stream(vec![frame.clone()], Duration::ZERO)
        });
        let provider = retrying(&fixture.endpoint, 3, Duration::from_millis(10));
        let events = run(&provider).await;
        assert!(retries(&events).is_empty());
        assert!(
            matches!(&events.last().unwrap().update, Update::Failed(error) if error.contains("partial reply retained"))
        );
        assert_eq!(fixture.count("POST /api/chat"), 1);
    }
}

#[tokio::test]
async fn retries_are_bounded_zero_disables_and_model_errors_are_not_retried() {
    let endpoint = ollama_fixture::endpoint(reserve());
    for (limit, expected) in [(0, 0), (2, 2)] {
        let events = run(&retrying(&endpoint, limit, Duration::from_millis(5))).await;
        assert_eq!(retries(&events).len(), expected);
        assert!(
            matches!(&events.last().unwrap().update, Update::Failed(error) if error.contains("Cannot reach Ollama"))
        );
    }
    assert!(Ollama::new(&endpoint, Duration::from_secs(3))
        .unwrap()
        .with_connect_retries(11)
        .is_err());
    let fixture = serve(reserve(), |_, _| {
        Reply::Stream(
            vec![b"{\"error\":\"model not found\"}\n".to_vec()],
            Duration::ZERO,
        )
    });
    let events = run(&retrying(&fixture.endpoint, 3, Duration::from_millis(5))).await;
    assert!(retries(&events).is_empty());
    assert_eq!(fixture.count("POST /api/chat"), 1);
    let missing = serve(reserve(), |_, _| Reply::Json(404, "{}".into()));
    let events = run(&retrying(&missing.endpoint, 3, Duration::from_millis(5))).await;
    assert!(retries(&events).is_empty());
    assert_eq!(missing.count("POST /api/chat"), 1);
}

#[tokio::test]
async fn aborting_during_retry_backoff_cancels_immediately() {
    let provider = retrying(
        &ollama_fixture::endpoint(reserve()),
        3,
        Duration::from_secs(30),
    );
    let (sender, mut receiver) = mpsc::channel(8);
    let job = tokio::spawn(async move {
        provider
            .chat(0, 1, "fixture".into(), prompt(), sender)
            .await;
    });
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(event.update, Update::Retrying(_)) {
            break;
        }
    }
    job.abort();
    let result = tokio::time::timeout(Duration::from_millis(100), job)
        .await
        .unwrap();
    assert!(result.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn preload_bypasses_admission_sends_keep_alive_and_reports_failure_as_error() {
    let fixture = serve(reserve(), |request, _| match request.path.as_str() {
        "POST /api/chat" => Reply::Stream(vec![b"\n".to_vec(); 40], Duration::from_millis(50)),
        "POST /api/generate" => Reply::Json(
            200,
            "{\"model\":\"fixture\",\"response\":\"\",\"done\":true,\"done_reason\":\"load\"}"
                .into(),
        ),
        _ => Reply::Json(404, "{}".into()),
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(10))
        .unwrap()
        .with_parallelism(1)
        .unwrap()
        .with_keep_alive(Some("30m".into()));
    let (sender, _receiver) = mpsc::channel(64);
    let busy = provider.clone();
    let job = tokio::spawn(async move {
        busy.chat(0, 1, "fixture".into(), prompt(), sender).await;
    });
    while fixture.count("POST /api/chat") == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::timeout(Duration::from_secs(1), provider.preload("fixture"))
        .await
        .expect("preload must not wait for inference capacity")
        .unwrap();
    job.abort();
    let generate = fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|request| request.path == "POST /api/generate")
        .unwrap()
        .body
        .clone();
    assert_eq!(generate["model"], "fixture");
    assert_eq!(generate["keep_alive"], "30m");
    assert_eq!(generate["stream"], false);
    let failing = serve(reserve(), |_, _| {
        Reply::Json(500, "{\"error\":\"no memory\"}".into())
    });
    let provider = Ollama::new(&failing.endpoint, Duration::from_secs(3)).unwrap();
    assert!(provider
        .preload("fixture")
        .await
        .unwrap_err()
        .contains("500"));
}

#[test]
fn ollama_host_forms_normalize_to_an_http_origin() {
    use alfredo_tui::provider::normalize_endpoint;
    for (input, expected) in [
        ("127.0.0.1:11434", "http://127.0.0.1:11434"),
        ("0.0.0.0", "http://127.0.0.1:11434"),
        ("0.0.0.0:8080", "http://127.0.0.1:8080"),
        ("http://0.0.0.0:11434", "http://127.0.0.1:11434"),
        ("localhost", "http://localhost:11434"),
        (" myhost:9000/ ", "http://myhost:9000"),
        ("http://localhost:11434", "http://localhost:11434"),
        ("https://models.example", "https://models.example"),
        ("[::1]:11434", "http://[::1]:11434"),
    ] {
        assert_eq!(normalize_endpoint(input), expected, "{input}");
        assert!(Ollama::new(&normalize_endpoint(input), Duration::from_secs(1)).is_ok());
    }
}
