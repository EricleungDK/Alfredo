//! Selection provenance stays in its own conversation domain, outside model/task authority.
use alfredo_tui::{
    command_intent::Intent,
    console_command::CommandState,
    conversations::{Autosave, ConversationStore, Snapshot},
    model::{App, Update},
    provider::Ollama,
    selection_command::{Choice, MissionChoice, Origin, Outcome, Phase, Request, WorkspaceChoice},
    task_control::TaskControl,
    tasks::TaskStore,
    workstation::Workstation,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    source: TaskStore,
    target: TaskStore,
    request: Request,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-selection-command-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("source")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        let source_path = root.join("source").canonicalize().unwrap();
        let target_path = root.join("target").canonicalize().unwrap();
        let source = TaskStore::new(&root.join("state"), &source_path, "source-mission").unwrap();
        let target = TaskStore::new(&root.join("state"), &target_path, "target-mission").unwrap();
        let request = Request {
            correlation: "selection-proof".into(),
            origin: Origin::Conversation {
                workspace: source_path,
                mission: "source-mission".into(),
                conversation: "source-chat".into(),
                session: 0,
            },
            choice: Choice {
                workspace: WorkspaceChoice::Existing { path: target_path },
                mission: MissionChoice::Resume {
                    name: "target-mission".into(),
                },
            },
            conversation: "target-chat".into(),
        };
        Self {
            root,
            source,
            target,
            request,
        }
    }

    fn open_workstation(&self) -> Workstation {
        for name in ["source", "target"] {
            assert!(std::process::Command::new("git")
                .args(["init", "-q", "--template="])
                .arg(self.root.join(name))
                .status()
                .unwrap()
                .success());
        }
        self.source.select_mission(true).unwrap();
        self.target.select_mission(true).unwrap();
        Workstation::open(
            &self.root.join("state"),
            &self.root.join("source"),
            "source-mission",
            "source-chat",
            "fixture",
            Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn selection_origin_and_arrival_persist_separately_and_never_grant_task_authority() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut source = App::new("source-model".into());
    source.sessions[0].insert("Source draft");
    let source_id = source.sessions[0]
        .submit_selection_command(
            "Select target workspace and mission".into(),
            fixture.request.clone(),
        )
        .unwrap();
    let mut source_save =
        Autosave::new(ConversationStore::open(&fixture.source, "source-chat").unwrap());
    assert!(!source_save.contains_saved_command(0, &source.sessions[0].commands()[0]));
    source_save
        .finish(&runtime, &source, Default::default(), None)
        .unwrap();
    assert!(source_save.contains_saved_command(0, &source.sessions[0].commands()[0]));
    source.sessions[0].set_command_state(
        &source_id,
        CommandState::Selection {
            outcome: Outcome::at(Phase::HandoffPrepared),
        },
    );
    source_save
        .finish(&runtime, &source, Default::default(), None)
        .unwrap();
    let mut target = App::new("target-model".into());
    target.add_session();
    target.sessions[1].insert("Prior destination discussion");
    target.sessions[1].begin().unwrap();
    target.sessions[1].apply(1, Update::Token("Destination history\n".repeat(20)));
    target.sessions[1].apply(1, Update::Done);
    target.sessions[1].insert("Destination unfinished draft");
    target.sessions[1].left();
    target.sessions[1].scroll.set(10);
    target.sessions[1].reading_position(&[1; 25], 5);
    let before = serde_json::to_value(&target.sessions[1]).unwrap();
    let arrival_id = target.sessions[1]
        .submit_selection_arrival(fixture.request.clone(), Outcome::at(Phase::HandoffPrepared))
        .unwrap();
    let after = serde_json::to_value(&target.sessions[1]).unwrap();
    for field in [
        "messages", "draft", "cursor", "reading", "scroll", "status", "attempt",
    ] {
        assert_eq!(before[field], after[field], "Arrival changed {field}");
    }
    assert_ne!(arrival_id, source_id);
    assert_eq!(
        target.sessions[1].commands()[0].intent.selection_request(),
        source.sessions[0].commands()[0].intent.selection_request()
    );
    let target_store = ConversationStore::open(&fixture.target, "target-chat").unwrap();
    target_store
        .save(&Snapshot::capture(&target, "target-chat"))
        .unwrap();
    target.sessions[1].set_command_state(
        &arrival_id,
        CommandState::Selection {
            outcome: Outcome::at(Phase::Selected),
        },
    );
    target_store
        .save(&Snapshot::capture(&target, "target-chat"))
        .unwrap();
    let mut restored = target_store.load().unwrap().unwrap().restore();
    assert_eq!(restored.selected, 1);
    assert_eq!(restored.sessions[1].messages, target.sessions[1].messages);
    assert_eq!(restored.sessions[1].draft, "Destination unfinished draft");
    assert!(restored.sessions[1].retry_command(&arrival_id).is_err());
    let mut tasks = TaskControl::new(fixture.source.clone());
    let task_state = fixture.source.snapshot().unwrap();
    let scope_state = fixture.source.understanding().snapshot().unwrap();
    for intent in [
        &source.sessions[0].commands()[0].intent,
        &target.sessions[1].commands()[0].intent,
    ] {
        assert!(tasks.dispatch_prepared(&runtime, intent).is_err());
        assert!(intent
            .reconcile(Some(&task_state), Some(&scope_state))
            .is_none());
        assert!(intent.task_receipts(Some(&task_state)).is_empty());
        assert!(intent.scope_request().is_none());
        assert!(intent.planner_request().is_none());
    }
    assert!(fixture.source.snapshot().unwrap().tasks.is_empty());
    assert!(fixture.target.snapshot().unwrap().tasks.is_empty());
    // Journal unavailability may replace a recorded local outcome with explicit uncertainty.
    restored.sessions[1].set_command_state(
        &arrival_id,
        CommandState::Unknown {
            reason: "Selection journal unavailable; inspect retained work".into(),
        },
    );
    target_store
        .save(&Snapshot::capture(&restored, "target-chat"))
        .unwrap();
    assert!(matches!(
        target_store.load().unwrap().unwrap().restore().sessions[1].commands()[0].state,
        CommandState::Unknown { .. }
    ));
}

#[test]
fn selection_arrival_rejects_wrong_domain_incomplete_phases_and_same_target() {
    let fixture = Fixture::new();
    let source_store = ConversationStore::open(&fixture.source, "source-chat").unwrap();
    let target_store = ConversationStore::open(&fixture.target, "target-chat").unwrap();
    let mut target = App::new("fixture".into());
    for phase in [
        Phase::Admitted,
        Phase::RepositoryReady,
        Phase::MissionReady,
        Phase::TargetLoaded,
        Phase::AlreadyCurrent,
    ] {
        assert!(target.sessions[0]
            .submit_selection_arrival(fixture.request.clone(), Outcome::at(phase))
            .is_err());
        assert!(target.sessions[0].commands().is_empty());
    }
    target.sessions[0]
        .submit_selection_arrival(fixture.request.clone(), Outcome::at(Phase::HandoffPrepared))
        .unwrap();
    target_store
        .save(&Snapshot::capture(&target, "target-chat"))
        .unwrap();
    assert!(source_store
        .save(&Snapshot::capture(&target, "source-chat"))
        .is_err());
    let other_workspace = TaskStore::new(
        &fixture.root.join("state"),
        &fixture.root.join("source"),
        "target-mission",
    )
    .unwrap();
    let other_workspace = ConversationStore::open(&other_workspace, "target-chat").unwrap();
    assert!(other_workspace
        .save(&Snapshot::capture(&target, "target-chat"))
        .is_err());
    let other_mission = TaskStore::new(
        &fixture.root.join("state"),
        &fixture.root.join("target"),
        "other-mission",
    )
    .unwrap();
    let other_mission = ConversationStore::open(&other_mission, "target-chat").unwrap();
    assert!(other_mission
        .save(&Snapshot::capture(&target, "target-chat"))
        .is_err());
    let mut same = fixture.request.clone();
    same.origin = Origin::Conversation {
        workspace: same.choice.target(),
        mission: "target-mission".into(),
        conversation: "target-chat".into(),
        session: 0,
    };
    let mut same_app = App::new("fixture".into());
    same_app.sessions[0]
        .submit_selection_arrival(same, Outcome::at(Phase::HandoffPrepared))
        .unwrap();
    assert!(target_store
        .save(&Snapshot::capture(&same_app, "target-chat"))
        .is_err());
    let mut startup = fixture.request.clone();
    startup.origin = Origin::Startup;
    let mut launcher_target = App::new("fixture".into());
    assert!(launcher_target.sessions[0]
        .submit_selection_command("Startup has no source session".into(), startup.clone())
        .is_err());
    launcher_target.sessions[0]
        .submit_selection_arrival(startup, Outcome::at(Phase::HandoffPrepared))
        .unwrap();
    target_store
        .save(&Snapshot::capture(&launcher_target, "target-chat"))
        .unwrap();
    let intent = Intent::SelectionArrival {
        request: fixture.request.clone(),
    };
    let mut unsupported = Snapshot::capture(&target, "target-chat");
    unsupported.schema_version = 16;
    assert!(target_store.save(&unsupported).is_err());
    assert!(intent.validate().is_ok());
}

#[test]
fn target_command_capacity_refuses_handoff_and_retains_source_owner() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut work = fixture.open_workstation();
    work.app.sessions[0].insert("Latest source draft must remain");
    let mut target = App::new("destination-model".into());
    target.sessions[0].insert("Destination draft at full command capacity");
    let mut full = false;
    for number in 0..=512 {
        let command = Intent::Task {
            request: alfredo_tui::tasks::Request {
                correlation: format!("retained-{number}"),
                expected_revision: 0,
                action: alfredo_tui::tasks::Action::Approve { task: 1 },
            },
        };
        if target.sessions[0]
            .submit_command("/approve 1".into(), command)
            .is_err()
        {
            full = true;
            break;
        }
    }
    assert!(
        full,
        "Fixture must reach the actual command admission budget"
    );
    let target_snapshot = Snapshot::capture(&target, "target-chat");
    {
        let owner = ConversationStore::open(&fixture.target, "target-chat").unwrap();
        owner.save(&target_snapshot).unwrap();
    }
    let error = work.select(&runtime, fixture.request.clone()).unwrap_err();
    assert!(error.contains("capacity"), "{error}");
    assert_eq!(
        work.workspace(),
        fixture.root.join("source").canonicalize().unwrap()
    );
    assert_eq!(work.mission(), "source-mission");
    assert_eq!(
        work.app.sessions[0].draft,
        "Latest source draft must remain"
    );
    assert!(ConversationStore::open(&fixture.source, "source-chat").is_err());
    let target_owner = ConversationStore::open(&fixture.target, "target-chat").unwrap();
    assert_eq!(target_owner.load().unwrap().unwrap(), target_snapshot);
    let records = alfredo_tui::selection_store::Store::new(&fixture.root.join("state"))
        .unwrap()
        .snapshot()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].request, fixture.request);
    assert_eq!(records[0].outcome.phase, Phase::TargetLoaded);
    assert!(records[0]
        .outcome
        .failure
        .as_deref()
        .unwrap()
        .contains("capacity"));
    assert!(matches!(&work.app.sessions[0].commands()[0].state,
        CommandState::Selection { outcome } if outcome == &records[0].outcome));
    assert!(fixture.source.snapshot().unwrap().tasks.is_empty());
    assert!(fixture.target.snapshot().unwrap().tasks.is_empty());
}

#[test]
fn handoff_keeps_captured_origin_session_and_destination_drafts_reading() {
    let fixture = Fixture::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut work = fixture.open_workstation();
    work.app.sessions[0].insert("Captured origin draft");
    work.app.add_session();
    work.app.sessions[1].insert("Other selected source session");
    assert_eq!(work.app.selected, 1);
    let mut target = App::new("first-destination-model".into());
    target.sessions[0].insert("First destination draft");
    target.add_session();
    target.sessions[1].model = "selected-destination-model".into();
    target.sessions[1].insert("Earlier target question");
    target.sessions[1].begin().unwrap();
    target.sessions[1].apply(1, Update::Token("Saved target answer\n".repeat(20)));
    target.sessions[1].apply(1, Update::Done);
    target.sessions[1].insert("Selected destination unfinished 界 draft");
    target.sessions[1].left();
    target.sessions[1].scroll.set(10);
    target.sessions[1].reading_position(&[1; 25], 5);
    let before = serde_json::to_value(&target.sessions[1]).unwrap();
    {
        let owner = ConversationStore::open(&fixture.target, "target-chat").unwrap();
        owner
            .save(&Snapshot::capture(&target, "target-chat"))
            .unwrap();
    }
    assert!(work.select(&runtime, fixture.request.clone()).unwrap());
    assert_eq!(work.mission(), "target-mission");
    assert_eq!(work.conversation(), "target-chat");
    assert_eq!(work.app.selected, 1);
    assert_eq!(work.app.sessions[0].draft, "First destination draft");
    let after = serde_json::to_value(&work.app.sessions[1]).unwrap();
    for field in ["messages", "draft", "cursor", "reading", "scroll", "model"] {
        assert_eq!(
            before[field], after[field],
            "Handoff changed target {field}"
        );
    }
    assert!(work.app.sessions[0].commands().is_empty());
    let arrival = &work.app.sessions[1].commands()[0];
    assert!(
        matches!(&arrival.intent, Intent::SelectionArrival { request } if request == &fixture.request)
    );
    assert!(
        matches!(&arrival.state, CommandState::Selection { outcome } if outcome.phase == Phase::Selected && outcome.failure.is_none())
    );
    let source = ConversationStore::open(&fixture.source, "source-chat").unwrap();
    let saved = source.load().unwrap().unwrap();
    assert_eq!(saved.selected, 1);
    assert_eq!(saved.sessions[0].draft, "Captured origin draft");
    assert_eq!(saved.sessions[1].draft, "Other selected source session");
    assert!(saved.sessions[1].commands().is_empty());
    let origin = &saved.sessions[0].commands()[0];
    assert!(matches!(&origin.intent, Intent::Selection { request } if request == &fixture.request));
    assert_eq!(origin.state, arrival.state);
    assert!(ConversationStore::open(&fixture.target, "target-chat").is_err());
    assert!(fixture.source.snapshot().unwrap().tasks.is_empty());
    assert!(fixture.target.snapshot().unwrap().tasks.is_empty());
}
