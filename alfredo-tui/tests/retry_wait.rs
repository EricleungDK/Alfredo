//! Issue #584: the wait for a queued `--resume` retry, and how a refused retry is found.
use alfredo_tui::{
    autopilot::{retry_wait, RetryWait},
    console_command::{refused_reason, ConsoleCommand},
};
use serde_json::json;
use std::time::Duration;

#[test]
fn the_wait_holds_until_the_retry_is_refused_or_past_the_limit() {
    let limit = Duration::from_secs(60);
    assert_eq!(retry_wait(None, Duration::ZERO, limit), RetryWait::Hold);
    assert_eq!(retry_wait(None, limit, limit), RetryWait::Hold);
    assert_eq!(
        retry_wait(None, limit + Duration::from_millis(1), limit),
        RetryWait::TimedOut
    );
    assert_eq!(
        retry_wait(Some("no"), Duration::ZERO, limit),
        RetryWait::Refused("no".into())
    );
    assert_eq!(
        retry_wait(Some("no"), limit * 2, limit),
        RetryWait::Refused("no".into())
    );
}

fn command(id: &str, state: serde_json::Value) -> ConsoleCommand {
    serde_json::from_value(json!({
        "id": id, "sequence": 1, "after_messages": 0, "attempt": 0,
        "text": "/repair 1 retry",
        "intent": {"kind": "run", "correlation": "c", "expected_revision": 0, "task": 1},
        "state": state,
    }))
    .unwrap()
}

#[test]
fn only_the_named_commands_refusal_is_reported() {
    let refused = |reason: &str| json!({"kind": "refused", "reason": reason});
    let commands = [
        command("a", refused("not a")),
        command("b", json!({"kind": "submitted"})),
        command("c", refused("not c")),
    ];
    assert_eq!(refused_reason(&commands, "c").as_deref(), Some("not c"));
    assert_eq!(refused_reason(&commands, "a").as_deref(), Some("not a"));
    // A command that was not refused has no refusal, whatever else was.
    assert_eq!(refused_reason(&commands, "b"), None);
    assert_eq!(refused_reason(&commands, "missing"), None);
    assert_eq!(refused_reason(&[], "a"), None);
}
