//! Real scope storage and async routing exercise the console save-before-write boundary.
use alfredo_tui::{
    command_intent::Intent,
    console_command::CommandState,
    conversations::{Autosave, ConversationStore},
    model::{App, Status},
    tasks::TaskStore,
    understanding::{Action, Request},
    wayfinder::{Completion, Router, Turn},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    tasks: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-wayfinder-command-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("workspace")).unwrap();
        let tasks =
            TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
        Self { root, tasks }
    }
    fn conversation_file(&self) -> PathBuf {
        fs::read_dir(self.tasks.conversation_directory().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("conversations-")
                    && path
                        .extension()
                        .is_some_and(|extension| extension == "json")
            })
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn turn(app: &mut App, session: usize, prompt: &str) -> (usize, Turn) {
    let user_message = app.sessions[session].messages.len();
    app.sessions[session].insert(prompt);
    let messages = app.sessions[session].begin().unwrap();
    (
        user_message,
        Turn {
            session,
            attempt: app.sessions[session].attempt,
            model: app.sessions[session].model.clone(),
            messages,
        },
    )
}
fn prepared(router: &mut Router, runtime: &tokio::runtime::Runtime) -> (Turn, Request) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            router.poll(runtime).is_empty(),
            "Expected an inert prepared scope request"
        );
        if let Some(ready) = router.prepared().into_iter().next() {
            return ready;
        }
        assert!(Instant::now() < deadline, "Wayfinder preparation timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
fn completion(router: &mut Router, runtime: &tokio::runtime::Runtime) -> Completion {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let mut results = router.poll(runtime);
        if !results.is_empty() {
            assert_eq!(results.len(), 1);
            return results.remove(0);
        }
        assert!(
            Instant::now() < deadline,
            "Wayfinder acknowledgment timed out"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn wayfinder_entry_draft_and_confirmation_wait_for_saved_origin_and_replay_exactly_after_restart() {
    let fixture = Fixture::new();
    let scope = fixture.tasks.understanding();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut router = Router::new(scope.clone());
    let mut app = App::new("fixture".into());
    app.add_session();
    app.sessions[1].insert("Unrelated conversation draft");
    let mut autosave = Autosave::new(ConversationStore::open(&fixture.tasks, "default").unwrap());
    let prompts = [
        "Build a new project for local calculations",
        "Destination: Reliable calculations\nScope: Local computation only\nConstraints: Keep compatibility\nUncertainty: Latency needs measurement",
        "confirm shared understanding 2",
    ];
    for (index, prompt) in prompts.iter().enumerate() {
        let (user_message, submitted) = turn(&mut app, 0, prompt);
        let before = scope.snapshot().unwrap();
        router
            .start(&runtime, submitted, Some(before.revision))
            .unwrap();
        let (origin, request) = prepared(&mut router, &runtime);
        assert_eq!(origin.session, 0);
        assert_eq!(scope.snapshot().unwrap(), before);
        assert!(matches!(
            (&request.action, index),
            (Action::Enter { .. }, 0)
                | (Action::Draft { .. }, 1)
                | (Action::Confirm { draft_revision: 2 }, 2)
        ));
        let id = app.sessions[0]
            .submit_wayfinder_command(user_message, request.clone())
            .unwrap();
        let entry = app.sessions[0].commands().last().unwrap();
        assert!(!autosave.contains_saved_command(0, entry));
        assert_eq!(scope.snapshot().unwrap(), before);
        autosave
            .finish(&runtime, &app, Default::default(), None)
            .unwrap();
        assert!(autosave.contains_saved_command(0, app.sessions[0].commands().last().unwrap()));
        assert!(!autosave.contains_saved_command(1, app.sessions[0].commands().last().unwrap()));
        router.dispatch_prepared(&runtime, 0, &request).unwrap();
        app.sessions[0].set_command_state(&id, CommandState::Submitted);
        let done = completion(&mut router, &runtime);
        assert_eq!(done.turn.session, 0);
        assert_eq!(done.request.as_ref(), Some(&request));
        let decision = done.result.unwrap();
        assert_eq!(decision.state.revision, index as u64 + 1);
        assert!(app.sessions[0]
            .commands()
            .last()
            .unwrap()
            .intent
            .reconcile(None, Some(&decision.state))
            .is_some());
        assert_eq!(decision.state.receipts.last().unwrap().request, request);
        app.sessions[0].wayfinder_reply(
            done.turn.attempt,
            decision.acknowledgment.unwrap(),
            decision.receipt,
        );
        assert_eq!(app.sessions[0].messages.len(), 2 * (index + 1));
        assert!(app.sessions[1].commands().is_empty());
    }
    let final_scope = scope.snapshot().unwrap();
    assert!(final_scope.confirmed);
    assert!(fixture.tasks.snapshot().unwrap().tasks.is_empty());
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    drop(autosave);
    let store = ConversationStore::open(&fixture.tasks, "default").unwrap();
    let mut restored = store.load().unwrap().unwrap().restore();
    assert_eq!(restored.selected, 1);
    assert_eq!(restored.sessions[1].draft, "Unrelated conversation draft");
    assert_eq!(restored.sessions[0].messages.len(), 6);
    let before_messages = restored.sessions[0].messages.clone();
    assert!(restored.sessions[0]
        .commands()
        .iter()
        .all(|command| matches!(command.state, CommandState::Unknown { .. })));
    let mut restarted = Router::new(scope.clone());
    assert!(!restarted.active());
    assert!(restarted.poll(&runtime).is_empty());
    assert_eq!(scope.snapshot().unwrap(), final_scope);
    let command = restored.sessions[0].commands().last().unwrap().clone();
    let Intent::Wayfinder {
        request,
        user_message,
    } = &command.intent
    else {
        unreachable!()
    };
    assert_eq!(*user_message, 4);
    restored.sessions[0].retry_command(&command.id).unwrap();
    let replay_turn = Turn {
        session: 0,
        attempt: restored.sessions[0].attempt,
        model: restored.sessions[0].model.clone(),
        messages: restored.sessions[0].messages[..*user_message + 1].to_vec(),
    };
    restarted.resume(replay_turn, request.clone()).unwrap();
    assert_eq!(scope.snapshot().unwrap(), final_scope);
    let mut autosave = Autosave::new(store);
    assert!(!autosave.contains_saved_command(0, restored.sessions[0].commands().last().unwrap()));
    autosave
        .finish(&runtime, &restored, Default::default(), None)
        .unwrap();
    assert!(autosave.contains_saved_command(0, restored.sessions[0].commands().last().unwrap()));
    restarted.dispatch_prepared(&runtime, 0, request).unwrap();
    let replayed = completion(&mut restarted, &runtime);
    assert_eq!(replayed.request.as_ref(), Some(request));
    let decision = replayed.result.unwrap();
    assert_eq!(decision.state, final_scope);
    assert!(decision.receipt.is_some());
    assert_eq!(restored.sessions[0].messages, before_messages);
    assert!(fixture.tasks.snapshot().unwrap().tasks.is_empty());
}

#[test]
fn failed_wayfinder_intent_save_and_withdrawal_cannot_write_scope() {
    let fixture = Fixture::new();
    let scope = fixture.tasks.understanding();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut app = App::new("fixture".into());
    let mut autosave = Autosave::new(ConversationStore::open(&fixture.tasks, "default").unwrap());
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    let path = fixture.conversation_file();
    let original = fs::read(&path).unwrap();
    let mut router = Router::new(scope.clone());
    let (index, submitted) = turn(&mut app, 0, "Build a new project for a calculator");
    router.start(&runtime, submitted, Some(0)).unwrap();
    let (_, request) = prepared(&mut router, &runtime);
    let id = app.sessions[0]
        .submit_wayfinder_command(index, request.clone())
        .unwrap();
    app.sessions[0].insert("Keep the newer draft");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(autosave
        .finish(&runtime, &app, Default::default(), None)
        .is_err());
    assert!(!autosave.contains_saved_command(0, app.sessions[0].commands().last().unwrap()));
    assert_eq!(scope.snapshot().unwrap().revision, 0);
    assert!(router.withdraw_prepared(0, &request));
    app.sessions[0].cancel();
    app.sessions[0].set_command_state(
        &id,
        CommandState::Refused {
            reason: "Intent save failed; no action dispatched".into(),
        },
    );
    assert!(!router.active());
    assert!(router.dispatch_prepared(&runtime, 0, &request).is_err());
    assert!(router.poll(&runtime).is_empty());
    assert_eq!(scope.snapshot().unwrap().revision, 0);
    assert_eq!(app.sessions[0].draft, "Keep the newer draft");
    fs::remove_dir(&path).unwrap();
    fs::write(&path, original).unwrap();
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    drop(autosave);
    let restored = ConversationStore::open(&fixture.tasks, "default")
        .unwrap()
        .load()
        .unwrap()
        .unwrap()
        .restore();
    assert!(matches!(restored.sessions[0].status, Status::Cancelled));
    assert!(matches!(
        restored.sessions[0].commands()[0].state,
        CommandState::Refused { .. }
    ));
    assert_eq!(scope.snapshot().unwrap().revision, 0);
}

#[test]
fn cancellation_after_wayfinder_write_preserves_canonical_receipt_without_reviving_reply() {
    let fixture = Fixture::new();
    let scope = fixture.tasks.understanding();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut app = App::new("fixture".into());
    let mut router = Router::new(scope.clone());
    let (index, submitted) = turn(&mut app, 0, "Build a new project for a calculator");
    router.start(&runtime, submitted, Some(0)).unwrap();
    let (_, request) = prepared(&mut router, &runtime);
    let id = app.sessions[0]
        .submit_wayfinder_command(index, request.clone())
        .unwrap();
    let mut autosave = Autosave::new(ConversationStore::open(&fixture.tasks, "default").unwrap());
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    assert!(autosave.contains_saved_command(0, app.sessions[0].commands().last().unwrap()));
    router.dispatch_prepared(&runtime, 0, &request).unwrap();
    app.sessions[0].set_command_state(&id, CommandState::Submitted);
    // Wait for the durable write itself. A read that finds the scope lock busy
    // (the writer holds it) is "not yet", not a failure.
    let deadline = Instant::now() + Duration::from_secs(60);
    while scope.snapshot().map_or(true, |state| state.revision == 0) {
        assert!(Instant::now() < deadline, "Wayfinder write timed out");
        thread::sleep(Duration::from_millis(5));
    }
    app.sessions[0].cancel();
    let messages = app.sessions[0].messages.clone();
    let done = completion(&mut router, &runtime);
    assert_eq!(done.request.as_ref(), Some(&request));
    let decision = done.result.unwrap();
    app.sessions[0].wayfinder_reply(
        done.turn.attempt,
        decision.acknowledgment.unwrap(),
        decision.receipt,
    );
    assert!(matches!(app.sessions[0].status, Status::Cancelled));
    assert_eq!(app.sessions[0].messages, messages);
    assert!(app.sessions[0].commands()[0]
        .intent
        .reconcile(None, Some(&decision.state))
        .is_some());
    autosave
        .finish(&runtime, &app, Default::default(), None)
        .unwrap();
    drop(autosave);
    let restored = ConversationStore::open(&fixture.tasks, "default")
        .unwrap()
        .load()
        .unwrap()
        .unwrap()
        .restore();
    assert!(matches!(restored.sessions[0].status, Status::Cancelled));
    assert!(restored.sessions[0].commands()[0]
        .intent
        .reconcile(None, Some(&scope.snapshot().unwrap()))
        .is_some());
    assert_eq!(scope.snapshot().unwrap().revision, 1);
    assert!(fixture.tasks.snapshot().unwrap().tasks.is_empty());
}
