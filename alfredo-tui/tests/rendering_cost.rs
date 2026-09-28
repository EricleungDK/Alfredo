//! Opt-in real rendering cohorts; no model, network, subprocess or duration threshold.
use alfredo_tui::{
    commands::Completion,
    conversations::TaskView,
    model::App,
    review::View,
    task_control::TaskControl,
    tasks::{Action, Receipt, Request, Snapshot, TaskStatus, TaskStore, WorkPolicy},
    ui,
    worker::Evidence,
};
use ratatui::{backend::TestBackend, Terminal};
use sha2::{Digest, Sha256};
use std::{fs, hint::black_box, path::PathBuf, time::Instant};

struct Fixture {
    root: PathBuf,
    store: TaskStore,
    snapshot: Snapshot,
    evidence: String,
    bytes: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        // Constant path keeps identities and visible buffers identical between cohorts.
        // Exclusive creation refuses concurrent runs; only this fixture is removed.
        let root = std::env::temp_dir().join("alfredo-mission-rendering-cohort-v1");
        fs::create_dir(&root).expect("Rendering cohort fixture already exists");
        fs::create_dir(root.join("workspace")).unwrap();
        let store = TaskStore::new(
            &root.join("state"),
            &root.join("workspace"),
            "Rendering cohort",
        )
        .unwrap();
        let policy = WorkPolicy {
            files: vec!["answer.py".into()],
            check: vec!["/bin/true".into()],
        };
        let mut fixture = Self {
            root,
            snapshot: store.snapshot().unwrap(),
            store,
            evidence: String::new(),
            bytes: vec![],
        };
        fixture.action(Action::Propose {
            title: "Inspect retained work".into(),
            model: "fixture".into(),
            dependencies: vec![],
        });
        fixture.action(Action::Permit {
            task: 1,
            policy: policy.clone(),
        });
        // Construct the bounded history efficiently, then validate it through the
        // actual store's complete receipt replay before timing any rendering.
        while fixture.snapshot.revision < 4093 {
            let revision = fixture.snapshot.revision + 1;
            fixture.snapshot.receipts.push(Receipt {
                revision,
                task: 1,
                request: Request {
                    correlation: format!("cohort-{revision}"),
                    expected_revision: revision - 1,
                    action: Action::Permit {
                        task: 1,
                        policy: policy.clone(),
                    },
                },
            });
            fixture.snapshot.revision = revision;
        }
        let path = fixture
            .store
            .conversation_directory()
            .unwrap()
            .join("tasks.json");
        fs::write(&path, serde_json::to_vec(&fixture.snapshot).unwrap()).unwrap();
        fixture.snapshot = fixture.store.snapshot().unwrap();
        fixture.action(Action::Approve { task: 1 });
        let owner = fixture.store.claim_worker(1).unwrap();
        fixture.action(Action::Start {
            task: 1,
            baseline: "a".repeat(40),
            inputs: vec![],
        });
        let run = fixture.snapshot.tasks[0].run.as_ref().unwrap().id.clone();
        // Canonical failed evidence fixture: no check or candidate success inferred.
        let detail = "Fixture model stopped before applying files".to_string();
        fixture.evidence = serde_json::to_string(&Evidence {
            agent: None,
            candidate_commit: None,
            model_metrics: None,
            generation: None,
            run: run.clone(),
            baseline: "a".repeat(40),
            status: TaskStatus::Failed,
            detail: detail.clone(),
            patch: String::new(),
            check: None,
        })
        .unwrap();
        let directory = fixture.store.run_directory(&run).unwrap();
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("evidence.json"), &fixture.evidence).unwrap();
        fixture.action(Action::Finish {
            task: 1,
            run,
            status: TaskStatus::Failed,
            evidence_sha256: hash(fixture.evidence.as_bytes()),
            detail,
        });
        drop(owner);
        assert_eq!(fixture.store.evidence(1).unwrap(), fixture.evidence);
        fixture.bytes = fs::read(path).unwrap();
        assert!(fixture.bytes.len() < 4 * 1024 * 1024);
        assert_eq!(fixture.snapshot.receipts.len(), 4096);
        fixture
    }
    fn action(&mut self, action: Action) {
        self.snapshot = self
            .store
            .transact(Request {
                correlation: format!("cohort-{}", self.snapshot.revision + 1),
                expected_revision: self.snapshot.revision,
                action,
            })
            .unwrap()
            .0;
    }
    fn control(&self, panel: &str) -> TaskControl {
        let mut control = TaskControl::new(self.store.clone());
        control.snapshot = Some(self.snapshot.clone());
        control
            .restore_view(TaskView {
                visible: true,
                selected: Some(1),
                query: String::new(),
            })
            .unwrap();
        match panel {
            "evidence" => control.evidence = Some(View::from_verified(1, &self.evidence).unwrap()),
            "activity" => control.activity = Some("#1".into()),
            _ => {}
        }
        control
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn percentile(samples: &[u128], numerator: usize) -> u128 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() * numerator).div_ceil(100).saturating_sub(1)]
}
fn draw(terminal: &mut Terminal<TestBackend>, app: &App, control: &TaskControl) -> u128 {
    let start = Instant::now();
    terminal
        .draw(|frame| ui::draw_with_tasks(frame, black_box(app), black_box(control)))
        .unwrap();
    black_box(terminal.backend().buffer());
    start.elapsed().as_nanos()
}

#[test]
#[ignore = "explicit release-mode Mission Work redraw measurement"]
fn measure_mission_work_redraws() {
    let fixture = Fixture::new();
    let mut cohorts = vec![];
    for (width, height) in [(140, 40), (32, 10)] {
        for panel in ["inspector", "evidence", "activity", "models", "completion"] {
            let mut cold = vec![];
            let mut warm = vec![];
            let mut screen_hashes = vec![];
            for _ in 0..5 {
                let control = fixture.control(panel);
                let mut app = App::new("fixture".into());
                app.sessions[0].insert("Keep this draft");
                if panel == "models" {
                    app.models_visible = true;
                    app.models = vec!["fixture".into()];
                }
                if panel == "completion" {
                    app.completion = Completion::open("/ta");
                    assert!(app.completion.is_some());
                }
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                cold.push(draw(&mut terminal, &app, &control));
                for _ in 0..20 {
                    // Exercise actual changed-frame rendering without growing the fixture.
                    app.sessions[0].insert("x");
                    warm.push(draw(&mut terminal, &app, &control));
                }
                screen_hashes.push(hash(
                    format!("{:?}", terminal.backend().buffer()).as_bytes(),
                ));
            }
            assert!(screen_hashes.iter().all(|value| value == &screen_hashes[0]));
            cohorts.push(serde_json::json!({
                "panel": panel, "viewport": [width,height], "cold_samples_ns": cold,
                "cold_p50_ns": percentile(&cold,50), "cold_p95_ns": percentile(&cold,95),
                "warm_samples_ns": warm, "warm_p50_ns": percentile(&warm,50), "warm_p95_ns": percentile(&warm,95),
                "final_buffer_sha256": screen_hashes[0],
            }));
        }
    }
    assert_eq!(fixture.store.snapshot().unwrap().revision, 4096);
    assert_eq!(
        fs::read(
            fixture
                .store
                .conversation_directory()
                .unwrap()
                .join("tasks.json")
        )
        .unwrap(),
        fixture.bytes
    );
    println!(
        "{}",
        serde_json::json!({"schema_version": 1, "fixture": "validated-4096-receipt-history-v1", "snapshot_bytes": fixture.bytes.len(), "snapshot_sha256": hash(&fixture.bytes), "cohorts": cohorts,
        "scope": "Actual draw_with_tasks with Ratatui TestBackend; cold means fresh presentation caches, not OS/process cold. Excludes terminal IO and model/network latency. No duration threshold."})
    );
}

/// Dashboard with 200 tasks and 4 streaming workers; returns (control, senders, root).
fn streaming_dashboard() -> (
    TaskControl,
    Vec<tokio::sync::watch::Sender<alfredo_tui::worker::Progress>>,
    PathBuf,
) {
    use alfredo_tui::tasks::{Task, TaskRun};
    let root = std::env::temp_dir().join(format!(
        "alfredo-dashboard-cost-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(root.join("workspace")).unwrap();
    let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "Cost").unwrap();
    let mut control = TaskControl::new(store);
    let statuses = [
        TaskStatus::Accepted,
        TaskStatus::Proposed,
        TaskStatus::Approved,
        TaskStatus::ReviewReady,
        TaskStatus::Failed,
    ];
    let tasks: Vec<Task> = (1..=200u64)
        .map(|id| {
            let status = if id <= 4 {
                TaskStatus::Running
            } else {
                statuses[id as usize % statuses.len()].clone()
            };
            Task {
                id,
                title: format!("Task number {id} with a reasonably long descriptive title"),
                model: "fixture".into(),
                dependencies: if id > 1 { vec![id - 1] } else { vec![] },
                status: status.clone(),
                policy: Some(WorkPolicy {
                    files: vec![format!("file{id}.py")],
                    check: vec!["python3".into(), "-m".into(), "unittest".into()],
                }),
                repair_of: None,
                run: (status == TaskStatus::Running).then(|| TaskRun {
                    id: format!("run-{id}"),
                    baseline: "a".repeat(40),
                    inputs: vec![],
                    evidence_sha256: None,
                    detail: "Running".into(),
                }),
            }
        })
        .collect();
    control.snapshot = Some(Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace: root.join("workspace"),
        mission: "Cost".into(),
        revision: 0,
        tasks,
        receipts: vec![],
    });
    control
        .restore_view(TaskView {
            visible: true,
            selected: Some(2),
            query: String::new(),
        })
        .unwrap();
    let mut senders = vec![];
    for task in 1..=4 {
        let (sender, receiver) = tokio::sync::watch::channel(alfredo_tui::worker::Progress {
            stage: "Receiving model plan",
            model_output: "streamed model text line\n".repeat(300),
            check_stdout: b"test output\n".repeat(400),
            ..Default::default()
        });
        control.attach_progress(
            task,
            receiver,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        senders.push(sender);
    }
    (control, senders, root)
}

fn average_streaming_redraw_ms(frames: u32) -> f64 {
    let (control, senders, root) = streaming_dashboard();
    let app = App::new("fixture".into());
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    draw(&mut terminal, &app, &control);
    let mut total = 0u128;
    for frame in 0..frames {
        for sender in &senders {
            sender.send_modify(|progress| {
                progress.model_output.push_str(&format!("token {frame}\n"));
                progress.received_bytes += 8;
            });
        }
        total += draw(&mut terminal, &app, &control);
    }
    let _ = fs::remove_dir_all(root);
    total as f64 / f64::from(frames) / 1e6
}

#[test]
fn dashboard_redraw_with_streaming_workers_is_bounded() {
    let average = average_streaming_redraw_ms(20);
    let limit = if cfg!(debug_assertions) { 50.0 } else { 16.0 };
    assert!(
        average < limit,
        "average redraw {average:.2} ms ≥ {limit} ms"
    );
}

#[test]
#[ignore = "explicit release-mode dashboard redraw measurement"]
fn measure_dashboard_streaming_redraws() {
    let average = average_streaming_redraw_ms(200);
    println!(
        "{}",
        serde_json::json!({"fixture": "200 tasks, 4 streaming workers, 140x40", "average_ms": average})
    );
    assert!(average < 16.0, "average redraw {average:.2} ms ≥ 16 ms");
}
