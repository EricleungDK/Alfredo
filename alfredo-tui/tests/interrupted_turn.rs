//! #148: a cancelled or failed chat turn must not trap the conversation.
use alfredo_tui::conversations::{ConversationStore, Snapshot};
use alfredo_tui::model::{App, Session, Status, Update, NO_TEXT};
use alfredo_tui::tasks::TaskStore;

fn roles(request: &[alfredo_tui::model::Message]) -> Vec<(&str, &str)> {
    request
        .iter()
        .map(|m| (m.role.as_str(), m.content.as_str()))
        .collect()
}

fn interrupted(how: &str) -> Session {
    let mut session = Session::new("test-model".into());
    session.insert("wrong question");
    session.begin().unwrap();
    session.apply(1, Update::Token("partial".into()));
    match how {
        "cancel" => session.cancel(),
        _ => session.apply(1, Update::Failed("connection lost".into())),
    }
    session
}

#[test]
fn new_prompt_after_cancel_starts_a_turn_and_keeps_the_cancelled_one_visible() {
    let mut session = interrupted("cancel");
    assert_eq!(session.status, Status::Cancelled);
    session.insert("right question");
    let request = session.begin().unwrap();
    assert_eq!(roles(&request), [("user", "right question")]);
    assert_eq!(session.status, Status::Connecting);
    assert_eq!(session.messages.len(), 4);
    assert_eq!(session.messages[0].content, "wrong question");
    assert_eq!(session.messages[1].content, "partial");
    assert!(session.interrupted_turn(0));
    assert!(!session.interrupted_turn(2));
    session.apply(2, Update::Token("answer".into()));
    session.apply(2, Update::Done);
    session.insert("follow up");
    let request = session.begin().unwrap();
    assert_eq!(
        roles(&request),
        [
            ("user", "right question"),
            ("assistant", "answer"),
            ("user", "follow up")
        ]
    );
}

#[test]
fn an_empty_reply_fails_the_turn_and_is_not_resent() {
    let mut session = Session::new("test-model".into());
    session.insert("hello there");
    session.begin().unwrap();
    session.apply(1, Update::Done);
    assert_eq!(session.status, Status::Failed(NO_TEXT.into()));
    assert_eq!(
        NO_TEXT,
        "Model returned no text; Ctrl+R retries or send again"
    );
    session.insert("second");
    let request = session.begin().unwrap();
    assert_eq!(roles(&request), [("user", "second")]);
}

#[test]
fn a_whitespace_only_reply_also_fails() {
    let mut session = Session::new("test-model".into());
    session.insert("look");
    session.begin().unwrap();
    session.apply(1, Update::Token(" \n".into()));
    session.apply(1, Update::Done);
    assert_eq!(session.status, Status::Failed(NO_TEXT.into()));
}

#[test]
fn an_empty_reply_shows_one_notice_and_no_blank_assistant_block() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("hello there");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Done);
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows: Vec<String> = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect();
    let screen = rows.join("\n");
    assert!(screen.contains("hello there"), "{screen}");
    assert!(!screen.contains("Assistant"), "{screen}");
    assert!(screen.contains(&format!("✗ {NO_TEXT}")), "{screen}");
    // Transcript row plus status line: the text appears in both.
    assert!(screen.matches(NO_TEXT).count() >= 2, "{screen}");
}

#[test]
fn tool_rounds_then_an_empty_final_reply_fail() {
    use alfredo_tui::model::{ToolCall, ToolStep};
    let mut session = Session::new("test-model".into());
    session.insert("look");
    session.begin().unwrap();
    session.apply(
        1,
        Update::ToolStep(ToolStep {
            calls: vec![ToolCall {
                name: "list_files".into(),
                arguments: serde_json::json!({}),
            }],
            results: vec!["a.txt".into()],
        }),
    );
    session.apply(1, Update::Done);
    assert_eq!(session.status, Status::Failed(NO_TEXT.into()));
}

#[test]
fn a_propose_plan_hand_off_stays_complete() {
    use alfredo_tui::model::{ToolCall, ToolStep};
    let mut session = Session::new("test-model".into());
    session.insert("plan it");
    session.begin().unwrap();
    session.apply(
        1,
        Update::ToolStep(ToolStep {
            calls: vec![ToolCall {
                name: "propose_plan".into(),
                arguments: serde_json::json!({"goal": "g", "mode": "plan", "reason": "r"}),
            }],
            results: vec!["proposed".into()],
        }),
    );
    session.apply(1, Update::Token("Proposed `/plan g`.".into()));
    session.apply(1, Update::Done);
    assert_eq!(session.status, Status::Complete);
}

#[test]
fn retry_after_an_empty_reply_resends_the_prompt() {
    let mut session = Session::new("test-model".into());
    session.insert("hello there");
    session.begin().unwrap();
    session.apply(1, Update::Done);
    let request = session.retry().unwrap();
    assert_eq!(roles(&request), [("user", "hello there")]);
    session.apply(2, Update::Token("hi".into()));
    session.apply(2, Update::Done);
    assert_eq!(session.status, Status::Complete);
}

#[test]
fn new_prompt_after_failure_does_not_resend_the_failed_turn() {
    let mut session = interrupted("fail");
    session.insert("another");
    let request = session.begin().unwrap();
    assert_eq!(roles(&request), [("user", "another")]);
    assert_eq!(session.messages[1].content, "partial");
}

#[test]
fn empty_prompt_after_interruption_is_refused_and_still_retryable() {
    let mut session = interrupted("cancel");
    assert_eq!(session.begin(), Err("Enter a prompt"));
    assert_eq!(session.status, Status::Cancelled);
    let request = session.retry().unwrap();
    assert_eq!(roles(&request), [("user", "wrong question")]);
    assert!(!session.interrupted_turn(0));
}

#[test]
fn a_started_new_turn_leaves_the_old_one_not_retryable() {
    let mut session = interrupted("cancel");
    session.insert("next");
    session.begin().unwrap();
    session.apply(2, Update::Token("answer".into()));
    session.apply(2, Update::Done);
    assert!(session.retry().is_err());
}

#[test]
fn interrupted_turn_exclusion_survives_restart() {
    let mut app = App::new("m".into());
    app.sessions[0] = interrupted("fail");
    app.sessions[0].insert("next");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(2, Update::Token("ok".into()));
    app.sessions[0].apply(2, Update::Done);
    let json = serde_json::to_string(&Snapshot::capture(&app, "default")).unwrap();
    let snapshot: Snapshot = serde_json::from_str(&json).unwrap();
    let mut restored = snapshot.restore();
    assert!(restored.sessions[0].interrupted_turn(0));
    restored.sessions[0].insert("third");
    let request = restored.sessions[0].begin().unwrap();
    assert_eq!(
        roles(&request),
        [("user", "next"), ("assistant", "ok"), ("user", "third")]
    );
}

#[test]
fn model_switch_after_cancel_or_failure_applies_to_the_next_turn() {
    for how in ["cancel", "fail"] {
        let mut app = App::new("original".into());
        app.receive_models(Ok(vec!["other".into()]));
        app.sessions[0] = interrupted(how);
        app.sessions[0].model = "original".into();
        app.select_model("other").unwrap();
        assert_eq!(app.sessions[0].model, "other");
        app.sessions[0].insert("next");
        app.sessions[0].begin().unwrap();
        assert_eq!(app.sessions[0].model, "other");
    }
}

#[test]
fn model_switch_is_still_refused_while_a_turn_is_active() {
    let mut app = App::new("original".into());
    app.receive_models(Ok(vec!["other".into()]));
    app.sessions[0].insert("q");
    app.sessions[0].begin().unwrap();
    assert!(app.select_model("other").is_err());
}

#[test]
fn request_through_omits_earlier_interrupted_turns_but_keeps_its_own_target() {
    let mut session = interrupted("cancel");
    session.insert("right");
    let request = session.begin().unwrap();
    assert_eq!(session.request_through(2), request);
    assert_eq!(
        roles(&session.request_through(0)),
        [("user", "wrong question")]
    );
}

/// Issue #271: results as the tool loop reports them, one call at a time.
fn ran(session: &mut Session, name: &str, path: &str, result: &str, new_round: bool) {
    session.apply(
        1,
        Update::ToolResult {
            call: alfredo_tui::model::ToolCall {
                name: name.into(),
                arguments: serde_json::json!({"path": path}),
            },
            result: result.into(),
            new_round,
        },
    );
}

#[test]
fn an_interrupted_turn_keeps_its_applied_changes_for_the_next_request() {
    let mut session = Session::new("m".into());
    session.insert("make files");
    session.begin().unwrap();
    ran(
        &mut session,
        "write_file",
        "a.txt",
        "wrote a.txt (3 bytes)",
        true,
    );
    ran(&mut session, "read_file", "r.txt", "READ_ONLY_BODY", false);
    ran(
        &mut session,
        "write_file",
        "denied.txt",
        "error: the owner denied this action",
        false,
    );
    ran(
        &mut session,
        "edit_file",
        "b.txt",
        "error: the file changed",
        false,
    );
    // Rounds join as calls finish; one round, four calls.
    assert_eq!(session.tool_steps(1).len(), 1);
    assert_eq!(session.tool_steps(1)[0].calls.len(), 4);
    session.cancel();
    session.insert("hello");
    let request = session.begin().unwrap();
    let all: String = request.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(request.first().unwrap().content, "make files");
    assert!(all.contains("write_file a.txt: wrote a.txt"), "{all}");
    for left_out in ["READ_ONLY_BODY", "denied.txt", "b.txt"] {
        assert!(!all.contains(left_out), "{left_out} leaked: {all}");
    }
    assert_eq!(request.last().unwrap().content, "hello");
    // The transcript keeps every executed call, applied or not.
    assert_eq!(session.tool_steps(1)[0].calls.len(), 4);
}

#[test]
fn an_interrupted_turn_with_only_reads_or_refusals_is_still_dropped() {
    let mut session = Session::new("m".into());
    session.insert("look around");
    session.begin().unwrap();
    ran(&mut session, "read_file", "r.txt", "body", true);
    ran(&mut session, "write_file", "d.txt", "error: denied", false);
    session.apply(1, Update::Failed("connection lost".into()));
    session.insert("hello");
    assert_eq!(roles(&session.begin().unwrap()), [("user", "hello")]);
}

#[test]
fn applied_steps_of_an_interrupted_turn_survive_a_restart() {
    let mut app = App::new("m".into());
    let session = &mut app.sessions[0];
    session.insert("make files");
    session.begin().unwrap();
    ran(session, "write_file", "a.txt", "wrote a.txt", true);
    session.cancel();
    session.insert("hello");
    session.begin().unwrap();
    session.apply(2, Update::Done);
    let (root, store) = store();
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    let mut restored = store.load().unwrap().unwrap().restore();
    assert_eq!(restored.sessions[0].tool_steps(1).len(), 1);
    restored.sessions[0].insert("again");
    let request = restored.sessions[0].begin().unwrap();
    assert!(request
        .iter()
        .any(|m| m.content.contains("write_file a.txt: wrote a.txt")));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cancel_then_model_switch_then_retry_uses_the_new_model() {
    let mut app = App::new("original".into());
    app.receive_models(Ok(vec!["other".into()]));
    app.sessions[0] = interrupted("cancel");
    app.sessions[0].model = "original".into();
    app.select_model("other").unwrap();
    let request = app.sessions[0].retry().unwrap();
    assert_eq!(roles(&request), [("user", "wrong question")]);
    assert_eq!(app.sessions[0].model, "other");
}

fn store() -> (std::path::PathBuf, ConversationStore) {
    static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "alfredo-interrupted-{}-{}",
        std::process::id(),
        ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let tasks = TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
    let store = ConversationStore::open(&tasks, "default").unwrap();
    (root, store)
}

fn snapshot_json() -> serde_json::Value {
    let mut app = App::new("m".into());
    app.sessions[0] = interrupted("fail");
    app.sessions[0].insert("next");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(2, Update::Token("answer".into()));
    app.sessions[0].apply(2, Update::Done);
    serde_json::to_value(Snapshot::capture(&app, "default")).unwrap()
}

#[test]
fn snapshot_without_the_interrupted_field_still_loads() {
    let mut json = snapshot_json();
    let session = &mut json["sessions"][0];
    assert!(session
        .as_object_mut()
        .unwrap()
        .remove("interrupted")
        .is_some());
    let snapshot: Snapshot = serde_json::from_value(json).unwrap();
    let (root, store) = store();
    store.save(&snapshot).unwrap();
    let mut restored = store.load().unwrap().unwrap().restore();
    restored.sessions[0].insert("third");
    // Without the record, an old snapshot sends the whole history, as before.
    assert_eq!(restored.sessions[0].begin().unwrap().len(), 5);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn snapshot_validation_rejects_invalid_interrupted_indices() {
    let (root, store) = store();
    for bad in [
        serde_json::json!([1]),
        serde_json::json!([2]),
        serde_json::json!([40]),
        serde_json::json!([0, 0]),
    ] {
        let mut json = snapshot_json();
        json["sessions"][0]["interrupted"] = bad.clone();
        let snapshot: Snapshot = serde_json::from_value(json).unwrap();
        assert!(store.save(&snapshot).is_err(), "accepted {bad}");
    }
    let snapshot: Snapshot = serde_json::from_value(snapshot_json()).unwrap();
    store.save(&snapshot).unwrap();
    let _ = std::fs::remove_dir_all(root);
}
