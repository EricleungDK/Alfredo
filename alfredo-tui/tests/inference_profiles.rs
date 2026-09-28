use alfredo_tui::{
    inference_admission::Class,
    inference_profile::{digest, ContextProfile, RequestOutcome, RequestRecorder},
    model::{Event, Message, Update},
    provider::Ollama,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc as blocking, Arc,
    },
    thread,
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};

const COMPLETE: &str = "{\"message\":{\"content\":\"ok\"},\"done\":true,\"total_duration\":120,\"load_duration\":10,\"prompt_eval_duration\":20,\"eval_duration\":90,\"prompt_eval_count\":4,\"eval_count\":2}\n";
struct Received {
    bytes: Vec<u8>,
    respond: blocking::Sender<&'static str>,
}
#[derive(Default)]
struct Probe {
    blocked: AtomicBool,
    failed: AtomicBool,
    seen: AtomicUsize,
    context: AtomicUsize,
}
struct Server {
    endpoint: String,
    requests: mpsc::UnboundedReceiver<Received>,
    count: Arc<AtomicUsize>,
    probe: Arc<Probe>,
    stop: Arc<AtomicBool>,
    job: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (sender, requests) = mpsc::unbounded_channel();
        let count = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let probe = Arc::new(Probe::default());
        let probes = probe.clone();
        let seen = count.clone();
        let stopping = stop.clone();
        let job = thread::spawn(move || {
            let mut children = Vec::new();
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let sender = sender.clone();
                        let seen = seen.clone();
                        let probe = probes.clone();
                        let stopping = stopping.clone();
                        children.push(thread::spawn(move || {
                            socket.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
                            let mut header = Vec::new();
                            let mut byte = [0];
                            while !header.ends_with(b"\r\n\r\n") {
                                socket.read_exact(&mut byte).unwrap();
                                header.push(byte[0]);
                                assert!(header.len() < 8192);
                            }
                            let header = String::from_utf8(header).unwrap();
                            if header.starts_with("GET ") {
                                probe.seen.fetch_add(1, Ordering::SeqCst);
                                while probe.blocked.load(Ordering::SeqCst) && !stopping.load(Ordering::SeqCst) {
                                    thread::sleep(Duration::from_millis(2));
                                }
                                let response = if probe.failed.load(Ordering::SeqCst) {
                                    "not JSON".to_owned()
                                } else if header.starts_with("GET /api/version ") {
                                    serde_json::json!({"version":"fixture-1"}).to_string()
                                } else if header.starts_with("GET /api/tags ") {
                                    serde_json::json!({"models":[{"name":"fixture","digest":"a".repeat(64),"details":{"quantization_level":"Q4_K_M"}}]}).to_string()
                                } else {
                                    assert!(header.starts_with("GET /api/ps "));
                                    serde_json::json!({"models":[{"name":"fixture","digest":"a".repeat(64),"size":1024,"size_vram":512,"context_length":probe.context.load(Ordering::SeqCst)}]}).to_string()
                                };
                                let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len());
                                let _ = socket.write_all(header.as_bytes()).and_then(|()| socket.write_all(response.as_bytes()));
                                return;
                            }
                            assert!(header.starts_with("POST /api/chat HTTP/1.1"));
                            assert!(header.to_ascii_lowercase().contains("content-type: application/json"));
                            let length: usize = header.lines().find_map(|line| {
                                line.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_owned)
                            }).unwrap().parse().unwrap();
                            assert!(length <= 4 * 1024 * 1024);
                            let mut bytes = vec![0; length];
                            socket.read_exact(&mut bytes).unwrap();
                            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                            probe.context.store(body["options"]["num_ctx"].as_u64().unwrap_or(4096) as usize, Ordering::SeqCst);
                            seen.fetch_add(1, Ordering::SeqCst);
                            let (respond, response) = blocking::channel();
                            if sender.send(Received { bytes, respond }).is_err() { return; }
                            let Ok(response) = response.recv_timeout(Duration::from_secs(5)) else { return; };
                            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len());
                            let _ = socket.write_all(header.as_bytes()).and_then(|()| socket.write_all(response.as_bytes()));
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("{error}"),
                }
            }
            for child in children {
                child.join().unwrap();
            }
        });
        Self {
            endpoint,
            requests,
            count,
            probe,
            stop,
            job: Some(job),
        }
    }
    fn provider(&self) -> Ollama {
        Ollama::new(&self.endpoint, Duration::from_secs(2))
            .unwrap()
            .with_parallelism(1)
            .unwrap()
    }
    async fn request(&mut self) -> Received {
        tokio::time::timeout(Duration::from_secs(3), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn probing(&self) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.probe.seen.load(Ordering::SeqCst) < 3 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(job) = self.job.take() {
            job.join().unwrap();
        }
    }
}
fn messages(prefix: &str, suffix: &str) -> Vec<Message> {
    vec![
        Message {
            role: "system".into(),
            content: prefix.into(),
        },
        Message {
            role: "user".into(),
            content: suffix.into(),
        },
    ]
}
fn spawn(provider: Ollama, messages: Vec<Message>) -> (JoinHandle<()>, mpsc::Receiver<Event>) {
    let (sender, events) = mpsc::channel(64);
    let job = tokio::spawn(async move {
        provider
            .chat(4, 9, "fixture".into(), messages, sender)
            .await;
    });
    (job, events)
}
async fn finish(job: JoinHandle<()>, mut events: mpsc::Receiver<Event>) -> Vec<Update> {
    tokio::time::timeout(Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap();
    let mut result = Vec::new();
    while let Some(event) = events.recv().await {
        result.push(event.update);
    }
    result
}
async fn queued(events: &mut mpsc::Receiver<Event>) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        match event.update {
            Update::QueueProgress(_) => return,
            Update::Queued => {}
            other => panic!("Unexpected update before queue: {other:?}"),
        }
    }
}

#[tokio::test]
async fn recorded_wire_preserves_baseline_and_uses_explicit_class_for_context() {
    let mut server = Server::new();
    let recorder = RequestRecorder::new();
    let input = messages("Private source 🦀", "Private fixture suffix");
    let schema = serde_json::json!({"type":"object","properties":{"answer":{"type":"string"}}});
    for (index, (profile, class, structured, think)) in [
        (
            ContextProfile::Baseline,
            Class::Foreground,
            false,
            Some(false),
        ),
        (ContextProfile::Baseline, Class::Background, true, None),
        (
            ContextProfile::ContextCandidate,
            Class::Foreground,
            true,
            Some(true),
        ),
        (
            ContextProfile::ContextCandidate,
            Class::Background,
            true,
            Some(false),
        ),
        (
            ContextProfile::ContextCandidate,
            Class::Foreground,
            false,
            Some(true),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut provider = server
            .provider()
            .with_context_profile(profile)
            .with_priority(class)
            .with_structured_thinking(think)
            .with_request_recorder(recorder.clone());
        if structured {
            provider = provider.with_json_schema(schema.clone());
        }
        // Cloned provider settings, not fixture-side reconstruction, determine the wire.
        let (job, events) = spawn(provider.clone(), input.clone());
        let received = server.request().await;
        let actual: serde_json::Value = serde_json::from_slice(&received.bytes).unwrap();
        let mut expected = serde_json::json!({"model":"fixture","messages":input,"stream":true,"options":{"num_predict":4096}});
        if structured {
            expected["format"] = schema.clone();
            expected["options"]["temperature"] = 0.into();
            if let Some(think) = think {
                expected["think"] = think.into();
            }
        }
        if let Some(context) = profile.context(class) {
            expected["options"]["num_ctx"] = context.into();
        }
        assert_eq!(actual, expected);
        let in_flight = &recorder.snapshot().unwrap()[index];
        assert_eq!(in_flight.outcome, RequestOutcome::InFlight);
        assert_eq!(in_flight.request_sha256, digest(&received.bytes));
        assert_eq!(in_flight.request_bytes, received.bytes.len());
        assert_eq!(in_flight.profile.num_ctx, profile.context(class));
        assert_eq!(
            in_flight.profile.think,
            if structured { think } else { None }
        );
        assert_eq!(in_flight.profile.temperature, structured.then_some(0));
        assert_eq!(in_flight.profile.class, class);
        assert!(in_flight.matches_binding(profile, &server.endpoint, "fixture", 1));
        received.respond.send(COMPLETE).unwrap();
        let updates = finish(job, events).await;
        assert!(matches!(updates.last(), Some(Update::Done)));
        let record = &recorder.snapshot().unwrap()[index];
        assert_eq!(record.outcome, RequestOutcome::Completed);
        assert!(record.first_content_ms.is_some());
        assert!(record.total_ms.is_some());
        assert!(record.generation_ms.is_some());
        assert!(record.runtime_probe_ms.is_some());
        assert!(record.runtime_error.is_none());
        let runtime = record.runtime_after.as_ref().unwrap();
        assert!(runtime
            .request_reasons(&server.endpoint, "fixture", profile.context(class))
            .is_empty());
        assert_eq!(record.metrics.as_ref().unwrap().prompt_eval_count, Some(4));
        record.validate().unwrap();
        let retained = serde_json::to_string(record).unwrap();
        assert!(!retained.contains("Private source"));
        assert!(!retained.contains("Private fixture suffix"));
    }
}

#[tokio::test]
async fn prefix_observations_bind_source_order_schema_and_profile_without_caching_suffixes() {
    let mut server = Server::new();
    let recorder = RequestRecorder::new();
    for index in 0..6 {
        let mut provider = server.provider().with_request_recorder(recorder.clone());
        let mut input = vec![
            Message {
                role: "system".into(),
                content: "source A".into(),
            },
            Message {
                role: "user".into(),
                content: "source B".into(),
            },
            Message {
                role: "user".into(),
                content: "suffix A".into(),
            },
        ];
        match index {
            0 => {}
            1 => input[2].content = "suffix B".into(),
            2 => input[0].content = "changed source".into(),
            3 => input.swap(0, 1),
            4 => provider = provider.with_json_schema(serde_json::json!({"type":"object"})),
            5 => provider = provider.with_context_profile(ContextProfile::ContextCandidate),
            _ => unreachable!(),
        }
        let (job, events) = spawn(provider, input);
        server.request().await.respond.send(COMPLETE).unwrap();
        assert!(matches!(
            finish(job, events).await.last(),
            Some(Update::Done)
        ));
    }
    let records = recorder.snapshot().unwrap();
    assert_ne!(records[0].request_sha256, records[1].request_sha256);
    assert_eq!(records[0].prefix_sha256, records[1].prefix_sha256);
    for record in &records[2..] {
        assert_ne!(records[0].prefix_sha256, record.prefix_sha256);
    }
    let mut tampered = records[0].clone();
    tampered.profile.num_ctx = Some(8192);
    assert!(tampered.validate().is_err());
    let mut tampered = records[0].clone();
    tampered.profile.endpoint_origin = "http://localhost:1".into();
    assert!(tampered.validate().is_err());
    let mut tampered = records[0].clone();
    tampered.prefix_wire_sha256 = digest(b"changed prefix");
    assert!(tampered.validate().is_err());
    let mut tampered = records[0].clone();
    tampered.total_ms = None;
    assert!(tampered.validate().is_err());
}

#[tokio::test]
async fn recorder_limit_refuses_http_and_dropped_or_failed_streams_remain_terminal() {
    assert!(RequestRecorder::with_limit(0).is_err());
    assert!(RequestRecorder::with_limit(129).is_err());
    let mut server = Server::new();
    let recorder = RequestRecorder::with_limit(2).unwrap();
    let provider = server.provider().with_request_recorder(recorder.clone());
    let (job, events) = spawn(provider.clone(), messages("source", "cancel"));
    let request = server.request().await;
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    drop(events);
    request.respond.send(COMPLETE).unwrap();
    let cancelled = &recorder.snapshot().unwrap()[0];
    assert_eq!(cancelled.outcome, RequestOutcome::Interrupted);
    cancelled.validate().unwrap();
    let (job, events) = spawn(provider.clone(), messages("source", "malformed"));
    server.request().await.respond.send("not json\n").unwrap();
    assert!(matches!(
        finish(job, events).await.last(),
        Some(Update::Failed(_))
    ));
    let failed = &recorder.snapshot().unwrap()[1];
    assert_eq!(failed.outcome, RequestOutcome::Failed);
    failed.validate().unwrap();
    let (job, events) = spawn(provider, messages("source", "over budget"));
    assert!(
        matches!(finish(job, events).await.last(), Some(Update::Failed(error)) if error.contains("request limit"))
    );
    assert_eq!(server.count.load(Ordering::SeqCst), 2);
    assert_eq!(recorder.snapshot().unwrap().len(), 2);
    // A refusal releases the shared slot; a separate diagnostic recorder can run.
    let (job, events) = spawn(server.provider(), messages("source", "fresh"));
    server.request().await.respond.send(COMPLETE).unwrap();
    assert!(matches!(
        finish(job, events).await.last(),
        Some(Update::Done)
    ));
}

#[tokio::test]
async fn queued_cancellation_and_stale_guard_produce_no_record_or_http() {
    let mut server = Server::new();
    let recorder = RequestRecorder::new();
    let provider = server
        .provider()
        .with_request_recorder(recorder.clone())
        .with_context_profile(ContextProfile::ContextCandidate);
    let (holder, holder_events) = spawn(provider.clone(), messages("source", "holder"));
    let first = server.request().await;
    let (cancelled, mut cancelled_events) =
        spawn(provider.clone(), messages("source", "queued cancel"));
    queued(&mut cancelled_events).await;
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    assert_eq!(recorder.snapshot().unwrap().len(), 1);
    let checked = Arc::new(AtomicBool::new(false));
    let observed = checked.clone();
    let (sender, mut events) = mpsc::channel(32);
    let stale = tokio::spawn(async move {
        provider
            .chat_with_admission(
                4,
                9,
                "fixture".into(),
                messages("source", "stale"),
                sender,
                move || {
                    observed.store(true, Ordering::SeqCst);
                    async move { Err("Canonical task changed".into()) }
                },
            )
            .await;
    });
    queued(&mut events).await;
    assert!(!checked.load(Ordering::SeqCst));
    first.respond.send(COMPLETE).unwrap();
    assert!(matches!(
        finish(holder, holder_events).await.last(),
        Some(Update::Done)
    ));
    assert!(
        matches!(finish(stale, events).await.last(), Some(Update::Failed(error)) if error == "Canonical task changed")
    );
    assert!(checked.load(Ordering::SeqCst));
    assert_eq!(recorder.snapshot().unwrap().len(), 1);
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn runtime_probe_retains_permit_and_done_cannot_be_downgraded_by_receiver_abort() {
    let mut server = Server::new();
    server.probe.blocked.store(true, Ordering::SeqCst);
    let recorder = RequestRecorder::new();
    let provider = server
        .provider()
        .with_request_recorder(recorder.clone())
        .with_priority(Class::Background)
        .with_context_profile(ContextProfile::ContextCandidate);
    let observer = provider.request_observer().unwrap();
    let (job, mut events) = spawn(provider, messages("source", "observed request"));
    let request = server.request().await;
    assert_eq!(
        observer.active_generations(Class::Background).unwrap(),
        vec![1]
    );
    assert!(observer.generation_active(1, Class::Background).unwrap());
    request.respond.send(COMPLETE).unwrap();
    server.probing().await;
    assert!(!observer.generation_active(1, Class::Background).unwrap());
    let during = &recorder.snapshot().unwrap()[0];
    assert_eq!(during.outcome, RequestOutcome::InFlight);
    assert!(during.generation_ms.is_some());
    assert!(during.runtime_after.is_none());
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event.update, Update::Done | Update::Failed(_)));
    }
    // A different client cannot change the model's running context before inspection.
    let (next, mut next_events) = spawn(server.provider(), messages("source", "next"));
    queued(&mut next_events).await;
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    server.probe.blocked.store(false, Ordering::SeqCst);
    loop {
        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        match event.update {
            Update::Done => {
                // Match worker behavior: inspect immediately at delivery, then abort/join.
                assert_eq!(
                    recorder.snapshot().unwrap()[0].outcome,
                    RequestOutcome::Completed
                );
                job.abort();
                let _ = job.await;
                break;
            }
            Update::Failed(error) => panic!("{error}"),
            _ => {}
        }
    }
    let record = &recorder.snapshot().unwrap()[0];
    assert_eq!(record.outcome, RequestOutcome::Completed);
    assert!(record.total_ms.unwrap() >= record.generation_ms.unwrap());
    assert!(record.runtime_probe_ms.is_some());
    assert_eq!(
        record
            .runtime_after
            .as_ref()
            .unwrap()
            .running
            .as_ref()
            .unwrap()
            .context_length,
        Some(16384)
    );
    record.validate().unwrap();
    server.request().await.respond.send(COMPLETE).unwrap();
    assert!(matches!(
        finish(next, next_events).await.last(),
        Some(Update::Done)
    ));
    // Unrecorded normal requests do not pay for metadata probes.
    assert_eq!(server.probe.seen.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn missing_runtime_proof_does_not_rewrite_successful_generation_as_a_transport_failure() {
    let mut server = Server::new();
    server.probe.failed.store(true, Ordering::SeqCst);
    let recorder = RequestRecorder::new();
    let (job, events) = spawn(
        server.provider().with_request_recorder(recorder.clone()),
        messages("source", "probe error"),
    );
    server.request().await.respond.send(COMPLETE).unwrap();
    assert!(matches!(
        finish(job, events).await.last(),
        Some(Update::Done)
    ));
    let record = &recorder.snapshot().unwrap()[0];
    assert_eq!(record.outcome, RequestOutcome::Completed);
    assert!(record.runtime_after.is_none());
    assert!(record.runtime_error.is_some());
    assert!(record.generation_ms.is_some());
    assert!(record.runtime_probe_ms.is_some());
    record.validate().unwrap();
}

#[tokio::test]
async fn cancellation_during_runtime_probe_keeps_generation_timestamp_and_explicit_missing_proof() {
    let mut server = Server::new();
    server.probe.blocked.store(true, Ordering::SeqCst);
    let recorder = RequestRecorder::new();
    let (job, mut events) = spawn(
        server.provider().with_request_recorder(recorder.clone()),
        messages("source", "probe cancelled"),
    );
    server.request().await.respond.send(COMPLETE).unwrap();
    server.probing().await;
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    while let Some(event) = events.recv().await {
        assert!(!matches!(event.update, Update::Done));
    }
    let record = &recorder.snapshot().unwrap()[0];
    assert_eq!(record.outcome, RequestOutcome::Interrupted);
    assert!(record.generation_ms.is_some());
    assert!(record.runtime_probe_ms.is_some());
    assert!(record.runtime_after.is_none());
    assert!(record
        .runtime_error
        .as_ref()
        .unwrap()
        .contains("before runtime inspection completed"));
    record.validate().unwrap();
    server.probe.blocked.store(false, Ordering::SeqCst);
    let (fresh, events) = spawn(
        server.provider(),
        messages("source", "after interrupted probe"),
    );
    server.request().await.respond.send(COMPLETE).unwrap();
    assert!(matches!(
        finish(fresh, events).await.last(),
        Some(Update::Done)
    ));
}
