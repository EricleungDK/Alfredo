//! #148: a cancelled or failed chat turn must not trap the conversation.
use alfredo_tui::conversations::Snapshot;
use alfredo_tui::model::{App, Session, Status, Update};

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
