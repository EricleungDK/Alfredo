//! Issue #233: tool exchanges are kept per reply, sent as context, compacted,
//! counted against the size limits, saved and shown in the transcript.
use alfredo_tui::{
    conversations::{ConversationStore, Snapshot},
    model::{compaction, App, Session, Status, ToolCall, ToolStep, Update, TOOL_CALLS_ROLE},
    tasks::TaskStore,
    ui,
};
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;

fn step(result: &str) -> ToolStep {
    ToolStep {
        calls: vec![ToolCall {
            name: "read_file".into(),
            arguments: json!({"path": "README.md"}),
        }],
        results: vec![result.into()],
    }
}

fn tool_turn(session: &mut Session, prompt: &str, result: &str, reply: &str) {
    session.insert(prompt);
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    session.apply(attempt, Update::ToolStep(step(result)));
    session.apply(attempt, Update::Token(reply.into()));
    session.apply(attempt, Update::Done);
}

#[test]
fn steps_are_recorded_and_sent_before_their_reply() {
    let mut session = Session::new("m".into());
    tool_turn(&mut session, "summarise README", "1\t# Demo", "A demo.");
    assert_eq!(session.tool_steps(1), &[step("1\t# Demo")]);
    session.insert("next");
    let request = session.begin().unwrap();
    let roles: Vec<_> = request.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        ["user", TOOL_CALLS_ROLE, "tool", "assistant", "user"]
    );
    assert_eq!(request[2].content, "1\t# Demo");
    let calls: Vec<ToolCall> = serde_json::from_str(&request[1].content).unwrap();
    assert_eq!(calls[0].name, "read_file");
}

#[test]
fn late_or_inactive_steps_are_ignored_and_retry_drops_them() {
    let mut session = Session::new("m".into());
    session.insert("p");
    session.begin().unwrap();
    session.apply(9, Update::ToolStep(step("late")));
    assert!(session.tool_steps(1).is_empty());
    session.apply(1, Update::ToolStep(step("kept")));
    session.apply(1, Update::Failed("boom".into()));
    session.apply(1, Update::ToolStep(step("after failure")));
    assert_eq!(session.tool_steps(1).len(), 1);
    let request = session.retry().unwrap();
    assert!(session.tool_steps(1).is_empty());
    assert!(request.iter().all(|m| m.role != "tool"));
}

#[test]
fn interrupted_turns_do_not_send_their_steps() {
    let mut session = Session::new("m".into());
    session.insert("p");
    session.begin().unwrap();
    session.apply(1, Update::ToolStep(step("SECRET_STEP")));
    session.cancel();
    session.insert("q");
    let request = session.begin().unwrap();
    assert!(request.iter().all(|m| !m.content.contains("SECRET_STEP")));
}

#[test]
fn tool_bytes_count_toward_the_conversation_limits() {
    let mut session = Session::new("m".into());
    let big = "z".repeat(40_000);
    tool_turn(&mut session, "a", &big, "r1");
    tool_turn(&mut session, "b", &big, "r2");
    session.insert("c");
    // 80 KB of tool output alone passes the 96 KiB budget with a small window.
    assert!(session.compaction_due(Some(32_768)));
    tool_turn(&mut session, "d", &big, "r3");
    session.insert(&"y".repeat(12_000));
    assert!(
        session.begin().is_err(),
        "120 KB of tool output plus a 12 KB prompt passes 128 KiB"
    );
}

#[test]
fn compaction_summarises_tool_steps_and_renumbers_the_kept_ones() {
    let mut session = Session::new("m".into());
    tool_turn(&mut session, "one", "OLD_TOOL_OUTPUT", "r1");
    tool_turn(&mut session, "two", "MID", "r2");
    tool_turn(&mut session, "three", "NEW_TOOL_OUTPUT", "r3");
    session.insert("four");
    let plan = session.start_compaction(None, true).unwrap();
    let asked = compaction::request(&plan, None)[1].content.clone();
    assert!(asked.contains("OLD_TOOL_OUTPUT"), "{asked}");
    assert!(asked.contains("read_file"), "{asked}");
    let attempt = session.attempt;
    session
        .finish_compaction(attempt, &plan, Ok("note".into()))
        .unwrap();
    assert_eq!(session.messages.len(), 4);
    assert_eq!(session.tool_steps(1)[0].results[0], "MID");
    assert_eq!(session.tool_steps(3)[0].results[0], "NEW_TOOL_OUTPUT");
}

#[test]
fn steps_survive_save_and_restore_and_bound_their_size() {
    let root = std::env::temp_dir().join(format!("alfredo-tool-steps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let tasks = TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
    let store = ConversationStore::open(&tasks, "default").unwrap();
    let mut app = App::new("m".into());
    tool_turn(
        &mut app.sessions[0],
        "summarise README",
        "RESULT",
        "A demo.",
    );
    let snapshot = Snapshot::capture(&app, "default");
    assert_eq!(snapshot.schema_version, 19);
    let text = serde_json::to_string(&snapshot).unwrap();

    let mut old: serde_json::Value = serde_json::from_str(&text).unwrap();
    old["schema_version"] = 18.into();
    let old: Snapshot = serde_json::from_value(old).unwrap();
    assert!(store.save(&old).is_err(), "v18 cannot hold tool steps");

    let mut bad: serde_json::Value = serde_json::from_str(&text).unwrap();
    bad["sessions"][0]["tool_steps"]["0"] = bad["sessions"][0]["tool_steps"]["1"].clone();
    let bad: Snapshot = serde_json::from_value(bad).unwrap();
    assert!(store.save(&bad).is_err(), "steps belong to replies");

    let mut mismatched: serde_json::Value = serde_json::from_str(&text).unwrap();
    mismatched["sessions"][0]["tool_steps"]["1"][0]["results"] = json!([]);
    let mismatched: Snapshot = serde_json::from_value(mismatched).unwrap();
    assert!(store.save(&mismatched).is_err(), "one result per call");

    store.save(&snapshot).unwrap();
    let restored = store.load().unwrap().unwrap().restore();
    assert_eq!(restored.sessions[0].tool_steps(1), &[step("RESULT")]);
    assert_eq!(restored.sessions[0].status, Status::Complete);
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn transcript_shows_each_tool_call_and_a_truncated_result() {
    let mut app = App::new("m".into());
    let long = format!("1\t# Demo\n{}", "w".repeat(500));
    tool_turn(
        &mut app.sessions[0],
        "summarise README",
        &long,
        "FINAL_ANSWER",
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("⚙ read_file README.md"), "{text}");
    assert!(text.contains("1 # Demo"), "{text}");
    assert!(text.contains("…"), "result is truncated");
    assert!(
        !text.contains(&"w".repeat(200)),
        "full result is not dumped"
    );
    assert!(text.contains("FINAL_ANSWER"));
}

fn screen_text(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn numbered(count: usize) -> String {
    (0..count)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn ctrl_o_toggles_full_tool_results_and_the_footer_says_which() {
    let mut app = App::new("m".into());
    tool_turn(&mut app.sessions[0], "show", &numbered(5), "FINAL");
    assert!(!app.sessions[0].tool_results_expanded, "starts collapsed");
    let collapsed = screen_text(&app, 120, 40);
    assert!(collapsed.contains("↳ line 0 … (5 lines)"), "{collapsed}");
    assert!(!collapsed.contains("line 3"), "{collapsed}");
    assert!(collapsed.contains("^O expand results"), "{collapsed}");

    app.sessions[0].toggle_tool_results();
    let full = screen_text(&app, 120, 40);
    for n in 0..5 {
        assert!(full.contains(&format!("line {n}")), "{full}");
    }
    assert!(!full.contains("(5 lines)"), "{full}");
    assert!(full.contains("^O collapse results"), "{full}");
    assert!(full.contains("FINAL"), "{full}");

    app.sessions[0].toggle_tool_results();
    assert_eq!(screen_text(&app, 120, 40), collapsed);
}

#[test]
fn full_tool_results_are_capped_at_200_lines() {
    let mut app = App::new("m".into());
    tool_turn(&mut app.sessions[0], "show", &numbered(250), "FINAL");
    app.sessions[0].toggle_tool_results();
    // Tall enough to hold the whole expanded transcript.
    let full = screen_text(&app, 120, 400);
    assert!(full.contains("line 199"), "{full}");
    assert!(!full.contains("line 200"), "{full}");
    assert!(full.contains("… 50 more lines"), "{full}");
    // Exactly 200 lines is shown whole, with no marker.
    let mut exact = App::new("m".into());
    tool_turn(&mut exact.sessions[0], "show", &numbered(200), "FINAL");
    exact.sessions[0].toggle_tool_results();
    let text = screen_text(&exact, 120, 400);
    assert!(
        text.contains("line 199") && !text.contains("more lines"),
        "{text}"
    );
}

#[test]
fn a_reloaded_conversation_shows_the_full_stored_result_when_expanded() {
    let root = std::env::temp_dir().join(format!("alfredo-tool-expand-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let tasks = TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
    let store = ConversationStore::open(&tasks, "default").unwrap();
    let mut app = App::new("m".into());
    tool_turn(&mut app.sessions[0], "show", &numbered(30), "FINAL");
    app.sessions[0].toggle_tool_results();
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    let restored = store.load().unwrap().unwrap().restore();
    assert!(
        !restored.sessions[0].tool_results_expanded,
        "the toggle is not persisted"
    );
    let collapsed = screen_text(&restored, 120, 60);
    assert!(collapsed.contains("(30 lines)"), "{collapsed}");
    let mut restored = restored;
    restored.sessions[0].toggle_tool_results();
    let full = screen_text(&restored, 120, 60);
    assert!(
        full.contains("line 0") && full.contains("line 29"),
        "{full}"
    );
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
