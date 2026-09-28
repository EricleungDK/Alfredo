//! Actual HTTP proves separate provider objects share scheduling and preserve request semantics.
use alfredo_tui::{
    inference_admission::Class,
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

struct Received {
    body: serde_json::Value,
    respond: blocking::Sender<()>,
}
struct Server {
    endpoint: String,
    requests: mpsc::UnboundedReceiver<Received>,
    count: Arc<AtomicUsize>,
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
        let count_in = count.clone();
        let stop_in = stop.clone();
        let job = thread::spawn(move || {
            let mut children = Vec::new();
            while !stop_in.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let sender = sender.clone();
                        let count = count_in.clone();
                        children.push(thread::spawn(move || {
                            socket.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
                            let mut header = Vec::new();
                            let mut byte = [0];
                            while !header.ends_with(b"\r\n\r\n") {
                                socket.read_exact(&mut byte).unwrap();
                                header.push(byte[0]);
                                assert!(header.len() <= 64 * 1024);
                            }
                            let header = String::from_utf8(header).unwrap();
                            let response = if header.starts_with("GET /api/tags HTTP/1.1") {
                                r#"{"models":[{"name":"fixture"}]}"#.to_owned()
                            } else {
                                assert!(header.starts_with("POST /api/chat HTTP/1.1"));
                                let length: usize = header.lines().find_map(|line| {
                                    line.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_owned)
                                }).unwrap().parse().unwrap();
                                assert!(length < 512 * 1024);
                                let mut bytes = vec![0; length];
                                socket.read_exact(&mut bytes).unwrap();
                                let body = serde_json::from_slice(&bytes).unwrap();
                                count.fetch_add(1, Ordering::SeqCst);
                                let (respond, response) = blocking::channel();
                                sender.send(Received { body, respond }).unwrap();
                                if response.recv_timeout(Duration::from_secs(5)).is_err() { return; }
                                "{\"message\":{\"content\":\"ok\"},\"done\":true}\n".into()
                            };
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
            stop,
            job: Some(job),
        }
    }
    fn provider(&self, capacity: usize) -> Ollama {
        Ollama::new(&self.endpoint, Duration::from_secs(5))
            .unwrap()
            .with_parallelism(capacity)
            .unwrap()
    }
    async fn request(&mut self, model: &str) -> Received {
        let request = tokio::time::timeout(Duration::from_secs(3), self.requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.body["model"], model);
        request
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
fn spawn(provider: Ollama, model: &str) -> (JoinHandle<()>, mpsc::Receiver<Event>) {
    let model = model.to_owned();
    let (sender, receiver) = mpsc::channel(32);
    let job = tokio::spawn(async move {
        provider
            .chat(
                3,
                7,
                model,
                vec![Message {
                    role: "user".into(),
                    content: "Fixture request".into(),
                }],
                sender,
            )
            .await;
    });
    (job, receiver)
}
async fn event(events: &mut mpsc::Receiver<Event>) -> Update {
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((event.session, event.attempt), (3, 7));
    event.update
}
async fn queued(events: &mut mpsc::Receiver<Event>, class: Class) {
    assert!(matches!(event(events).await, Update::Queued));
    let Update::QueueProgress(observation) = event(events).await else {
        panic!("Missing shared queue observation");
    };
    assert_eq!(observation.class, class);
    assert_eq!(observation.active, 1);
    assert_eq!(observation.capacity, 1);
    assert!((1..=observation.waiting).contains(&observation.position));
}
async fn finished(job: JoinHandle<()>, mut events: mpsc::Receiver<Event>) {
    tokio::time::timeout(Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap();
    let mut admitted = false;
    let mut done = false;
    while let Some(Event { update, .. }) = events.recv().await {
        match update {
            Update::Admitted => admitted = true,
            Update::Done => done = true,
            Update::Failed(error) => panic!("Unexpected provider failure: {error}"),
            _ => {}
        }
    }
    assert!(admitted && done);
}

#[tokio::test]
async fn independent_providers_prioritize_foreground_without_inferring_role_from_json_format() {
    let mut server = Server::new();
    let (holder, holder_events) = spawn(server.provider(1), "holder");
    let first = server.request("holder").await;
    assert!(first.body.get("format").is_none());
    let schema = serde_json::json!({"type":"object"});
    let background = server
        .provider(1)
        .with_json_schema(schema.clone())
        .with_priority(Class::Background);
    let (worker, mut worker_events) = spawn(background, "background-worker");
    queued(&mut worker_events, Class::Background).await;
    // A planner is structured too; role, not JSON format, chooses foreground priority.
    let foreground = server
        .provider(1)
        .with_json_schema(schema.clone())
        .with_structured_thinking(Some(true));
    let (planner, mut planner_events) = spawn(foreground, "foreground-planner");
    queued(&mut planner_events, Class::Foreground).await;
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    first.respond.send(()).unwrap();
    finished(holder, holder_events).await;
    let next = server.request("foreground-planner").await;
    assert_eq!(next.body["format"], schema);
    assert_eq!(next.body["think"], true);
    assert_eq!(next.body["options"]["num_predict"], 4096);
    next.respond.send(()).unwrap();
    finished(planner, planner_events).await;
    let last = server.request("background-worker").await;
    assert_eq!(last.body["format"], schema);
    assert_eq!(last.body["think"], false);
    last.respond.send(()).unwrap();
    finished(worker, worker_events).await;
    assert_eq!(server.count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn queued_cancellation_and_capacity_conflict_send_no_http_while_discovery_stays_available() {
    let mut server = Server::new();
    let (holder, holder_events) = spawn(server.provider(1), "holder");
    let first = server.request("holder").await;
    let (cancelled, mut cancelled_events) = spawn(server.provider(1), "cancelled");
    queued(&mut cancelled_events, Class::Foreground).await;
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    // Construction, configuring and discovery do not join or conflict with the live queue.
    let conflicting = server.provider(2);
    assert_eq!(conflicting.models().await.unwrap(), vec!["fixture"]);
    let (conflict, mut conflict_events) = spawn(conflicting.clone(), "conflicting");
    let Update::Failed(error) = event(&mut conflict_events).await else {
        panic!("Conflicting admission was not refused before dispatch");
    };
    assert!(error.to_ascii_lowercase().contains("capacity"), "{error}");
    conflict.await.unwrap();
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    first.respond.send(()).unwrap();
    finished(holder, holder_events).await;
    // No live ticket retains the former capacity; a new explicit configuration can proceed.
    let (next, next_events) = spawn(conflicting, "new-capacity");
    server
        .request("new-capacity")
        .await
        .respond
        .send(())
        .unwrap();
    finished(next, next_events).await;
    assert_eq!(server.count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn shared_admission_rechecks_captured_inputs_before_http_and_releases_refused_slot() {
    let mut server = Server::new();
    let (holder, holder_events) = spawn(server.provider(1), "holder");
    let first = server.request("holder").await;
    let checks = Arc::new(AtomicUsize::new(0));
    let changed = Arc::new(AtomicBool::new(false));
    let checks_in = checks.clone();
    let changed_in = changed.clone();
    let waiting = server.provider(1);
    let (sender, mut events) = mpsc::channel(32);
    let job = tokio::spawn(async move {
        waiting
            .chat_with_admission(
                3,
                7,
                "stale-captured-request".into(),
                vec![],
                sender,
                move || {
                    checks_in.fetch_add(1, Ordering::SeqCst);
                    let result = if changed_in.load(Ordering::SeqCst) {
                        Err("Captured task revision changed".into())
                    } else {
                        Ok(())
                    };
                    async move { result }
                },
            )
            .await;
    });
    queued(&mut events, Class::Foreground).await;
    assert_eq!(checks.load(Ordering::SeqCst), 0);
    changed.store(true, Ordering::SeqCst);
    first.respond.send(()).unwrap();
    finished(holder, holder_events).await;
    job.await.unwrap();
    let mut failure = false;
    while let Some(Event { update, .. }) = events.recv().await {
        match update {
            Update::Failed(error) => {
                assert_eq!(error, "Captured task revision changed");
                failure = true;
            }
            Update::Admitted | Update::Token(_) | Update::Done => {
                panic!("Stale request crossed admission")
            }
            _ => {}
        }
    }
    assert!(failure);
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    let (fresh, fresh_events) = spawn(server.provider(1), "fresh-request");
    server
        .request("fresh-request")
        .await
        .respond
        .send(())
        .unwrap();
    finished(fresh, fresh_events).await;
    assert_eq!(server.count.load(Ordering::SeqCst), 2);
}
