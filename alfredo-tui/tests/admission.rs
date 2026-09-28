use alfredo_tui::{
    model::{Event, Message, Session, Update},
    provider::Ollama,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

#[tokio::test]
async fn shared_admission_queues_cancels_without_dispatch_and_releases_for_next_worker() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider = Ollama::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        Duration::from_secs(5),
    )
    .unwrap()
    .with_parallelism(1)
    .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(AtomicBool::new(false));
    let server_count = count.clone();
    let server_release = release.clone();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut children = Vec::new();
        while server_count.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    let ordinal = server_count.fetch_add(1, Ordering::SeqCst);
                    let release = server_release.clone();
                    children.push(thread::spawn(move || {
                        socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                        let mut header = Vec::new();
                        let mut byte = [0];
                        while !header.ends_with(b"\r\n\r\n") { socket.read_exact(&mut byte).unwrap(); header.push(byte[0]); }
                        let header = String::from_utf8(header).unwrap();
                        let length: usize = header.lines().find_map(|line| line.to_lowercase().strip_prefix("content-length: ").map(str::to_owned)).unwrap().parse().unwrap();
                        let mut body = vec![0; length]; socket.read_exact(&mut body).unwrap();
                        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        assert_ne!(request["model"], "cancelled-before-dispatch");
                        while ordinal == 0 && !release.load(Ordering::SeqCst) && Instant::now() < deadline { thread::sleep(Duration::from_millis(2)); }
                        let body = b"{\"message\":{\"content\":\"ok\"},\"done\":true}\n";
                        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).unwrap();
                        socket.write_all(body).unwrap();
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
        assert_eq!(server_count.load(Ordering::SeqCst), 2);
    });
    let spawn = |provider: Ollama, id, model: &str| {
        let (sender, receiver) = mpsc::channel(128);
        let model = model.to_string();
        let job = tokio::spawn(async move {
            provider
                .chat(
                    id,
                    1,
                    model,
                    vec![Message {
                        role: "user".into(),
                        content: "test".into(),
                    }],
                    sender,
                )
                .await;
        });
        (job, receiver)
    };
    let (first, mut first_events) = spawn(provider.clone(), 0, "first");
    let deadline = Instant::now() + Duration::from_secs(3);
    while count.load(Ordering::SeqCst) != 1 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let (cancelled, mut cancelled_events) = spawn(provider.clone(), 1, "cancelled-before-dispatch");
    assert!(matches!(
        cancelled_events.recv().await.unwrap().update,
        Update::Queued
    ));
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    let structured = provider.with_json_schema(serde_json::json!({"type":"object"}));
    let (next, mut next_events) = spawn(structured, 2, "structured-worker");
    assert!(matches!(
        next_events.recv().await.unwrap().update,
        Update::Queued
    ));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    release.store(true, Ordering::SeqCst);
    first.await.unwrap();
    assert!(matches!(
        first_events.recv().await.unwrap().update,
        Update::Admitted
    ));
    assert!(matches!(
        first_events.recv().await.unwrap().update,
        Update::Token(_)
    ));
    loop {
        match next_events.recv().await.unwrap().update {
            Update::QueueProgress(_) => {}
            Update::Admitted => break,
            update => panic!("Unexpected queued request update: {update:?}"),
        }
    }
    next.await.unwrap();
    let mut done = false;
    while let Some(Event {
        session,
        attempt,
        update,
    }) = next_events.recv().await
    {
        assert_eq!((session, attempt), (2, 1));
        done |= matches!(update, Update::Done);
    }
    assert!(done);
    server.join().unwrap();
}

#[test]
fn queue_projection_never_changes_turn_identity_or_resurrects_cancellation() {
    let mut session = Session::new("fixture".into());
    session.insert("prompt");
    session.begin().unwrap();
    session.apply(1, Update::Queued);
    assert_eq!(session.status_label(), "Queued for Alfredo");
    session.apply(1, Update::Admitted);
    assert!(!session.status_label().contains("Queued"));
    session.apply(1, Update::Queued);
    session.cancel();
    session.apply(1, Update::Admitted);
    assert!(session.status_label().contains("Cancelled"));
    assert!(
        Ollama::new("http://localhost:11434", Duration::from_secs(1))
            .unwrap()
            .with_parallelism(0)
            .is_err()
    );
}
