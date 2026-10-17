use alfredo_tui::{
    model::{App, Status},
    provider::Ollama,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

async fn catalog(body: &str) -> Result<Vec<String>, String> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let provider = Ollama::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        Duration::from_secs(1),
    )
    .unwrap();
    let body = body.as_bytes().to_vec();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        assert!(request.starts_with(b"GET /api/tags HTTP/1.1"));
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .unwrap();
        for chunk in body.chunks(4096) {
            if socket.write_all(chunk).is_err() {
                break;
            }
        }
    });
    let result = provider.models().await;
    server.join().unwrap();
    result
}

#[tokio::test]
async fn installed_models_are_sorted_deduplicated_and_allow_extra_metadata() {
    assert_eq!(
        catalog(r#"{"models":[{"name":"z:14b","size":42},{"name":"a:7b"},{"name":"z:14b"}]}"#)
            .await
            .unwrap(),
        ["a:7b", "z:14b"]
    );
    assert!(catalog(r#"{"models":[]}"#).await.unwrap().is_empty());
}

#[tokio::test]
async fn malformed_control_and_oversized_catalogs_fail_without_partial_selection() {
    for body in ["{broken".to_string(), r#"{"models":[{"name":"bad\u001bname"}]}"#.into(),
        serde_json::json!({"models": (0..257).map(|i| serde_json::json!({"name":format!("m{i}")})).collect::<Vec<_>>()}).to_string(),
        "x".repeat(1024 * 1024 + 1)] {
        assert!(catalog(&body).await.is_err());
    }
}

#[test]
fn model_selection_is_explicit_and_does_not_reassign_active_or_interrupted_turns() {
    let mut app = App::new("original".into());
    app.receive_models(Ok(vec!["other".into()]));
    app.sessions[0].insert("draft");
    app.sessions[0].begin().unwrap();
    assert!(app.select_model("other").is_err());
    assert_eq!(app.sessions[0].model, "original");
    app.sessions[0].cancel();
    assert!(app.select_model("other").is_err());
    app.add_session();
    app.select_model("other").unwrap();
    assert_eq!(app.sessions[0].model, "original");
    assert_eq!(app.sessions[1].model, "other");
    assert!(app.select_model("missing").is_err());
    app.receive_models(Err("Disconnected".into()));
    assert_eq!(app.models, ["other"]);
    assert!(app.models_notice.contains("previous catalog retained"));
    assert_eq!(app.sessions[0].status, Status::Cancelled);
}

#[test]
fn model_name_before_first_discovery_waits_then_selects_an_installed_model() {
    let mut app = App::new("original".into());
    assert!(
        app.queue_model("llama3:8b"),
        "fresh launch must discover first"
    );
    assert_eq!(app.notice, "Checking installed models…");
    assert_eq!(app.sessions[0].model, "original");
    app.receive_models(Ok(vec!["llama3:8b".into(), "qwen3:14b".into()]));
    assert_eq!(app.sessions[0].model, "llama3:8b");
    assert!(
        app.notice.starts_with("Conversation model: llama3:8b"),
        "{}",
        app.notice
    );
    assert!(
        !app.queue_model("qwen3:14b"),
        "loaded catalog selects directly"
    );
}

#[test]
fn model_name_before_first_discovery_reports_not_installed_with_the_catalog() {
    let mut app = App::new("original".into());
    assert!(app.queue_model("missing"));
    app.receive_models(Ok(vec!["llama3:8b".into(), "qwen3:14b".into()]));
    assert_eq!(app.sessions[0].model, "original");
    assert!(
        app.notice.contains("missing is not installed"),
        "{}",
        app.notice
    );
    assert!(
        app.notice.contains("llama3:8b, qwen3:14b"),
        "{}",
        app.notice
    );
    assert!(app.notice.ends_with("· /models"), "{}", app.notice);
}

#[test]
fn not_installed_notice_caps_the_list_and_still_points_to_models() {
    let mut app = App::new("original".into());
    assert!(app.queue_model("missing"));
    app.receive_models(Ok((1..=6).map(|n| format!("m{n}")).collect()));
    assert!(
        app.notice
            .ends_with("m1, m2, m3, m4 (+2 more) · /models lists all"),
        "{}",
        app.notice
    );
}

#[test]
fn queued_model_applies_to_the_chat_that_asked_not_the_selected_one() {
    let mut app = App::new("original".into());
    app.add_session();
    app.selected = 0;
    assert!(app.queue_model("llama3:8b"));
    app.selected = 1;
    app.receive_models(Ok(vec!["llama3:8b".into()]));
    assert_eq!(app.sessions[0].model, "llama3:8b");
    assert_eq!(app.sessions[1].model, "original");
}

#[test]
fn queued_model_is_refused_with_a_notice_when_its_chat_became_busy() {
    let mut app = App::new("original".into());
    app.add_session();
    app.selected = 0;
    assert!(app.queue_model("llama3:8b"));
    app.sessions[0].status = Status::Failed("x".into());
    app.selected = 1;
    app.receive_models(Ok(vec!["llama3:8b".into()]));
    assert_eq!(app.sessions[0].model, "original");
    assert_eq!(app.sessions[1].model, "original");
    assert!(app.notice.contains("was not applied"), "{}", app.notice);
}

#[test]
fn model_name_before_first_discovery_reports_a_failed_discovery() {
    let mut app = App::new("original".into());
    assert!(app.queue_model("llama3:8b"));
    app.receive_models(Err("Ollama is not reachable".into()));
    assert_eq!(app.sessions[0].model, "original");
    assert!(
        app.notice.contains("Cannot check installed models"),
        "{}",
        app.notice
    );
    assert!(!app.notice.contains("catalog"), "{}", app.notice);
    assert!(
        app.queue_model("llama3:8b"),
        "a failed discovery does not mark the catalog loaded"
    );
}
