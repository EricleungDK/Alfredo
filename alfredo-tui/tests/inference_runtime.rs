use alfredo_tui::{
    inference_admission::{Class, Coordinator},
    inference_runtime::{self, Observation},
    provider::Ollama,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
#[derive(Clone)]
struct Response {
    body: String,
    status: u16,
    stall: bool,
    advertised: Option<usize>,
}
impl Response {
    fn json(value: Value) -> Self {
        Self {
            body: value.to_string(),
            status: 200,
            stall: false,
            advertised: None,
        }
    }
}
struct Server {
    endpoint: String,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    job: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(overrides: impl IntoIterator<Item = (&'static str, Response)>) -> Self {
        let mut replies = BTreeMap::from([
            (
                "/api/version",
                Response::json(json!({"version":"0.34.0","ignored":"not retained"})),
            ),
            (
                "/api/tags",
                Response::json(
                    json!({"models":[{"name":"fixture:latest","model":"fixture:latest","digest":DIGEST,"details":{"quantization_level":"Q4_K_M","context_length":999999}}]}),
                ),
            ),
            (
                "/api/ps",
                Response::json(
                    json!({"models":[{"name":"fixture:latest","model":"fixture:latest","digest":DIGEST,"context_length":4096,"size":2048,"size_vram":1024}]}),
                ),
            ),
        ]);
        replies.extend(overrides);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let history = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let job = thread::spawn(move || {
            let mut children = Vec::new();
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let seen = history.clone();
                        let replies = replies.clone();
                        let stop = stopping.clone();
                        children.push(thread::spawn(move || {
                            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                            let mut header=vec![]; let mut byte=[0];
                            while !header.ends_with(b"\r\n\r\n") {
                                if socket.read_exact(&mut byte).is_err() {return;}
                                header.push(byte[0]); assert!(header.len()<16*1024);
                            }
                            let header=String::from_utf8(header).unwrap();
                            let line=header.lines().next().unwrap(); let mut parts=line.split_whitespace();
                            assert_eq!(parts.next(),Some("GET")); let path=parts.next().unwrap();
                            seen.lock().unwrap().push(path.into());
                            let response=replies.get(path).unwrap();
                            if response.stall {while !stop.load(Ordering::SeqCst) {thread::sleep(Duration::from_millis(2));} return;}
                            let length=response.advertised.unwrap_or(response.body.len());
                            let header=format!("HTTP/1.1 {} Fixture\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n",response.status);
                            let _=socket.write_all(header.as_bytes()).and_then(|()| socket.write_all(response.body.as_bytes()));
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
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
            seen,
            stop,
            job: Some(job),
        }
    }
    fn provider(&self) -> Ollama {
        Ollama::new(&self.endpoint, Duration::from_secs(1)).unwrap()
    }
    async fn inspect(&self) -> Result<Observation, String> {
        inference_runtime::inspect(&self.provider(), "fixture:latest").await
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

#[tokio::test]
async fn selected_runtime_fields_are_distinct_bounded_and_do_not_create_admission_state() {
    let server = Server::new([]);
    #[cfg(unix)]
    let state = PathBuf::from(format!("/tmp/alfredo-inference-{}", unsafe {
        libc::geteuid()
    }))
    .join(format!("{:x}", Sha256::digest(server.endpoint.as_bytes())));
    #[cfg(unix)]
    let before = state.exists();
    let provider = server.provider();
    assert_eq!(provider.models().await.unwrap(), vec!["fixture:latest"]);
    let observed = inference_runtime::inspect(&provider, "fixture:latest")
        .await
        .unwrap();
    observed.validate().unwrap();
    assert!(observed.missing_reasons().is_empty());
    assert_eq!(observed.endpoint_origin, server.endpoint);
    assert_eq!(observed.selected_model, "fixture:latest");
    assert_eq!(observed.server_version.as_deref(), Some("0.34.0"));
    assert_eq!(
        observed.catalog.as_ref().unwrap().digest.as_deref(),
        Some(DIGEST)
    );
    assert_eq!(
        observed.catalog.as_ref().unwrap().quantization.as_deref(),
        Some("Q4_K_M")
    );
    assert_eq!(
        observed.running.as_ref().unwrap().context_length,
        Some(4096)
    );
    let bytes = serde_json::to_vec(&observed).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("999999"));
    assert_eq!(
        serde_json::from_slice::<Observation>(&bytes).unwrap(),
        observed
    );
    let mut seen = server.seen.lock().unwrap().clone();
    seen.sort();
    assert_eq!(
        seen,
        vec!["/api/ps", "/api/tags", "/api/tags", "/api/version"]
    );
    #[cfg(unix)]
    assert_eq!(state.exists(), before);
}
#[tokio::test]
async fn metadata_bypasses_occupied_capacity_and_leaves_the_exact_ledger_unchanged() {
    let server = Server::new([]);
    let coordinator = Coordinator::new(server.endpoint.clone(), 1).unwrap();
    let permit = coordinator.acquire(Class::Foreground).await.unwrap();
    #[cfg(unix)]
    let file = PathBuf::from(format!("/tmp/alfredo-inference-{}", unsafe {
        libc::geteuid()
    }))
    .join(format!("{:x}", Sha256::digest(server.endpoint.as_bytes())))
    .join("ledger.json");
    #[cfg(unix)]
    let before = fs::read(&file).unwrap();
    let observed = tokio::time::timeout(Duration::from_secs(2), server.inspect())
        .await
        .unwrap()
        .unwrap();
    assert!(observed.missing_reasons().is_empty());
    #[cfg(unix)]
    assert_eq!(fs::read(file).unwrap(), before);
    drop(permit);
}
#[tokio::test]
async fn missing_fields_stay_unknown_and_catalog_context_is_never_inferred_as_running_context() {
    let server = Server::new([
        ("/api/version", Response::json(json!({}))),
        (
            "/api/tags",
            Response::json(
                json!({"models":[{"name":"fixture:latest","details":{"context_length":40960}}]}),
            ),
        ),
        (
            "/api/ps",
            Response::json(json!({"models":[{"model":"fixture:latest","size_vram":0}]})),
        ),
    ]);
    let observed = server.inspect().await.unwrap();
    assert!(observed.server_version.is_none());
    assert!(observed.catalog.unwrap().digest.is_none());
    let running = observed.running.unwrap();
    assert!(running.digest.is_none());
    assert!(running.context_length.is_none());
    assert!(running.size.is_none());
    assert_eq!(running.size_vram, Some(0));
    let server = Server::new([("/api/ps", Response::json(json!({"models":[]})))]);
    let observed = server.inspect().await.unwrap();
    assert!(observed.running.is_none());
    assert_eq!(
        observed.missing_reasons(),
        vec!["Selected model not observed running"]
    );
}
#[tokio::test]
async fn duplicate_selected_names_conflicting_aliases_and_wrong_running_digests_refuse() {
    for models in [
        json!([{"name":"fixture:latest","digest":DIGEST},{"name":"fixture:latest","digest":DIGEST}]),
        json!([{"name":"fixture:latest","model":"different:latest","digest":DIGEST}]),
        json!([{"name":"different:latest","model":"fixture:latest","digest":DIGEST}]),
    ] {
        let server = Server::new([("/api/tags", Response::json(json!({"models":models})))]);
        assert!(server.inspect().await.is_err());
    }
    let server = Server::new([(
        "/api/ps",
        Response::json(json!({"models":[{"name":"fixture:latest","digest":"b".repeat(64)}]})),
    )]);
    assert!(server
        .inspect()
        .await
        .unwrap_err()
        .contains("does not match"));
    let server = Server::new([(
        "/api/ps",
        Response::json(json!({"models":[{"name":"fixture:latest"},{"model":"fixture:latest"}]})),
    )]);
    assert!(server.inspect().await.unwrap_err().contains("duplicated"));
}
#[tokio::test]
async fn malformed_numeric_and_identity_fields_cannot_supply_runtime_proof() {
    for (field, value) in [
        ("context_length", json!(0)),
        ("context_length", json!(-1)),
        ("context_length", json!(1.5)),
        ("context_length", json!(1u64 << 25)),
        ("size", json!("2048")),
        ("size", json!(u64::MAX)),
        ("size_vram", json!(2049)),
        ("digest", json!(format!("sha256:{DIGEST}"))),
        ("digest", json!("A".repeat(64))),
    ] {
        let mut entry = json!({"name":"fixture:latest","digest":DIGEST,"context_length":4096,"size":2048,"size_vram":1024});
        entry[field] = value;
        let server = Server::new([("/api/ps", Response::json(json!({"models":[entry]})))]);
        assert!(server.inspect().await.is_err(), "{field}");
    }
    for version in [json!("bad\nversion"), json!("x".repeat(129)), json!(42)] {
        let server = Server::new([("/api/version", Response::json(json!({"version":version})))]);
        assert!(server.inspect().await.is_err());
    }
}
#[tokio::test]
async fn malformed_oversized_deep_duplicate_key_and_excessive_catalog_responses_refuse() {
    let mut deep = json!(0);
    for _ in 0..10 {
        deep = json!({"nested":deep});
    }
    for body in [
        "{".to_string(),
        "{\"version\":\"a\",\"version\":\"b\"}".to_string(),
        deep.to_string(),
        format!("{{\"version\":\"{}\"}}", "x".repeat(1024 * 1024)),
    ] {
        let mut response = Response::json(json!({}));
        response.body = body;
        let server = Server::new([("/api/version", response)]);
        assert!(server.inspect().await.is_err());
    }
    let server = Server::new([(
        "/api/tags",
        Response::json(json!({"models":vec![json!({"name":"other"});257]})),
    )]);
    assert!(server.inspect().await.unwrap_err().contains("256"));
    let mut response = Response::json(json!({"version":"0.34.0"}));
    response.advertised = Some(1024 * 1024 + 1);
    let server = Server::new([("/api/version", response)]);
    assert!(server.inspect().await.unwrap_err().contains("1 MiB"));
}
#[tokio::test]
async fn runtime_drift_distinguishes_identity_changes_from_expected_residency_and_context_changes()
{
    let before = Server::new([]).inspect().await.unwrap();
    let mut after = before.clone();
    after.running = None;
    assert!(before.drift_reasons(&after).is_empty());
    after.running = before.running.clone();
    after.running.as_mut().unwrap().context_length = Some(8192);
    after.running.as_mut().unwrap().size = Some(4096);
    assert!(before.drift_reasons(&after).is_empty());
    after.catalog.as_mut().unwrap().digest = Some("b".repeat(64));
    after.running.as_mut().unwrap().digest = Some("b".repeat(64));
    after.server_version = Some("0.35.0".into());
    after.catalog.as_mut().unwrap().quantization = Some("Q8_0".into());
    after.validate().unwrap();
    assert_eq!(before.drift_reasons(&after).len(), 4);
    after.running.as_mut().unwrap().digest = Some(DIGEST.into());
    assert!(after.validate().is_err());
}
#[tokio::test]
async fn explicit_metadata_http_errors_do_not_fabricate_unknown_success() {
    let mut response = Response::json(json!({}));
    response.status = 404;
    let server = Server::new([("/api/version", response)]);
    assert!(server.inspect().await.unwrap_err().contains("HTTP 404"));
    let server = Server::new([]);
    assert!(inference_runtime::inspect(&server.provider(), "bad\nmodel")
        .await
        .is_err());
    assert!(server.seen.lock().unwrap().is_empty());
}
#[tokio::test]
async fn stalled_metadata_is_bounded_by_the_inspection_deadline() {
    let mut response = Response::json(json!({"version":"0.34.0"}));
    response.stall = true;
    let server = Server::new([("/api/version", response)]);
    let result = tokio::time::timeout(Duration::from_secs(12), server.inspect())
        .await
        .expect("inspection exceeded its bounded deadline");
    assert!(result.unwrap_err().contains("ten-second deadline"));
}

#[tokio::test]
async fn per_request_runtime_binding_requires_its_own_requested_context_and_complete_identity() {
    let server = Server::new([]);
    let mut observed = server.inspect().await.unwrap();
    assert!(observed
        .request_reasons(&server.endpoint, "fixture:latest", Some(4096))
        .is_empty());
    assert!(observed
        .request_reasons(&server.endpoint, "fixture:latest", Some(8192))
        .iter()
        .any(|reason| reason.contains("requested profile")));
    assert!(observed
        .request_reasons("http://localhost:11434", "fixture:latest", None)
        .iter()
        .any(|reason| reason.contains("endpoint")));
    assert!(observed
        .request_reasons(&server.endpoint, "other:latest", None)
        .iter()
        .any(|reason| reason.contains("selected model")));
    observed.running.as_mut().unwrap().context_length = None;
    assert!(!observed
        .request_reasons(&server.endpoint, "fixture:latest", None)
        .is_empty());
    assert!(observed
        .request_reasons(&server.endpoint, "fixture:latest", Some(4096))
        .iter()
        .any(|reason| reason.contains("requested profile")));
    observed.catalog.as_mut().unwrap().digest = None;
    assert!(observed
        .request_reasons(&server.endpoint, "fixture:latest", None)
        .iter()
        .any(|reason| reason.contains("Catalog model digest")));
}
