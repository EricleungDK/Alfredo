use alfredo_tui::{
    assessment::Outcome,
    inference_profile::RequestRecorder,
    provider::Ollama,
    qualification::{self, Phase},
    qualification_runner::{self, Scenario, ScenarioOutcome, ScenarioResult},
    tasks::{Action, Snapshot, TaskStatus},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const PORT_SOLUTION: &str = "def parse_port(text):\n    if not isinstance(text, str) or not text or any(c < '0' or c > '9' for c in text):\n        return None\n    value = int(text)\n    return value if 1 <= value <= 65535 else None\n";
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "alfredo-qualification-runner-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Server {
    endpoint: String,
    provider: Ollama,
    requests: Arc<Mutex<Vec<Value>>>,
    worker_seen: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(unsafe_policy: bool, wrong_worker: bool, delay: Duration) -> Self {
        Self::with_inspection_cancel(unsafe_policy, wrong_worker, delay, None)
    }
    fn with_inspection_cancel(
        unsafe_policy: bool,
        wrong_worker: bool,
        delay: Duration,
        inspection_cancel: Option<Arc<AtomicBool>>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let worker_seen = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop_thread = stop.clone();
        let seen_thread = worker_seen.clone();
        let captured = requests.clone();
        let thread = thread::spawn(move || {
            let mut version_responses = 0;
            while !stop_thread.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("fixture listener failed: {error}"),
                };
                let (path, body) = read_request(&mut stream);
                let answer = if path.starts_with("GET ") {
                    if path.contains("/api/version ") {
                        version_responses += 1;
                        if version_responses == 2 {
                            if let Some(cancel) = &inspection_cancel {
                                cancel.store(true, Ordering::SeqCst);
                            }
                        }
                        json!({"version":"fixture-1"})
                    } else {
                        json!({"models":[{"name":"fixture","model":"fixture","digest":"b".repeat(64),"context_length":8192,"size":100,"size_vram":100,"details":{"quantization_level":"Q4_K_M"}}]})
                    }
                } else {
                    let body: Value = serde_json::from_slice(&body).unwrap();
                    captured.lock().unwrap().push(body.clone());
                    let prompt = body["messages"].as_array().unwrap().last().unwrap()["content"]
                        .as_str()
                        .unwrap();
                    let response = if prompt.starts_with("Implement this task:") {
                        seen_thread.store(true, Ordering::SeqCst);
                        thread::sleep(delay);
                        let seed = prompt
                            .lines()
                            .next()
                            .unwrap()
                            .contains("QUALIFICATION_REPAIR_SEED")
                            && !prompt.starts_with("Implement this task: Repair");
                        let content = if prompt.contains("FILE reference_left.py") {
                            "def transform(value):\n    return value * 17 + 23\n"
                        } else if wrong_worker || seed {
                            "def parse_port(text):\n    return 0\n"
                        } else {
                            PORT_SOLUTION
                        };
                        json!({"files":[{"path":"solution.py","content":content}]})
                    } else if prompt.starts_with("Classify the decimal port numbers") {
                        json!({"valid":[1,80,443,65535],"invalid":[0,65536]})
                    } else {
                        let mut policy = parse_suffix(prompt, "The exact policy must be ");
                        let criteria = parse_suffix(
                            prompt,
                            "Preserve these exact ordered acceptance criteria: ",
                        );
                        let title = if prompt.contains("QUALIFICATION_REPAIR_SEED") {
                            "QUALIFICATION_REPAIR_SEED: keep parse_port returning 0 for this initial diagnostic attempt"
                        } else if policy["files"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|v| v == "reference_left.py")
                        {
                            "Implement solution.transform(value) using both reference files"
                        } else {
                            "Implement strict ASCII port parsing in solution.parse_port"
                        };
                        if unsafe_policy {
                            policy["files"]
                                .as_array_mut()
                                .unwrap()
                                .push(json!("check_fixture.py"));
                        }
                        json!({"tasks":[{"title":title,"acceptance":criteria,"model":body["model"],"dependencies":[],"policy":policy}]})
                    };
                    json!({"message":{"role":"assistant","content":response.to_string()},"done":true,"prompt_eval_count":800,"eval_count":80,"total_duration":10_000_000,"load_duration":1_000_000,"prompt_eval_duration":3_000_000,"eval_duration":6_000_000})
                };
                let bytes = format!("{answer}\n");
                let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                // A cancelled request is allowed to close before its fixture response.
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(bytes.as_bytes());
            }
        });
        Self {
            provider: Ollama::new(&endpoint, Duration::from_secs(5))
                .unwrap()
                .with_parallelism(1)
                .unwrap()
                .with_request_recorder(RequestRecorder::new()),
            endpoint,
            requests,
            worker_seen,
            stop,
            thread: Some(thread),
        }
    }
}

#[tokio::test]
async fn cancellation_during_case_inspection_starts_no_fixture_or_generation() {
    let scratch = Scratch::new();
    let cancel = Arc::new(AtomicBool::new(false));
    let server = Server::with_inspection_cancel(false, false, Duration::ZERO, Some(cancel.clone()));
    let report_path = scratch.0.join("cancelled-during-inspection.json");
    let report = tokio::time::timeout(
        Duration::from_secs(20),
        qualification::run(
            &report_path,
            &server.endpoint,
            "fixture",
            1,
            Some(false),
            cancel,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!report.finished);
    assert!(report
        .stop_reason
        .as_ref()
        .unwrap()
        .contains("during inspection"));
    assert_eq!(report.cases[0].phase, Phase::Finished);
    assert!(report.cases[0].before.is_some());
    assert!(report.cases[0].result.is_none());
    assert!(report.cases[1..]
        .iter()
        .all(|case| case.phase == Phase::Pending));
    assert!(report.cases.iter().all(|case| case.requests.is_empty()));
    assert!(server.requests.lock().unwrap().is_empty());
    assert!(fs::read_dir(&report.manifest.artifact_directory)
        .unwrap()
        .next()
        .is_none());
    let persisted = qualification::read(&report_path).unwrap();
    assert!(!persisted.finished);
    assert_eq!(persisted.sha256, report.sha256);
    assert!(server.requests.lock().unwrap().is_empty());
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn parse_suffix(prompt: &str, marker: &str) -> Value {
    serde_json::Deserializer::from_str(prompt.split_once(marker).unwrap().1)
        .into_iter::<Value>()
        .next()
        .unwrap()
        .unwrap()
}
fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut data = Vec::new();
    let mut buffer = [0u8; 4096];
    let boundary = loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        data.extend_from_slice(&buffer[..count]);
        assert!(data.len() <= 1024 * 1024);
        if let Some(offset) = data.windows(4).position(|v| v == b"\r\n\r\n") {
            break offset + 4;
        }
    };
    let header = String::from_utf8(data[..boundary].to_vec()).unwrap();
    let length = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    assert!(length <= 1024 * 1024);
    while data.len() < boundary + length {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        data.extend_from_slice(&buffer[..count]);
    }
    (
        header.lines().next().unwrap().to_owned(),
        data[boundary..boundary + length].to_vec(),
    )
}
fn snapshot(case: &Path) -> Snapshot {
    let entries: Vec<_> = fs::read_dir(case.join("state/rust-tasks-v1"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1);
    serde_json::from_slice(&fs::read(entries[0].join("tasks.json")).unwrap()).unwrap()
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn assert_receipts(result: &ScenarioResult) {
    let snapshot = snapshot(&result.artifact_directory);
    for run in &result.runs {
        let task = snapshot.tasks.iter().find(|t| t.id == run.task).unwrap();
        assert_eq!(task.status, run.status);
        assert_eq!(task.run.as_ref().unwrap().id, run.run);
        assert_eq!(
            task.run.as_ref().unwrap().evidence_sha256.as_ref(),
            Some(&run.evidence_sha256)
        );
        let review = run.review.as_ref().unwrap();
        let receipt = snapshot
            .receipts
            .iter()
            .find(|r| r.revision == review.receipt.revision)
            .unwrap();
        assert_eq!(receipt.task, review.receipt.task);
        assert_eq!(receipt.request.correlation, review.receipt.correlation);
        assert_eq!(
            sha(&serde_json::to_vec(receipt).unwrap()),
            review.receipt.sha256
        );
        match &receipt.request.action {
            Action::Decide { task, decision } | Action::ReviewAndRepair { task, decision } => {
                assert_eq!(*task, review.reviewed_task);
                assert_eq!(decision.outcome, review.outcome);
            }
            other => panic!("not a canonical review: {other:?}"),
        }
    }
}

#[tokio::test]
async fn saved_fixture_contract_reaches_workers_without_relying_on_planner_titles() {
    let scratch = Scratch::new();
    for scenario in [
        Scenario::SmallEdit,
        Scenario::RequiredSource,
        Scenario::Repair,
    ] {
        let server = Server::new(false, false, Duration::ZERO);
        let result = qualification_runner::run_scenario(
            &scratch.0.join(format!("{scenario:?}")),
            server.provider.clone(),
            "fixture",
            scenario,
        )
        .await
        .unwrap();
        assert!(result.accepted(), "{scenario:?}: {:?}", result.failure);
        let saved = snapshot(&result.artifact_directory);
        let requests = server.requests.lock().unwrap();
        let workers: Vec<_> = requests
            .iter()
            .filter_map(|request| {
                let prompt = request["messages"].as_array().unwrap().last().unwrap()["content"]
                    .as_str()
                    .unwrap();
                prompt.starts_with("Implement this task:").then_some(prompt)
            })
            .collect();
        assert_eq!(workers.len(), result.runs.len());
        for (prompt, run) in workers.iter().zip(&result.runs) {
            // Assert only the contract section of the actual HTTP payload. The
            // fake planner deliberately leaves these requirements out of titles.
            let title = prompt.lines().next().unwrap();
            assert!(!title.contains("None"));
            assert!(!title.contains("value * RATE + OFFSET"));
            assert!(!title.contains("byte-for-byte"));
            let criteria: Vec<String> =
                serde_json::from_value(parse_suffix(prompt, "\nRECORDED ACCEPTANCE CRITERIA\n"))
                    .unwrap();
            assert_eq!(criteria, saved.acceptance_for_task(run.task));
            let contract = criteria.join("\n");
            let requirements: &[&str] = if scenario == Scenario::RequiredSource {
                &[
                    "solution.transform(value)",
                    "value * RATE + OFFSET",
                    "Python int",
                    "positive, zero and negative integer inputs",
                    "Read RATE from reference_left.py and OFFSET from reference_right.py",
                    "Preserve reference_left.py and reference_right.py byte-for-byte",
                ]
            } else {
                &[
                    "solution.parse_port(text)",
                    "nonempty string",
                    "only ASCII decimal digits 0 through 9",
                    "1 through 65535 inclusive",
                    "return that integer as a Python int",
                    "Leading zeros are allowed",
                    "Return Python None for every other input",
                    "non-string values (None, booleans, numbers and collections)",
                    "empty strings, whitespace, signs, decimal points, Unicode digits and out-of-range values",
                    "Do not raise for rejected inputs",
                ]
            };
            for requirement in requirements {
                assert!(
                    contract.contains(requirement),
                    "{scenario:?} worker contract lacks {requirement:?}: {contract}"
                );
            }
            if scenario == Scenario::RequiredSource {
                assert!(!contract.contains("17") && !contract.contains("23"));
                assert!(
                    prompt.contains("FILE reference_left.py\n# Required fixture fact: RATE = 17")
                );
                assert!(prompt.contains("# Required fixture fact: OFFSET = 23"));
            } else {
                assert!(!contract.contains("QUALIFICATION_REPAIR_SEED"));
                assert!(!contract.contains("returning 0"));
            }
        }
        if scenario == Scenario::Repair {
            assert_eq!(workers.len(), 2);
            assert!(workers[0].starts_with("Implement this task: QUALIFICATION_REPAIR_SEED"));
            assert!(workers[1].starts_with("Implement this task: Repair"));
            assert!(!result.runs[0].check_passed && result.runs[1].check_passed);
        }
    }
}

#[tokio::test]
async fn governed_scenarios_retain_exact_review_evidence_and_required_sources() {
    let scratch = Scratch::new();
    let server = Server::new(false, false, Duration::from_millis(350));
    for (index, scenario) in Scenario::ALL.into_iter().enumerate() {
        let case = scratch.0.join(format!("case-{index}"));
        let result =
            qualification_runner::run_scenario(&case, server.provider.clone(), "fixture", scenario)
                .await
                .unwrap();
        assert!(result.accepted(), "{scenario:?}: {:?}", result.failure);
        result.validate().unwrap();
        assert_receipts(&result);
        assert_eq!(result.scope_revision, 2);
        assert!(result.reviewed_ms().is_some());
        assert!(result.required_sources_present);
        assert_eq!(
            result.fixture_digest,
            qualification_runner::fixture_definition(scenario).digest()
        );
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("def parse_port"));
        let mut invalid = result.clone();
        invalid.required_sources_present = false;
        assert!(invalid.validate().is_err());
        invalid = result.clone();
        invalid
            .planner_sources
            .push(invalid.planner_sources[0].clone());
        assert!(invalid.validate().is_err());
        invalid = result.clone();
        invalid.plan_receipt.as_mut().unwrap().task += 100;
        assert!(invalid.validate().is_err());
        invalid = result.clone();
        invalid.runs.last_mut().unwrap().check_exit_code = Some(1);
        assert!(invalid.validate().is_err());
        invalid = result.clone();
        invalid
            .runs
            .last_mut()
            .unwrap()
            .review
            .as_mut()
            .unwrap()
            .receipt
            .task += 100;
        assert!(invalid.validate().is_err());
        invalid = result.clone();
        invalid.runs.last_mut().unwrap().review = None;
        assert!(invalid.validate().is_err());
        invalid = result.clone();
        invalid.runs.push(invalid.runs[0].clone());
        assert!(invalid.validate().is_err());
        if scenario == Scenario::Repair {
            assert_eq!(result.runs.len(), 2);
            assert!(!result.runs[0].check_passed);
            assert_eq!(
                result.runs[0].review.as_ref().unwrap().outcome,
                Outcome::NeedsRepair
            );
            assert_eq!(result.runs[1].repair_of, Some(result.runs[0].task));
            invalid = result.clone();
            invalid.runs[0].review.as_mut().unwrap().receipt.task = invalid.runs[0].task;
            assert!(invalid.validate().is_err());
            invalid = result.clone();
            invalid.runs[1].repair_of = None;
            assert!(invalid.validate().is_err());
        }
        if scenario == Scenario::RequiredSource {
            assert!(
                result
                    .planner_sources
                    .iter()
                    .filter(|s| s.required)
                    .map(|s| s.bytes)
                    .sum::<usize>()
                    > 15_000
            );
            let requests = server.requests.lock().unwrap();
            let prompt = requests.last().unwrap()["messages"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["content"]
                .as_str()
                .unwrap();
            assert!(prompt.contains("RATE = 17") && prompt.contains("OFFSET = 23"));
        } else {
            assert_eq!(
                fs::read_to_string(case.join("workspace/solution.py")).unwrap(),
                "def parse_port(text):\n    return 0\n"
            );
        }
        if scenario == Scenario::QueuedForeground {
            let foreground = result.foreground.unwrap();
            assert!(foreground.queue_observed && foreground.completed && foreground.check_passed);
            assert!(foreground.queue_ms.unwrap() >= 100);
        }
    }
    assert_eq!(server.requests.lock().unwrap().len(), 10);
}

#[tokio::test]
async fn unsafe_plan_and_failed_check_remain_failed_without_reviewed_latency() {
    let scratch = Scratch::new();
    for unsafe_policy in [true, false] {
        let server = Server::new(unsafe_policy, true, Duration::from_millis(10));
        let result = qualification_runner::run_scenario(
            &scratch.0.join(format!("case-{unsafe_policy}")),
            server.provider.clone(),
            "fixture",
            Scenario::SmallEdit,
        )
        .await
        .unwrap();
        assert_eq!(result.outcome, ScenarioOutcome::Failed);
        assert!(result.reviewed_ms().is_none() && result.failure.is_some());
        if unsafe_policy {
            assert!(result.runs.is_empty());
            assert_eq!(server.requests.lock().unwrap().len(), 1);
        } else {
            assert_eq!(result.runs.len(), 1);
            assert!(!result.runs[0].check_passed);
            assert_eq!(result.runs[0].status, TaskStatus::Rejected);
            assert_eq!(
                result.runs[0].review.as_ref().unwrap().outcome,
                Outcome::Rejected
            );
            assert_receipts(&result);
        }
    }
}

#[tokio::test]
async fn cancellation_joins_the_claimed_worker_and_retains_its_canonical_result() {
    let scratch = Scratch::new();
    let server = Server::new(false, false, Duration::from_secs(2));
    let cancel = Arc::new(AtomicBool::new(false));
    let job_cancel = cancel.clone();
    let provider = server.provider.clone();
    let case = scratch.0.join("cancelled");
    let job_case = case.clone();
    let job = tokio::spawn(async move {
        qualification_runner::run_scenario_with_cancel(
            &job_case,
            provider,
            "fixture",
            Scenario::SmallEdit,
            job_cancel,
        )
        .await
    });
    let started = Instant::now();
    while !server.worker_seen.load(Ordering::SeqCst) {
        assert!(started.elapsed() < Duration::from_secs(10));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    cancel.store(true, Ordering::SeqCst);
    let result = tokio::time::timeout(Duration::from_secs(10), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.outcome, ScenarioOutcome::Incomplete);
    assert!(result.reviewed_ms().is_none());
    assert_eq!(result.runs.len(), 1);
    assert_eq!(result.runs[0].status, TaskStatus::Cancelled);
    let snapshot = snapshot(&case);
    assert_eq!(snapshot.tasks[0].status, TaskStatus::Cancelled);
    assert!(matches!(
        snapshot.receipts.last().unwrap().request.action,
        Action::Finish { .. }
    ));
    let count = server.requests.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(server.requests.lock().unwrap().len(), count);
}

#[tokio::test]
async fn existing_work_is_refused_before_http_and_fixture_hashes_are_pinned() {
    let scratch = Scratch::new();
    let server = Server::new(false, false, Duration::ZERO);
    let case = scratch.0.join("existing");
    fs::create_dir(&case).unwrap();
    fs::write(case.join("user-work"), "retained").unwrap();
    assert!(qualification_runner::run_scenario(
        &case,
        server.provider.clone(),
        "fixture",
        Scenario::SmallEdit
    )
    .await
    .is_err());
    assert_eq!(
        fs::read_to_string(case.join("user-work")).unwrap(),
        "retained"
    );
    assert!(server.requests.lock().unwrap().is_empty());
    for scenario in Scenario::ALL {
        let mut definition = qualification_runner::fixture_definition(scenario);
        definition.validate().unwrap();
        assert_eq!(definition.version, 2);
        assert!(definition.sources.iter().all(|s| s.bytes <= 8192));
        assert!(!definition
            .writable_paths
            .iter()
            .any(|p| p == "check_fixture.py"));
        definition.sources[0].sha256 = "f".repeat(64);
        assert!(definition.validate().is_err());
    }
}
