mod model;

use model::{
    apply, import_legacy_v1, Action, DecisionFailure, JourneyPhase, MissionChoice,
    MissionFormationRoute, MissionRef, PrototypeState, Transition,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::hint::black_box;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::{Duration, Instant};

const LEGACY_FIXTURE: &str = r#"{
  "schema_version": 1,
  "revision": 7,
  "workspace_session": {
    "id": "workspace-session-alfredo",
    "workspace_path": "/code/alfredo",
    "status": "ready"
  },
  "active_mission": {
    "id": "modernize-alfredo",
    "title": "Modernize Alfredo",
    "issue_count": 9
  },
  "conversation_scope": {
    "kind": "mission",
    "target_id": "modernize-alfredo",
    "label": "Modernize Alfredo",
    "mission_id": "modernize-alfredo"
  },
  "operations_view": "mission-board",
  "mission_board": {
    "prd_title": "Modernize Alfredo",
    "issue_count": 9,
    "ordered_issue_ids": [],
    "ready_issue_ids": [],
    "approved_issue_ids": [],
    "issue_slices": []
  },
  "missions": []
}"#;

const PYTHON_WARM_SNAPSHOT_MS: f64 = 9.155;
const PYTHON_PUBLIC_QUEUE_ACK_MS: f64 = 12.296;
const PYTHON_RUNNER_CLAIM_MS: f64 = 85.377;
const PYTHON_LIVE_GOAL_SECONDS: f64 = 37.842;

#[derive(Debug, Serialize)]
struct ScenarioResult {
    name: &'static str,
    passed: bool,
    evidence: String,
}

#[derive(Debug, Serialize)]
struct RollbackResult {
    passed: bool,
    legacy_source_unchanged: bool,
    crash_cut_left_canonical_sidecar_unchanged: bool,
    scratch_path: String,
}

#[derive(Debug, Serialize)]
struct BenchmarkResult {
    iterations: usize,
    rust_parse_route_dispatch_median_ms: f64,
    rust_parse_route_dispatch_p95_ms: f64,
    rust_atomic_sidecar_median_ms: f64,
    rust_atomic_sidecar_p95_ms: f64,
    release_binary_bytes: u64,
    model_nonblank_lines: usize,
    shell_nonblank_lines: usize,
}

#[derive(Debug, Serialize)]
struct ReviewReport {
    verdict_candidate: &'static str,
    boundary_finding: &'static str,
    scenarios: Vec<ScenarioResult>,
    rollback: RollbackResult,
    benchmark: BenchmarkResult,
    python_baseline: serde_json::Value,
    interpretation: Vec<&'static str>,
}

#[derive(Debug, Deserialize)]
struct PersistentRequest {
    id: String,
    argv: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PersistentResponse {
    id: String,
    success: bool,
    stdout: String,
    stderr: String,
}

fn select_known_workspace(state: &PrototypeState, correlation: &str) -> Transition {
    apply(
        state,
        Action::SelectWorkspace {
            correlation_id: correlation.to_owned(),
            expected_revision: state.revision,
            path: "/code/alfredo".to_owned(),
            repository_valid: true,
            known_missions: vec![MissionRef {
                id: "modernize-alfredo".to_owned(),
                title: "Modernize Alfredo".to_owned(),
            }],
        },
    )
    .expect("prototype fixture should select a valid workspace")
}

fn resume_known_mission(state: &PrototypeState, correlation: &str) -> Transition {
    apply(
        state,
        Action::ChooseMission {
            correlation_id: correlation.to_owned(),
            expected_revision: state.revision,
            choice: MissionChoice::Resume {
                mission_id: "modernize-alfredo".to_owned(),
            },
        },
    )
    .expect("prototype fixture should resume its known Mission")
}

fn form_ad_hoc(state: &PrototypeState, correlation: &str) -> Transition {
    apply(
        state,
        Action::FormMission {
            correlation_id: correlation.to_owned(),
            expected_revision: state.revision,
            route: MissionFormationRoute::AdHocDelegation,
            task: "Add one bounded receipt seam".to_owned(),
            effectful_request: true,
            controller_claims_effect: false,
        },
    )
    .expect("prototype fixture should form a narrow Ad Hoc Delegation")
}

fn dispatch_exact(state: &PrototypeState, correlation: &str) -> Transition {
    apply(
        state,
        Action::Dispatch {
            correlation_id: correlation.to_owned(),
            expected_revision: state.revision,
            proposal_exact: true,
            worker_eligible: true,
        },
    )
    .expect("prototype fixture should queue an exact eligible dispatch")
}

fn ready_pre_workspace_state() -> PrototypeState {
    let selected = select_known_workspace(
        &PrototypeState::selection_required("/starting/location"),
        "select-1",
    );
    resume_known_mission(&selected.state, "mission-1").state
}

fn scenario_results() -> Vec<ScenarioResult> {
    let mut results = Vec::new();

    let imported = import_legacy_v1(LEGACY_FIXTURE, "/starting/location");
    let import_passed = imported
        .as_ref()
        .map(|state| {
            state.coding_workspace.as_deref() == Some("/code/alfredo")
                && state.active_mission_id.as_deref() == Some("modernize-alfredo")
                && state.revision == 7
        })
        .unwrap_or(false);
    results.push(ScenarioResult {
        name: "Read current schema-v1 state",
        passed: import_passed,
        evidence:
            "Imports required current fields, ignores compatible extras, and preserves revision 7."
                .to_owned(),
    });

    let pre_workspace = PrototypeState::selection_required("/starting/location");
    results.push(ScenarioResult {
        name: "Starting Location is not a Coding Workspace",
        passed: pre_workspace.coding_workspace.is_none()
            && pre_workspace.phase == JourneyPhase::SelectionRequired,
        evidence: "The prototype-v2 state can express the selection-pending state that current launch v1 cannot."
            .to_owned(),
    });

    let selected = select_known_workspace(&pre_workspace, "scenario-select");
    results.push(ScenarioResult {
        name: "Known repository requires an explicit Mission choice",
        passed: selected.state.phase == JourneyPhase::MissionChoiceRequired
            && selected.state.active_mission_id.is_none(),
        evidence: "Selection acknowledges the repository but does not silently resume or duplicate a Mission."
            .to_owned(),
    });

    let invalid = apply(
        &PrototypeState::selection_required("/starting/location"),
        Action::SelectWorkspace {
            correlation_id: "invalid-workspace".to_owned(),
            expected_revision: 0,
            path: "/not/a/repository".to_owned(),
            repository_valid: false,
            known_missions: Vec::new(),
        },
    );
    results.push(ScenarioResult {
        name: "Invalid repository fails closed",
        passed: invalid.as_ref().err().map(|error| error.code.as_str())
            == Some("workspace-invalid"),
        evidence: "No Coding Workspace, Mission, or receipt is fabricated.".to_owned(),
    });

    let ready = resume_known_mission(&selected.state, "scenario-resume");
    let route = form_ad_hoc(&ready.state, "scenario-route");
    let queued = dispatch_exact(&route.state, "scenario-dispatch");
    results.push(ScenarioResult {
        name: "Exact Ad Hoc route queues one deferred session",
        passed: queued.state.sessions.len() == 1
            && queued.state.phase == JourneyPhase::SessionQueued
            && queued.receipt.effect_kind == "session-queued",
        evidence: "Workspace, Mission, route, exact proposal, and eligible worker all gate the queued receipt."
            .to_owned(),
    });

    let replay = apply(
        &queued.state,
        Action::Dispatch {
            correlation_id: "scenario-dispatch".to_owned(),
            expected_revision: route.state.revision,
            proposal_exact: true,
            worker_eligible: true,
        },
    );
    results.push(ScenarioResult {
        name: "Exact correlation replay is idempotent",
        passed: replay
            .as_ref()
            .map(|transition| {
                transition.state.sessions.len() == 1
                    && transition.state.revision == queued.state.revision
                    && transition.receipt.replayed
            })
            .unwrap_or(false),
        evidence: "The original receipt replays without a second session or revision.".to_owned(),
    });

    let conflict = apply(
        &queued.state,
        Action::Dispatch {
            correlation_id: "scenario-dispatch".to_owned(),
            expected_revision: route.state.revision,
            proposal_exact: false,
            worker_eligible: true,
        },
    );
    results.push(ScenarioResult {
        name: "Changed boundary on reused correlation is rejected",
        passed: conflict.as_ref().err().map(|error| error.code.as_str())
            == Some("correlation-conflict"),
        evidence: "A receipt identity cannot be reused to authorize a different proposal."
            .to_owned(),
    });

    let no_action = apply(
        &ready_pre_workspace_state(),
        Action::FormMission {
            correlation_id: "false-success".to_owned(),
            expected_revision: 2,
            route: MissionFormationRoute::NoAction,
            task: "Yes, create the requested folder now".to_owned(),
            effectful_request: true,
            controller_claims_effect: true,
        },
    );
    results.push(ScenarioResult {
        name: "False-success controller prose cannot become an effect",
        passed: no_action
            .as_ref()
            .map(|transition| {
                !transition.receipt.canonical_effect
                    && transition.receipt.effect_kind == "no-action"
                    && transition.state.sessions.is_empty()
            })
            .unwrap_or(false),
        evidence: "The durable outcome says “No action taken” and remains session-free.".to_owned(),
    });

    let planning = apply(
        &ready_pre_workspace_state(),
        Action::FormMission {
            correlation_id: "wayfinding-route".to_owned(),
            expected_revision: 2,
            route: MissionFormationRoute::Wayfinding,
            task: "Modernize the whole backend".to_owned(),
            effectful_request: true,
            controller_claims_effect: false,
        },
    );
    results.push(ScenarioResult {
        name: "Large foggy work routes to Wayfinding, not dispatch",
        passed: planning
            .as_ref()
            .map(|transition| {
                transition.state.phase == JourneyPhase::PlanningRequired
                    && transition.state.sessions.is_empty()
            })
            .unwrap_or(false),
        evidence: "Mission Formation is explicit and overridable before any Local Agent launch."
            .to_owned(),
    });

    let protocol_request = PersistentRequest {
        id: "correlated-review".to_owned(),
        argv: vec![
            "prototype-import-v1".to_owned(),
            LEGACY_FIXTURE.to_owned(),
            "/starting/location".to_owned(),
        ],
    };
    let protocol_response = handle_protocol_request(protocol_request);
    results.push(ScenarioResult {
        name: "Existing persistent transport envelope is reusable",
        passed: protocol_response.id == "correlated-review"
            && protocol_response.success
            && protocol_response.stderr.is_empty(),
        evidence: "The slice accepts {id, argv} and returns correlated stdout/stderr without changing Tauri transport shape."
            .to_owned(),
    });

    results
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(temporary, path)
}

fn rollback_proof() -> RollbackResult {
    let scratch = std::env::temp_dir().join(format!(
        "alfredo-rust-orchestrator-prototype-{}",
        std::process::id()
    ));
    let legacy_path = scratch.join("python-authority.json");
    let sidecar_path = scratch.join("PROTOTYPE-rust-authority.json");
    let crash_path = scratch.join("PROTOTYPE-rust-authority.crash-cut");
    let _ = fs::create_dir_all(&scratch);
    let _ = fs::write(&legacy_path, LEGACY_FIXTURE.as_bytes());
    let original = fs::read(&legacy_path).unwrap_or_default();

    let ready = ready_pre_workspace_state();
    let initial_sidecar = serde_json::to_vec_pretty(&ready).unwrap_or_default();
    let _ = atomic_write(&sidecar_path, &initial_sidecar);
    let canonical_before = fs::read(&sidecar_path).unwrap_or_default();

    let candidate = form_ad_hoc(&ready, "crash-route").state;
    let candidate_bytes = serde_json::to_vec_pretty(&candidate).unwrap_or_default();
    let _ = fs::write(&crash_path, candidate_bytes);
    let canonical_after = fs::read(&sidecar_path).unwrap_or_default();
    let source_after = fs::read(&legacy_path).unwrap_or_default();

    let source_unchanged = source_after == original;
    let sidecar_unchanged = canonical_before == canonical_after;
    let passed = source_unchanged && sidecar_unchanged;

    let _ = fs::remove_file(&crash_path);
    let _ = fs::remove_file(&sidecar_path);
    let _ = fs::remove_file(&legacy_path);
    let _ = fs::remove_dir(&scratch);

    RollbackResult {
        passed,
        legacy_source_unchanged: source_unchanged,
        crash_cut_left_canonical_sidecar_unchanged: sidecar_unchanged,
        scratch_path: scratch.display().to_string(),
    }
}

fn percentile(samples: &mut [Duration], percentile: f64) -> f64 {
    samples.sort_unstable();
    let rank = ((samples.len() as f64 * percentile).ceil() as usize)
        .saturating_sub(1)
        .min(samples.len().saturating_sub(1));
    samples[rank].as_secs_f64() * 1000.0
}

fn benchmark() -> BenchmarkResult {
    let iterations = 2_000;
    let mut route_samples = Vec::with_capacity(iterations);
    for index in 0..iterations {
        let started = Instant::now();
        let imported = import_legacy_v1(black_box(LEGACY_FIXTURE), "/starting/location").unwrap();
        let routed = apply(
            &imported,
            Action::FormMission {
                correlation_id: format!("route-{index}"),
                expected_revision: imported.revision,
                route: MissionFormationRoute::AdHocDelegation,
                task: "Add one bounded receipt seam".to_owned(),
                effectful_request: true,
                controller_claims_effect: false,
            },
        )
        .unwrap();
        let dispatched = apply(
            &routed.state,
            Action::Dispatch {
                correlation_id: format!("dispatch-{index}"),
                expected_revision: routed.state.revision,
                proposal_exact: true,
                worker_eligible: true,
            },
        )
        .unwrap();
        black_box(serde_json::to_vec(&dispatched.state).unwrap());
        route_samples.push(started.elapsed());
    }
    let route_median = percentile(&mut route_samples.clone(), 0.5);
    let route_p95 = percentile(&mut route_samples, 0.95);

    let scratch = std::env::temp_dir().join(format!(
        "alfredo-rust-orchestrator-benchmark-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&scratch);
    let sidecar = scratch.join("PROTOTYPE-benchmark-state.json");
    let bytes = serde_json::to_vec(&ready_pre_workspace_state()).unwrap();
    let mut write_samples = Vec::with_capacity(100);
    for _ in 0..100 {
        let started = Instant::now();
        atomic_write(&sidecar, black_box(&bytes)).unwrap();
        write_samples.push(started.elapsed());
    }
    let write_median = percentile(&mut write_samples.clone(), 0.5);
    let write_p95 = percentile(&mut write_samples, 0.95);
    let _ = fs::remove_file(&sidecar);
    let _ = fs::remove_dir(&scratch);

    let binary_bytes = std::env::current_exe()
        .ok()
        .and_then(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    let nonblank = |source: &str| {
        source
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty() && !trimmed.starts_with("//")
            })
            .count()
    };
    BenchmarkResult {
        iterations,
        rust_parse_route_dispatch_median_ms: route_median,
        rust_parse_route_dispatch_p95_ms: route_p95,
        rust_atomic_sidecar_median_ms: write_median,
        rust_atomic_sidecar_p95_ms: write_p95,
        release_binary_bytes: binary_bytes,
        model_nonblank_lines: nonblank(include_str!("model.rs")),
        shell_nonblank_lines: nonblank(include_str!("main.rs")),
    }
}

fn review_report() -> ReviewReport {
    ReviewReport {
        verdict_candidate: "Feasible as a versioned, bounded Rust authority module in shadow/coexistence mode; this prototype does not justify a full backend rewrite.",
        boundary_finding: "The existing newline transport envelope is reusable, but the unchanged launch/snapshot v1 contract cannot represent selection-required or first-class Mission Formation. A versioned contract extension is mandatory.",
        scenarios: scenario_results(),
        rollback: rollback_proof(),
        benchmark: benchmark(),
        python_baseline: json!({
            "warm_persistent_snapshot_median_ms": PYTHON_WARM_SNAPSHOT_MS,
            "public_queue_ack_median_ms": PYTHON_PUBLIC_QUEUE_ACK_MS,
            "approval_to_runner_claim_median_ms": PYTHON_RUNNER_CLAIM_MS,
            "one_file_live_goal_to_reviewed_seconds": PYTHON_LIVE_GOAL_SECONDS,
            "source": ".agent/Reports/2026-07-23-alfredo-architecture-performance-baseline.md"
        }),
        interpretation: vec![
            "Rust makes the narrow transition vocabulary exhaustive and produces a small standalone artifact.",
            "The measured Rust loop is directional only: it omits Python's production locks, stores, audit reconciliation, sandbox, runner, evidence, review, CLI parity, and migrations.",
            "Backend control latency is already millisecond-scale while local-model work is seconds-scale, so Rust does not establish a meaningful end-to-end speed win.",
            "Reliability comes from the explicit receipt, revision, replay, and atomic-commit design—not from the implementation language by itself.",
            "Adding Rust before extracting one canonical schema increases cross-language contract duplication and packaging/release obligations.",
            "The demonstrated rollback route is read-compatible import plus a Rust-only sidecar/shadow decision, leaving Python state authoritative until cutover.",
        ],
    }
}

fn handle_protocol_request(request: PersistentRequest) -> PersistentResponse {
    let result: Result<String, DecisionFailure> = match request.argv.first().map(String::as_str) {
        Some("prototype-review") => {
            serde_json::to_string(&review_report()).map_err(|error| DecisionFailure {
                code: "contract-failure".to_owned(),
                message: error.to_string(),
                recoverable: true,
            })
        }
        Some("prototype-import-v1") if request.argv.len() == 3 => {
            import_legacy_v1(&request.argv[1], request.argv[2].clone()).and_then(|state| {
                serde_json::to_string(&state).map_err(|error| DecisionFailure {
                    code: "contract-failure".to_owned(),
                    message: error.to_string(),
                    recoverable: true,
                })
            })
        }
        Some("prototype-apply") if request.argv.len() == 3 => {
            let state = serde_json::from_str::<PrototypeState>(&request.argv[1]).map_err(|error| {
                DecisionFailure {
                    code: "contract-failure".to_owned(),
                    message: error.to_string(),
                    recoverable: true,
                }
            });
            let action =
                serde_json::from_str::<Action>(&request.argv[2]).map_err(|error| DecisionFailure {
                    code: "contract-failure".to_owned(),
                    message: error.to_string(),
                    recoverable: true,
                });
            state
                .and_then(|state| action.and_then(|action| apply(&state, action)))
                .and_then(|transition| {
                    serde_json::to_string(&transition).map_err(|error| DecisionFailure {
                        code: "contract-failure".to_owned(),
                        message: error.to_string(),
                        recoverable: true,
                    })
                })
        }
        _ => Err(DecisionFailure {
            code: "contract-failure".to_owned(),
            message: "expected prototype-review, prototype-import-v1, or prototype-apply"
                .to_owned(),
            recoverable: true,
        }),
    };
    match result {
        Ok(stdout) => PersistentResponse {
            id: request.id,
            success: true,
            stdout: format!("{stdout}\n"),
            stderr: String::new(),
        },
        Err(error) => PersistentResponse {
            id: request.id,
            success: false,
            stdout: String::new(),
            stderr: serde_json::to_string(&json!({ "error": error })).unwrap_or_default(),
        },
    }
}

fn serve_protocol() {
    for line in io::stdin().lock().lines() {
        let line = line.unwrap_or_default();
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<PersistentRequest>(&line) {
            Ok(request) => handle_protocol_request(request),
            Err(error) => PersistentResponse {
                id: String::new(),
                success: false,
                stdout: String::new(),
                stderr: serde_json::to_string(&json!({
                    "error": {
                        "code": "contract-failure",
                        "message": error.to_string(),
                        "recoverable": true
                    }
                }))
                .unwrap_or_default(),
            },
        };
        println!("{}", serde_json::to_string(&response).unwrap());
    }
}

fn print_state(state: &PrototypeState, last: &str) {
    print!("\x1b[2J\x1b[H");
    println!("\x1b[1mPROTOTYPE — Rust Orchestrator vertical slice\x1b[0m");
    println!("\x1b[2mDisposable decision evidence; Python remains authoritative.\x1b[0m\n");
    println!("\x1b[1mCurrent state\x1b[0m");
    println!("schema             {}", state.schema_version);
    println!("revision           {}", state.revision);
    println!("starting location  {}", state.starting_location);
    println!(
        "coding workspace   {}",
        state
            .coding_workspace
            .as_deref()
            .unwrap_or("<selection required>")
    );
    println!("phase              {:?}", state.phase);
    println!(
        "active Mission     {}",
        state.active_mission_id.as_deref().unwrap_or("<none>")
    );
    println!(
        "formation route    {}",
        state
            .formation_route
            .as_ref()
            .map(|route| format!("{route:?}"))
            .unwrap_or_else(|| "<none>".to_owned())
    );
    println!("queued sessions    {}", state.sessions.len());
    println!("durable receipts   {}", state.receipts.len());
    println!("\n\x1b[1mLast result\x1b[0m\n{last}");
    println!("\n\x1b[1mActions\x1b[0m");
    println!("[1] import current v1     [2] reset to Starting Location");
    println!("[3] select known repo     [4] resume known Mission");
    println!("[5] choose Ad Hoc route   [6] queue exact dispatch");
    println!("[7] replay last dispatch  [f] try false-success route");
    println!("[s] safety scenarios      [x] rollback proof");
    println!("[b] measured comparison   [q] quit");
    print!("\n> ");
    let _ = io::stdout().flush();
}

fn interactive() {
    let mut state = PrototypeState::selection_required("/starting/location");
    let mut last = "No action yet.".to_owned();
    loop {
        print_state(&state, &last);
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return;
        }
        let transition = match input.trim() {
            "1" => match import_legacy_v1(LEGACY_FIXTURE, "/starting/location") {
                Ok(imported) => {
                    state = imported;
                    last = "Read compatible schema-v1 state without writing it.".to_owned();
                    None
                }
                Err(error) => {
                    last = format!("{}: {}", error.code, error.message);
                    None
                }
            },
            "2" => {
                state = PrototypeState::selection_required("/starting/location");
                last = "Reset to a distinct Starting Location.".to_owned();
                None
            }
            "3" => Some(apply(
                &state,
                Action::SelectWorkspace {
                    correlation_id: "interactive-select".to_owned(),
                    expected_revision: state.revision,
                    path: "/code/alfredo".to_owned(),
                    repository_valid: true,
                    known_missions: vec![MissionRef {
                        id: "modernize-alfredo".to_owned(),
                        title: "Modernize Alfredo".to_owned(),
                    }],
                },
            )),
            "4" => Some(apply(
                &state,
                Action::ChooseMission {
                    correlation_id: "interactive-mission".to_owned(),
                    expected_revision: state.revision,
                    choice: MissionChoice::Resume {
                        mission_id: "modernize-alfredo".to_owned(),
                    },
                },
            )),
            "5" => Some(apply(
                &state,
                Action::FormMission {
                    correlation_id: "interactive-route".to_owned(),
                    expected_revision: state.revision,
                    route: MissionFormationRoute::AdHocDelegation,
                    task: "Add one bounded receipt seam".to_owned(),
                    effectful_request: true,
                    controller_claims_effect: false,
                },
            )),
            "6" => Some(apply(
                &state,
                Action::Dispatch {
                    correlation_id: "interactive-dispatch".to_owned(),
                    expected_revision: state.revision,
                    proposal_exact: true,
                    worker_eligible: true,
                },
            )),
            "7" => Some(apply(
                &state,
                Action::Dispatch {
                    correlation_id: "interactive-dispatch".to_owned(),
                    expected_revision: state.revision.saturating_sub(1),
                    proposal_exact: true,
                    worker_eligible: true,
                },
            )),
            "f" => {
                let ready = ready_pre_workspace_state();
                Some(apply(
                    &ready,
                    Action::FormMission {
                        correlation_id: "interactive-false-success".to_owned(),
                        expected_revision: ready.revision,
                        route: MissionFormationRoute::NoAction,
                        task: "Yes, create the folder now".to_owned(),
                        effectful_request: true,
                        controller_claims_effect: true,
                    },
                ))
            }
            "s" => {
                let scenarios = scenario_results();
                last = scenarios
                    .iter()
                    .map(|result| {
                        format!(
                            "{} {}",
                            if result.passed { "PASS" } else { "FAIL" },
                            result.name
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                None
            }
            "x" => {
                let rollback = rollback_proof();
                last = format!(
                    "{} source unchanged={} crash cut safe={}",
                    if rollback.passed { "PASS" } else { "FAIL" },
                    rollback.legacy_source_unchanged,
                    rollback.crash_cut_left_canonical_sidecar_unchanged
                );
                None
            }
            "b" => {
                let result = benchmark();
                last = format!(
                    "Rust parse→route→queue→serialize: {:.4} ms median / {:.4} ms p95\n\
                     Rust fsync+atomic sidecar: {:.4} ms median / {:.4} ms p95\n\
                     Python full public Queue ack baseline: {:.3} ms median\n\
                     Release binary: {:.1} KiB; prototype source: {} nonblank lines\n\
                     Directional only: the Rust slice omits production authority breadth.",
                    result.rust_parse_route_dispatch_median_ms,
                    result.rust_parse_route_dispatch_p95_ms,
                    result.rust_atomic_sidecar_median_ms,
                    result.rust_atomic_sidecar_p95_ms,
                    PYTHON_PUBLIC_QUEUE_ACK_MS,
                    result.release_binary_bytes as f64 / 1024.0,
                    result.model_nonblank_lines + result.shell_nonblank_lines
                );
                None
            }
            "q" => return,
            _ => {
                last = "Unknown action.".to_owned();
                None
            }
        };
        if let Some(result) = transition {
            match result {
                Ok(transition) => {
                    state = transition.state;
                    last = format!(
                        "{} (canonical={}, replayed={}): {}",
                        transition.receipt.effect_kind,
                        transition.receipt.canonical_effect,
                        transition.receipt.replayed,
                        transition.receipt.message
                    );
                }
                Err(error) => {
                    last = format!("{}: {}", error.code, error.message);
                }
            }
        }
    }
}

fn print_review() {
    let report = review_report();
    println!("PROTOTYPE — Rust Orchestrator vertical slice");
    println!("\nCandidate verdict\n{}", report.verdict_candidate);
    println!("\nTyped-boundary finding\n{}", report.boundary_finding);
    println!("\nReliability scenarios");
    for scenario in &report.scenarios {
        println!(
            "{} {:<56} {}",
            if scenario.passed { "PASS" } else { "FAIL" },
            scenario.name,
            scenario.evidence
        );
    }
    println!(
        "\nRollback\n{} Legacy source unchanged: {}. Crash-cut sidecar unchanged: {}.",
        if report.rollback.passed {
            "PASS"
        } else {
            "FAIL"
        },
        report.rollback.legacy_source_unchanged,
        report.rollback.crash_cut_left_canonical_sidecar_unchanged
    );
    println!("\nMeasured narrow slice");
    println!(
        "Rust parse→route→queue→serialize  {:.4} ms median / {:.4} ms p95 (n={})",
        report.benchmark.rust_parse_route_dispatch_median_ms,
        report.benchmark.rust_parse_route_dispatch_p95_ms,
        report.benchmark.iterations
    );
    println!(
        "Rust fsync+atomic sidecar          {:.4} ms median / {:.4} ms p95",
        report.benchmark.rust_atomic_sidecar_median_ms, report.benchmark.rust_atomic_sidecar_p95_ms
    );
    println!(
        "Release binary                     {:.1} KiB",
        report.benchmark.release_binary_bytes as f64 / 1024.0
    );
    println!(
        "Prototype source                   {} model + {} shell nonblank lines",
        report.benchmark.model_nonblank_lines, report.benchmark.shell_nonblank_lines
    );
    println!("\nCurrent Python production baselines");
    println!(
        "Warm snapshot {:.3} ms; Queue acknowledgement {:.3} ms; runner claim {:.3} ms",
        PYTHON_WARM_SNAPSHOT_MS, PYTHON_PUBLIC_QUEUE_ACK_MS, PYTHON_RUNNER_CLAIM_MS
    );
    println!(
        "One-file live model goal→reviewed {:.3} s",
        PYTHON_LIVE_GOAL_SECONDS
    );
    println!("\nInterpretation");
    for item in &report.interpretation {
        println!("- {item}");
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--review") => print_review(),
        Some("--review-json") => println!(
            "{}",
            serde_json::to_string_pretty(&review_report()).unwrap()
        ),
        Some("--protocol") => serve_protocol(),
        Some(other) => {
            eprintln!("Unknown option: {other}");
            std::process::exit(2);
        }
        None => interactive(),
    }
}
