use alfredo_tui::{
    tasks::{Action as TaskAction, Request as TaskRequest, TaskStatus, TaskStore},
    understanding::{Action, Brief, Request},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    a: TaskStore,
    b: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-scope-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("workspace")).unwrap();
        Self {
            a: TaskStore::new(&root.join("state"), &root.join("workspace"), "a").unwrap(),
            b: TaskStore::new(&root.join("state"), &root.join("workspace"), "b").unwrap(),
            root,
        }
    }
    fn file(&self) -> PathBuf {
        fs::read_dir(self.root.join("state/rust-understanding-v1"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("understanding.json")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn brief() -> Brief {
    Brief {
        destination: "Requested result".into(),
        scope: "Chosen project scope".into(),
        constraints: "Keep compatibility".into(),
        uncertainty: "Performance remains unmeasured".into(),
    }
}
fn draft(id: &str, revision: u64) -> Request {
    Request {
        correlation: id.into(),
        expected_revision: revision,
        action: Action::Draft { brief: brief() },
    }
}
fn proposal(id: &str, revision: u64) -> TaskRequest {
    TaskRequest {
        correlation: id.into(),
        expected_revision: revision,
        action: TaskAction::Propose {
            title: "implement".into(),
            model: "fixture".into(),
            dependencies: vec![],
        },
    }
}
#[test]
fn gate_is_project_scoped_and_confirmation_never_approves_tasks() {
    let f = Fixture::new();
    let prior = proposal("prior", 0);
    f.a.transact(prior.clone()).unwrap();
    let scope = f.a.understanding();
    scope.transact(draft("draft", 0)).unwrap();
    assert!(f
        .a
        .transact(proposal("blocked", 1))
        .unwrap_err()
        .contains("Understanding pending"));
    assert!(f.b.transact(proposal("other-mission", 0)).is_err());
    assert!(f.a.transact(prior).is_ok()); // An exact acknowledged replay has no new effect.
    assert_eq!(f.b.understanding().snapshot().unwrap().brief, Some(brief()));
    assert!(scope
        .transact(Request {
            correlation: "bad-confirm".into(),
            expected_revision: 1,
            action: Action::Confirm { draft_revision: 0 }
        })
        .is_err());
    let confirmation = Request {
        correlation: "confirm".into(),
        expected_revision: 1,
        action: Action::Confirm { draft_revision: 1 },
    };
    let confirmed = scope.transact(confirmation.clone()).unwrap();
    assert!(confirmed.confirmed);
    assert_eq!(
        confirmed.receipts.last().unwrap().actor,
        "mission-commander"
    );
    assert_eq!(scope.transact(confirmation).unwrap(), confirmed);
    assert!(f.b.snapshot().unwrap().tasks.is_empty());
    f.b.transact(proposal("other-mission", 0)).unwrap();
    assert_eq!(
        f.b.snapshot().unwrap().tasks[0].status,
        TaskStatus::Proposed
    );
    scope.transact(draft("revised", 2)).unwrap();
    assert!(f.b.transact(proposal("blocked-again", 1)).is_err());
}
#[test]
fn stale_confirmation_and_fabricated_projection_never_open_gate() {
    let f = Fixture::new();
    let scope = f.a.understanding();
    scope.transact(draft("first", 0)).unwrap();
    scope.transact(draft("second", 1)).unwrap();
    let before = fs::read(f.file()).unwrap();
    assert!(scope
        .transact(Request {
            correlation: "stale".into(),
            expected_revision: 2,
            action: Action::Confirm { draft_revision: 1 }
        })
        .is_err());
    assert!(scope.transact(draft("first", 2)).is_err());
    assert_eq!(fs::read(f.file()).unwrap(), before);
    let mut forged: serde_json::Value = serde_json::from_slice(&before).unwrap();
    forged["confirmed"] = true.into();
    fs::write(f.file(), serde_json::to_vec(&forged).unwrap()).unwrap();
    assert!(scope.snapshot().is_err());
    assert!(f.b.transact(proposal("forged", 0)).is_err());
}
#[test]
fn pending_scope_prevents_owner_claim_but_allows_cancellation() {
    let f = Fixture::new();
    f.a.transact(proposal("task", 0)).unwrap();
    f.a.transact(TaskRequest {
        correlation: "approve".into(),
        expected_revision: 1,
        action: TaskAction::Approve { task: 1 },
    })
    .unwrap();
    f.b.understanding().transact(draft("pending", 0)).unwrap();
    assert!(f
        .a
        .claim_worker(1)
        .err()
        .unwrap()
        .to_string()
        .contains("Understanding pending"));
    f.a.transact(TaskRequest {
        correlation: "cancel".into(),
        expected_revision: 2,
        action: TaskAction::Cancel { task: 1 },
    })
    .unwrap();
    assert_eq!(
        f.a.snapshot().unwrap().tasks[0].status,
        TaskStatus::Cancelled
    );
}

#[test]
fn draft_admission_reserves_the_last_confirmation_slot() {
    let f = Fixture::new();
    let scope = f.a.understanding();
    for n in 0..127 {
        let revision = n * 2;
        scope
            .transact(draft(&format!("draft-{n}"), revision))
            .unwrap();
        scope
            .transact(Request {
                correlation: format!("confirm-{n}"),
                expected_revision: revision + 1,
                action: Action::Confirm {
                    draft_revision: revision + 1,
                },
            })
            .unwrap();
    }
    scope.transact(draft("last-draft", 254)).unwrap();
    let before = fs::read(f.file()).unwrap();
    assert!(scope.transact(draft("would-starve", 255)).is_err());
    assert_eq!(fs::read(f.file()).unwrap(), before);
    let final_state = scope
        .transact(Request {
            correlation: "last-confirmation".into(),
            expected_revision: 255,
            action: Action::Confirm {
                draft_revision: 255,
            },
        })
        .unwrap();
    assert!(final_state.confirmed);
    assert_eq!(final_state.revision, 256);
    f.b.transact(proposal("still-allowed", 0)).unwrap();
}

#[test]
fn task_view_observes_cross_mission_scope_and_never_resumes_dispatch_on_confirmation() {
    use alfredo_tui::{model::App, task_control::TaskControl, ui};
    use ratatui::{backend::TestBackend, Terminal};
    use std::time::{Duration, Instant};
    let f = Fixture::new();
    f.a.transact(proposal("existing", 0)).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut control = TaskControl::new(f.a.clone());
    let refresh = |control: &mut TaskControl| {
        control.refresh(&runtime);
        let deadline = Instant::now() + Duration::from_secs(5);
        while control.pending {
            control.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    refresh(&mut control);
    assert!(!control.scope_status.blocked);
    assert!(control.scope_status.label.contains("outside explicit flow"));
    control
        .command(&runtime, "/dispatch on", "fixture")
        .unwrap();
    f.b.understanding().transact(draft("external", 0)).unwrap();
    control.refresh_background(&runtime);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !control.scope_status.blocked {
        control.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!control.dispatch.enabled);
    assert!(control
        .command(&runtime, "/dispatch on", "fixture")
        .is_err());
    assert!(control
        .command(&runtime, "/run 1", "fixture")
        .unwrap_err()
        .contains("Scope pending"));
    assert!(control.dispatch.attempts.is_empty());
    control
        .command(&runtime, "/tasks scope pending", "fixture")
        .unwrap();
    assert_eq!(control.visible_tasks().len(), 1);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let app = App::new("fixture".into());
    terminal
        .draw(|frame| ui::draw_with_tasks(frame, &app, &control))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("Scope pending"));
    assert!(rendered.contains("/scope-confirm 1"));
    f.b.understanding()
        .transact(Request {
            correlation: "confirmed-elsewhere".into(),
            expected_revision: 1,
            action: Action::Confirm { draft_revision: 1 },
        })
        .unwrap();
    refresh(&mut control);
    assert!(!control.scope_status.blocked);
    assert!(!control.dispatch.enabled);
    assert!(control.visible_tasks().is_empty());
    assert_eq!(f.a.snapshot().unwrap().revision, 1);
    assert_eq!(
        f.a.snapshot().unwrap().tasks[0].status,
        TaskStatus::Proposed
    );
    fs::write(f.file(), b"corrupt").unwrap();
    refresh(&mut control);
    assert!(control.snapshot.is_some()); // Readable task history survives a broken scope journal.
    assert!(control.scope_status.blocked);
    assert!(control.scope_status.label.contains("unavailable"));
    assert!(control
        .command(&runtime, "/dispatch on", "fixture")
        .is_err());
}

#[test]
fn canonical_wayfinder_routes_first_contact_and_reuses_flow_across_missions() {
    use alfredo_tui::{
        understanding::Mode,
        wayfinder::{entry_mode, route},
    };
    for prompt in [
        "Build a new app for scheduling",
        "Migrate the architecture",
        "A cross-cutting change",
    ] {
        assert_eq!(entry_mode(prompt), Some(Mode::Chart), "{prompt}");
    }
    for prompt in [
        "Explain this architecture",
        "Can you review the migration?",
        "Please diagnose the new application",
        "Status of the new project",
        "How does platform-wide scheduling work?",
        "ordinary discussion",
    ] {
        assert_eq!(entry_mode(prompt), None, "{prompt}");
    }
    assert_eq!(
        entry_mode("Review Wayfinder ticket #42"),
        Some(Mode::WorkThrough)
    );
    let fixture = Fixture::new();
    let scope = fixture.a.understanding();
    let outside = route(&scope, "Explain the architecture", "read", Some(0)).unwrap();
    assert!(outside.acknowledgment.is_none());
    assert!(!fixture.file().exists());
    let entry = route(&scope, "Build a new project", "entry", Some(0)).unwrap();
    assert_eq!(entry.state.revision, 1);
    assert_eq!(entry.state.flow.as_ref().unwrap().mode, Mode::Chart);
    assert_eq!(entry.state.receipts[0].actor, "wayfinder-alfredo");
    assert!(entry.acknowledgment.unwrap().contains("Receipt: entry"));
    assert!(!entry.state.confirmed);
    assert!(fixture
        .b
        .transact(proposal("blocked", 0))
        .unwrap_err()
        .contains("Shared Understanding pending"));
    let bytes = fs::read(fixture.file()).unwrap();
    let continued = route(
        &fixture.b.understanding(),
        "Wayfinder ticket #42",
        "continue",
        Some(1),
    )
    .unwrap();
    assert_eq!(continued.state.flow.unwrap().mode, Mode::Chart);
    assert!(continued.acknowledgment.is_none());
    assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
    assert!(route(
        &scope,
        "confirm shared understanding 1",
        "premature",
        Some(1)
    )
    .is_err());
    let draft = route(&scope, "Destination: Working scheduler\nScope: Local tasks only\nConstraints: No external services\nUncertainty: Performance needs measurement", "brief", Some(1)).unwrap();
    assert_eq!(draft.state.revision, 2);
    assert_eq!(
        draft.state.brief.as_ref().unwrap().destination,
        "Working scheduler"
    );
    assert!(draft.acknowledgment.unwrap().contains("scope draft saved"));
    let bytes = fs::read(fixture.file()).unwrap();
    assert!(route(&scope, "confirm shared understanding 1", "stale", Some(2)).is_err());
    assert!(route(&scope, "confirm shared understanding", "ambiguous", Some(2)).is_err());
    assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
    let confirmed = route(&scope, "confirm shared understanding 2", "confirm", Some(2)).unwrap();
    assert!(confirmed.state.confirmed);
    assert!(confirmed.acknowledgment.unwrap().contains("turn complete"));
    assert!(fixture.a.snapshot().unwrap().tasks.is_empty());
    assert!(fixture.b.snapshot().unwrap().tasks.is_empty());
    let replay = route(&scope, "confirm shared understanding 2", "confirm", Some(3)).unwrap();
    assert_eq!(replay.state.revision, 3);
    let continued = route(&scope, "Explain the next step", "continue-open", Some(3)).unwrap();
    assert!(continued.acknowledgment.is_none());
    assert!(continued.state.confirmed);
    let mut forged: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.file()).unwrap()).unwrap();
    forged["flow"]["mode"] = "work-through".into();
    fs::write(fixture.file(), serde_json::to_vec(&forged).unwrap()).unwrap();
    assert!(scope.snapshot().unwrap_err().contains("projection differs"));
}

#[test]
fn concurrent_entry_creates_one_flow_and_work_through_survives_reload() {
    use alfredo_tui::{
        understanding::{Flow, Mode},
        wayfinder::route,
    };
    let fixture = Fixture::new();
    let workers: Vec<_> = (0..8)
        .map(|index| {
            let scope = fixture.a.understanding();
            std::thread::spawn(move || {
                scope
                    .enter(
                        format!("entry-{index}"),
                        Flow {
                            mode: Mode::WorkThrough,
                            prompt: "Wayfinder map #9".into(),
                        },
                    )
                    .unwrap()
                    .1
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .map(|job| usize::from(job.join().unwrap()))
            .sum::<usize>(),
        1
    );
    let reloaded = route(
        &fixture.b.understanding(),
        "Build a new project",
        "new-message",
        Some(1),
    )
    .unwrap();
    assert_eq!(reloaded.state.revision, 1);
    assert_eq!(
        reloaded.state.flow.as_ref().unwrap().mode,
        Mode::WorkThrough
    );
    assert!(reloaded.acknowledgment.is_none());
}

#[test]
fn understanding_v1_migration_backs_up_exact_bytes_and_refuses_conflicts() {
    let fixture = Fixture::new();
    let scope = fixture.a.understanding();
    scope.transact(draft("first", 0)).unwrap();
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.file()).unwrap()).unwrap();
    legacy["schema_version"] = 1.into();
    let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    fs::write(fixture.file(), &bytes).unwrap();
    assert_eq!(scope.snapshot().unwrap().schema_version, 1);
    let backup = fixture
        .file()
        .with_file_name("understanding-v1-backup.json");
    fs::write(&backup, b"unrelated backup").unwrap();
    assert!(scope
        .transact(draft("second", 1))
        .unwrap_err()
        .contains("backup conflict"));
    assert_eq!(fs::read(fixture.file()).unwrap(), bytes);
    assert_eq!(fs::read(&backup).unwrap(), b"unrelated backup");
    fs::remove_file(&backup).unwrap();
    let migrated = scope.transact(draft("second", 1)).unwrap();
    assert_eq!(migrated.schema_version, 2);
    assert!(migrated.flow.is_none());
    assert!(!migrated.confirmed);
    assert_eq!(fs::read(&backup).unwrap(), bytes);
    scope
        .transact(Request {
            correlation: "confirm".into(),
            expected_revision: 2,
            action: Action::Confirm { draft_revision: 2 },
        })
        .unwrap();
    assert_eq!(fs::read(&backup).unwrap(), bytes);
}

#[test]
fn replayed_confirmation_identifies_its_original_receipt_and_current_pending_scope() {
    use alfredo_tui::wayfinder::route;
    let fixture = Fixture::new();
    let scope = fixture.a.understanding();
    scope.transact(draft("first", 0)).unwrap();
    route(
        &scope,
        "confirm shared understanding 1",
        "confirm-first",
        Some(1),
    )
    .unwrap();
    scope.transact(draft("new-draft", 2)).unwrap();
    let before = fs::read(fixture.file()).unwrap();
    let replay = route(
        &scope,
        "confirm shared understanding 1",
        "confirm-first",
        Some(3),
    )
    .unwrap();
    assert!(!replay.state.confirmed);
    let text = replay.acknowledgment.unwrap();
    assert!(text.contains("Receipt: confirm-first"), "{text}");
    assert!(text.contains("pending"), "{text}");
    assert!(!text.contains("Shared Understanding confirmed"), "{text}");
    assert_eq!(fs::read(fixture.file()).unwrap(), before);
}

#[test]
fn explicit_wayfinder_capability_routes_entry_brief_and_exact_confirmation() {
    use alfredo_tui::{
        understanding::Mode,
        wayfinder::{entry_mode, route},
    };
    assert_eq!(entry_mode("inspect project scope"), None);
    assert_eq!(
        entry_mode("@wayfinder inspect project scope"),
        Some(Mode::Chart)
    );
    assert_eq!(
        entry_mode("@wayfinder review Wayfinder ticket #42"),
        Some(Mode::WorkThrough)
    );
    let fixture = Fixture::new();
    let scope = fixture.a.understanding();
    let prompt = "@wayfinder inspect project scope";
    let entered = route(&scope, prompt, "explicit-entry", Some(0)).unwrap();
    assert_eq!(entered.state.flow.unwrap().prompt, prompt);
    assert!(entered
        .acknowledgment
        .unwrap()
        .contains("Receipt: explicit-entry"));
    let original = fs::read(fixture.file()).unwrap();
    assert!(route(
        &scope,
        "@unknown confirm shared understanding 1",
        "unknown",
        Some(1)
    )
    .is_err());
    assert!(route(&scope, "@wayfinder", "empty", Some(1)).is_err());
    assert_eq!(fs::read(fixture.file()).unwrap(), original);
    let draft = route(&scope, "@wayfinder Destination: Working scheduler\nScope: Local tasks\nConstraints: No external services\nUncertainty: Latency", "explicit-brief", Some(1)).unwrap();
    assert_eq!(draft.state.revision, 2);
    assert!(!draft.state.confirmed);
    assert!(route(
        &scope,
        "@wayfinder confirm shared understanding 1",
        "stale",
        Some(2)
    )
    .is_err());
    let confirmed = route(
        &scope,
        "@wayfinder confirm shared understanding 2",
        "explicit-confirm",
        Some(2),
    )
    .unwrap();
    assert!(confirmed.state.confirmed);
    assert_eq!(confirmed.receipt.unwrap().revision, 3);
    assert!(fixture.a.snapshot().unwrap().tasks.is_empty());
    let original = fs::read(fixture.file()).unwrap();
    let continued = route(
        &scope,
        "@wayfinder discuss remaining uncertainty",
        "continue",
        Some(3),
    )
    .unwrap();
    assert!(continued.acknowledgment.is_none());
    assert_eq!(fs::read(fixture.file()).unwrap(), original);
}

#[test]
fn prepared_first_contact_race_acknowledges_only_the_exact_winner_and_replays_it() {
    use alfredo_tui::wayfinder::{dispatch, prepare, request_matches_prompt, Preparation};
    let fixture = Fixture::new();
    let scope = fixture.a.understanding();
    let prompt = "Build a new project";
    let take_request = |correlation| match prepare(&scope, prompt, correlation, Some(0)).unwrap() {
        Preparation::Request(request) => request,
        Preparation::Discussion(_) => panic!("Expected inert first-contact request"),
    };
    let first = take_request("first-prepared");
    let second = take_request("second-prepared");
    assert_eq!(scope.snapshot().unwrap().revision, 0);
    assert!(!fixture.file().exists());
    assert!(request_matches_prompt(&first, prompt));
    assert!(!request_matches_prompt(&first, "Build a new service"));
    let entered = dispatch(&scope, &first).unwrap();
    assert_eq!(entered.receipt.unwrap().correlation, first.correlation);
    let original = fs::read(fixture.file()).unwrap();
    let lost = dispatch(&scope, &second).unwrap();
    assert!(lost.receipt.is_none());
    assert!(lost.acknowledgment.is_none());
    assert_eq!(lost.state, entered.state);
    assert_eq!(fs::read(fixture.file()).unwrap(), original);
    let replay = dispatch(&scope, &first).unwrap();
    assert_eq!(replay.receipt.unwrap().correlation, first.correlation);
    assert_eq!(fs::read(fixture.file()).unwrap(), original);
    let mut conflicting = first.clone();
    let Action::Enter { flow } = &mut conflicting.action else {
        unreachable!()
    };
    flow.prompt = "Build another new project".into();
    assert!(dispatch(&scope, &conflicting)
        .err()
        .unwrap()
        .contains("correlation"));
}

#[test]
fn router_preparation_and_resume_are_inert_and_exact_withdrawal_prevents_dispatch() {
    use alfredo_tui::{
        model::Message,
        wayfinder::{Router, Turn},
    };
    use std::time::{Duration, Instant};
    let fixture = Fixture::new();
    let scope = fixture.a.understanding();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut router = Router::new(scope.clone());
    let turn = Turn {
        session: 1,
        attempt: 3,
        model: "fixture".into(),
        messages: vec![Message {
            role: "user".into(),
            content: "Build a new project".into(),
        }],
    };
    router.start(&runtime, turn.clone(), Some(0)).unwrap();
    assert!(router.active());
    assert!(router.session_active(1));
    let deadline = Instant::now() + Duration::from_secs(5);
    let (prepared_turn, request) = loop {
        assert!(router.poll(&runtime).is_empty());
        if let Some(prepared) = router.prepared().pop() {
            break prepared;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(prepared_turn.session, 1);
    assert_eq!(prepared_turn.attempt, 3);
    assert_eq!(scope.snapshot().unwrap().revision, 0);
    assert!(!fixture.file().exists());
    assert!(router.request_pending(&request));
    let mut changed = request.clone();
    changed.correlation.push_str("-changed");
    assert!(!router.withdraw_prepared(1, &changed));
    assert!(!router.withdraw_prepared(0, &request));
    assert!(router.dispatch_prepared(&runtime, 1, &changed).is_err());
    assert!(router.withdraw_prepared(1, &request));
    assert!(!router.active());
    assert!(!router.request_pending(&request));
    assert!(router.dispatch_prepared(&runtime, 1, &request).is_err());
    assert_eq!(scope.snapshot().unwrap().revision, 0);
    let mut mismatched_turn = turn.clone();
    mismatched_turn.messages[0].content = "Unrelated discussion".into();
    assert!(router.resume(mismatched_turn, request.clone()).is_err());
    router.resume(turn, request.clone()).unwrap();
    assert_eq!(scope.snapshot().unwrap().revision, 0);
    router.dispatch_prepared(&runtime, 1, &request).unwrap();
    assert!(!router.withdraw_prepared(1, &request));
    assert!(router.request_pending(&request));
    let deadline = Instant::now() + Duration::from_secs(5);
    let completion = loop {
        if let Some(completion) = router.poll(&runtime).pop() {
            break completion;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(completion.turn.session, 1);
    assert_eq!(completion.turn.attempt, 3);
    assert_eq!(completion.request, Some(request.clone()));
    assert_eq!(
        completion.result.unwrap().receipt.unwrap().correlation,
        request.correlation
    );
    assert_eq!(scope.snapshot().unwrap().revision, 1);
    assert!(!router.active());
}
