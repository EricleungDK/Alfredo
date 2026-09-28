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
