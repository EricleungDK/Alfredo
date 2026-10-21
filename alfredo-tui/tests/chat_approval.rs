//! Issue #235: mutating chat tools pause for owner Approval; denied actions never
//! touch disk; outcomes go to a bounded chat-approval log shown in F4 activity,
//! never to the task store.
mod ollama_fixture;

use alfredo_tui::{
    activity,
    chat_tools::{
        self,
        approval::{self, CardKind, Decision, Gate, Log, Outcome},
        Tier, TOOLS,
    },
    model::{App, Event, Message, Session, Status, ToolCall, Update},
    provider::Ollama,
    tasks::TaskStore,
    ui,
};
use ollama_fixture::{done, Reply};
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

static ID: AtomicU64 = AtomicU64::new(0);

const README: &str = "# Demo\nOld intro line.\nKeep this line.\n";

fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-approval-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), README).unwrap();
    root
}

fn tool_call(name: &str, arguments: serde_json::Value) -> Reply {
    Reply::Stream(
        vec![
            format!(
                "{}\n",
                json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":name,"arguments":arguments}}]},"done":false})
            )
            .into_bytes(),
            format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        name: name.into(),
        arguments,
    }
}

/// A gate answering every request with `decision` (None drops the reply, as Esc does).
fn gate(decision: Option<Decision>) -> (Gate, Arc<Mutex<Vec<approval::Card>>>) {
    gate_with(decision, None, || {})
}

/// As [`gate`], logging outcomes to `log` and running `before` ahead of the answer.
fn gate_with(
    decision: Option<Decision>,
    log: Option<Log>,
    before: impl Fn() + Send + 'static,
) -> (Gate, Arc<Mutex<Vec<approval::Card>>>) {
    let (sender, mut receiver) = mpsc::channel::<approval::Request>(4);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let cards = seen.clone();
    tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            cards.lock().unwrap().push(request.card.clone());
            before();
            if let Some(decision) = decision {
                let _ = request.reply.send(decision);
            }
        }
    });
    (Gate::new(sender, log), seen)
}

fn log_dir(tag: &str) -> PathBuf {
    workspace(&format!("log-{tag}"))
}

/// One turn: the model asks for `first`, then answers "done".
async fn turn(root: &Path, first: Reply, gate: Option<Gate>) -> (Session, Vec<serde_json::Value>) {
    let addr = ollama_fixture::reserve();
    let first = Mutex::new(Some(first));
    let fixture = ollama_fixture::serve(addr, move |_, index| {
        if index == 0 {
            first.lock().unwrap().take().unwrap()
        } else {
            done("done")
        }
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let mut session = Session::new("fixture".into());
    session.insert("redraft README");
    let messages = session.begin().unwrap();
    let (sender, mut receiver) = mpsc::channel::<Event>(256);
    chat_tools::run_turn(
        &provider,
        root,
        0,
        1,
        "fixture".into(),
        messages,
        sender,
        gate,
    )
    .await;
    while let Some(event) = receiver.recv().await {
        session.apply(event.attempt, event.update);
    }
    let bodies = fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path == "POST /api/chat")
        .map(|r| r.body.clone())
        .collect();
    (session, bodies)
}

fn redraft() -> Reply {
    tool_call(
        "edit_file",
        json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "New intro line."}),
    )
}

#[test]
fn mutating_tools_are_registered_with_the_mutating_tier() {
    for name in ["write_file", "edit_file"] {
        let tool = TOOLS.iter().find(|tool| tool.name == name).unwrap();
        assert_eq!(tool.tier, Tier::Mutating, "{name}");
    }
    let names: Vec<_> = chat_tools::definitions()
        .iter()
        .map(|d| d["function"]["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"edit_file".to_string()));
}

#[tokio::test]
async fn redraft_readme_edits_in_place_after_approval() {
    let root = workspace("approve");
    let (gate, seen) = gate(Some(Decision::Approve));
    let (session, bodies) = turn(&root, redraft(), Some(gate)).await;
    assert_eq!(session.status, Status::Complete);
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "# Demo\nNew intro line.\nKeep this line.\n"
    );
    let cards = seen.lock().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool, "edit_file");
    assert_eq!(cards[0].kind, CardKind::Edit);
    assert!(
        cards[0].preview.contains("-Old intro line."),
        "{}",
        cards[0].preview
    );
    assert!(
        cards[0].preview.contains("+New intro line."),
        "{}",
        cards[0].preview
    );
    assert!(cards[0].summary.contains("README.md"));
    let second = bodies[1]["messages"].as_array().unwrap();
    let result = second.last().unwrap();
    assert_eq!(result["role"], "tool");
    assert!(!result["content"].as_str().unwrap().starts_with("error"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_denied_action_never_touches_disk_and_the_model_gets_a_refusal() {
    let root = workspace("deny");
    let (gate, seen) = gate(Some(Decision::Deny));
    let (session, bodies) = turn(&root, redraft(), Some(gate)).await;
    assert_eq!(session.status, Status::Complete);
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let result = &session.tool_steps(1)[0].results[0];
    assert!(result.contains("denied"), "{result}");
    let second = bodies[1]["messages"].as_array().unwrap();
    assert!(second.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .contains("denied"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_dropped_decision_counts_as_denial() {
    let root = workspace("drop");
    let (gate, _) = gate(None);
    let (_, _) = turn(&root, redraft(), Some(gate)).await;
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn without_a_gate_mutating_calls_are_refused() {
    let root = workspace("nogate");
    let (session, _) = turn(&root, redraft(), None).await;
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
    assert!(session.tool_steps(1)[0].results[0].starts_with("error: "));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn invalid_edits_are_refused_before_asking() {
    for (tag, arguments) in [
        (
            "zero",
            json!({"path": "README.md", "old_text": "absent", "new_text": "x"}),
        ),
        (
            "ambiguous",
            json!({"path": "README.md", "old_text": "line", "new_text": "x"}),
        ),
        (
            "escape",
            json!({"path": "../outside.md", "old_text": "a", "new_text": "b"}),
        ),
        (
            "secret",
            json!({"path": ".env", "old_text": "a", "new_text": "b"}),
        ),
        (
            "identical",
            json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "Old intro line."}),
        ),
    ] {
        let root = workspace(tag);
        fs::write(root.join(".env"), "a\n").unwrap();
        let (gate, seen) = gate(Some(Decision::Approve));
        let (session, _) = turn(&root, tool_call("edit_file", arguments), Some(gate)).await;
        assert!(seen.lock().unwrap().is_empty(), "{tag} asked");
        assert!(
            session.tool_steps(1)[0].results[0].starts_with("error: "),
            "{tag}"
        );
        assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
        assert_eq!(fs::read_to_string(root.join(".env")).unwrap(), "a\n");
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn write_file_previews_creates_and_refuses_excluded_paths() {
    let root = workspace("write");
    let card = approval::card(
        &root,
        &call(
            "write_file",
            json!({"path": "docs/new.md", "content": "hello\n"}),
        ),
    );
    assert!(card.is_err(), "missing parent directory is refused");
    fs::create_dir_all(root.join("docs")).unwrap();
    let new = call(
        "write_file",
        json!({"path": "docs/new.md", "content": "hello\n"}),
    );
    let card = approval::card(&root, &new).unwrap();
    assert!(card.preview.contains("+hello"), "{}", card.preview);
    assert!(card.summary.contains("new file"), "{}", card.summary);
    assert!(!root.join("docs/new.md").exists(), "preview never writes");
    let result = chat_tools::dispatch(&root, &new);
    assert!(!result.starts_with("error"), "{result}");
    assert_eq!(
        fs::read_to_string(root.join("docs/new.md")).unwrap(),
        "hello\n"
    );
    let replace = call(
        "write_file",
        json!({"path": "README.md", "content": "# New\n"}),
    );
    let card = approval::card(&root, &replace).unwrap();
    assert!(card.preview.contains("-Old intro line."));
    assert!(card.preview.contains("+# New"));
    for path in [".env", ".git/config", "/etc/passwd", "a/../../x"] {
        let refused = call("write_file", json!({"path": path, "content": "x"}));
        assert!(approval::card(&root, &refused).is_err(), "{path}");
        assert!(chat_tools::dispatch(&root, &refused).starts_with("error"));
    }
    assert!(!root.join(".env").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_matches_lf_old_text_in_a_crlf_file_and_keeps_crlf() {
    let root = workspace("crlf-edit");
    fs::write(root.join("win.txt"), b"one\r\ntwo\r\nthree\r\n").unwrap();
    let edit = call(
        "edit_file",
        json!({"path": "win.txt", "old_text": "one\ntwo", "new_text": "uno\ndos\ndos-b"}),
    );
    let card = approval::card(&root, &edit).unwrap();
    assert!(!card.preview.contains("line endings"), "{}", card.preview);
    let result = chat_tools::dispatch(&root, &edit);
    assert!(!result.starts_with("error"), "{result}");
    assert_eq!(
        fs::read(root.join("win.txt")).unwrap(),
        b"uno\r\ndos\r\ndos-b\r\nthree\r\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_on_a_mixed_ending_file_matches_exact_bytes() {
    let root = workspace("mixed-edit");
    fs::write(root.join("m.txt"), b"a\r\nb\nc\r\n").unwrap();
    let lf = call(
        "edit_file",
        json!({"path": "m.txt", "old_text": "a\nb", "new_text": "x"}),
    );
    let result = chat_tools::dispatch(&root, &lf);
    assert!(result.contains("mixes CRLF and LF"), "{result}");
    let exact = call(
        "edit_file",
        json!({"path": "m.txt", "old_text": "b\nc", "new_text": "y\nz"}),
    );
    let result = chat_tools::dispatch(&root, &exact);
    assert!(!result.starts_with("error"), "{result}");
    assert_eq!(fs::read(root.join("m.txt")).unwrap(), b"a\r\ny\nz\r\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ending_only_writes_show_an_explicit_card_line_and_keep_the_fingerprint() {
    let root = workspace("ending-card");
    fs::write(root.join("win.txt"), b"one\r\ntwo\r\nthree\r\n").unwrap();
    let lf = call(
        "write_file",
        json!({"path": "win.txt", "content": "one\ntwo\nthree\n"}),
    );
    let card = approval::card(&root, &lf).unwrap();
    assert!(
        card.preview.contains("line endings CRLF \u{2192} LF"),
        "{}",
        card.preview
    );
    assert!(
        !card.preview.contains("no line changes"),
        "{}",
        card.preview
    );
    let bare = call(
        "write_file",
        json!({"path": "win.txt", "content": "one\r\ntwo\r\nthree"}),
    );
    let card = approval::card(&root, &bare).unwrap();
    assert!(
        card.preview.contains("final newline removed"),
        "{}",
        card.preview
    );
    let result = chat_tools::dispatch(&root, &lf);
    assert!(!result.starts_with("error"), "{result}");
    assert_eq!(
        fs::read(root.join("win.txt")).unwrap(),
        b"one\ntwo\nthree\n"
    );
    // Identical content still reports no change.
    let same = approval::card(&root, &lf).unwrap();
    assert!(same.preview.contains("no line changes"), "{}", same.preview);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_refuses_identical_new_text_with_clear_error() {
    let root = workspace("edit-identical");
    fs::write(root.join("f.txt"), "a\nb\n").unwrap();
    let edit = call(
        "edit_file",
        json!({"path": "f.txt", "old_text": "b", "new_text": "b"}),
    );
    let card = approval::card(&root, &edit);
    assert_eq!(
        card.unwrap_err(),
        "new_text is identical to old_text; nothing to change"
    );
    let result = chat_tools::dispatch(&root, &edit);
    assert_eq!(
        result,
        "error: new_text is identical to old_text; nothing to change"
    );
    assert_eq!(fs::read_to_string(root.join("f.txt")).unwrap(), "a\nb\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_refuses_line_ending_only_change_in_crlf_file() {
    let root = workspace("edit-crlf-noop");
    fs::write(root.join("win.txt"), b"a\r\nb\r\n").unwrap();
    let edit = call(
        "edit_file",
        json!({"path": "win.txt", "old_text": "b\r\n", "new_text": "b\n"}),
    );
    let card = approval::card(&root, &edit);
    assert_eq!(
        card.unwrap_err(),
        "edit_file keeps the file's CRLF endings; use write_file to change them"
    );
    let result = chat_tools::dispatch(&root, &edit);
    assert_eq!(
        result,
        "error: edit_file keeps the file's CRLF endings; use write_file to change them"
    );
    assert_eq!(fs::read(root.join("win.txt")).unwrap(), b"a\r\nb\r\n");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn outcomes_are_logged_after_execution_and_shown_in_activity() {
    let dir = log_dir("outcomes");
    let log = Log::new(&dir);
    let root = workspace("logged");
    let (gate, _) = gate_with(Some(Decision::Deny), Some(log.clone()), || {});
    turn(&root, redraft(), Some(gate)).await;
    // Approved, but the file changed after the preview: logged as failed, not applied.
    let readme = root.join("README.md");
    let (gate, _) = gate_with(Some(Decision::Approve), Some(log.clone()), move || {
        fs::write(&readme, format!("{README}extra\n")).unwrap();
    });
    turn(&root, redraft(), Some(gate)).await;
    let (gate, _) = gate_with(Some(Decision::Approve), Some(log.clone()), || {});
    turn(&root, redraft(), Some(gate)).await;
    let records = log.records();
    let outcomes: Vec<_> = records.iter().map(|r| r.outcome.clone()).collect();
    assert!(matches!(outcomes[0], Outcome::Denied), "{outcomes:?}");
    assert!(matches!(outcomes[1], Outcome::Failed(_)), "{outcomes:?}");
    assert!(matches!(outcomes[2], Outcome::Applied), "{outcomes:?}");
    assert_eq!(records[0].tool, "edit_file");
    assert!(records[0].target.contains("README.md"));
    let entries = activity::chat_entries(&records, "");
    assert!(
        entries[0].summary.contains("applied"),
        "{}",
        entries[0].summary
    );
    assert!(entries[1].summary.contains("failed"));
    assert!(entries[2].summary.contains("denied"));
    assert!(activity::chat_entries(&records, "denied").len() == 1);
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_chat_log_is_bounded_and_never_touches_the_task_store() {
    let base = workspace("bounded");
    fs::create_dir_all(base.join("ws")).unwrap();
    let store = TaskStore::new(&base.join("state"), &base.join("ws"), "approval").unwrap();
    let before = store.snapshot().unwrap();
    let log = Log::for_store(&store).unwrap();
    let card = approval::card(
        &base,
        &call(
            "edit_file",
            json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "N"}),
        ),
    )
    .unwrap();
    for _ in 0..(approval::MAX_LOG * 2 + 10) {
        log.append(&card, &Outcome::Denied);
    }
    let records = log.records();
    assert!(records.len() <= approval::MAX_LOG * 2, "{}", records.len());
    assert!(records.len() >= approval::MAX_LOG);
    let after = store.snapshot().unwrap();
    assert_eq!(after.revision, before.revision);
    assert!(after.receipts.is_empty());
    assert_eq!(after.schema_version, alfredo_tui::tasks::SCHEMA_VERSION);
    assert_eq!(
        alfredo_tui::tasks::SCHEMA_VERSION,
        17,
        "no task schema bump"
    );
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn a_file_changed_after_the_preview_is_refused_at_approval() {
    for (tag, first) in [
        ("toctou-edit", redraft()),
        (
            "toctou-write",
            tool_call(
                "write_file",
                json!({"path": "README.md", "content": "# Replaced\n"}),
            ),
        ),
    ] {
        let root = workspace(tag);
        let target = root.join("README.md");
        let changed = "# Demo\nOld intro line.\nKeep this line.\nOwner added this.\n";
        let (gate, _) = gate_with(Some(Decision::Approve), None, move || {
            fs::write(&target, changed).unwrap();
        });
        let (session, _) = turn(&root, first, Some(gate)).await;
        let result = &session.tool_steps(1)[0].results[0];
        assert!(result.contains("changed since"), "{tag}: {result}");
        assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), changed);
        let _ = fs::remove_dir_all(root);
    }
}

fn pending_session() -> Session {
    let root = workspace("card");
    let card = approval::card(
        &root,
        &call(
            "edit_file",
            json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "New intro line."}),
        ),
    )
    .unwrap();
    let _ = fs::remove_dir_all(root);
    let mut session = Session::new("m".into());
    session.insert("redraft README");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    let now = Instant::now();
    assert!(
        !session.await_approval(attempt + 1, card.clone(), now),
        "stale"
    );
    assert!(session.await_approval(attempt, card, now));
    session.approval_visible(true, now);
    session
}

#[test]
fn the_pending_card_shows_tool_args_diff_and_keys() {
    let mut app = App::new("m".into());
    app.sessions[0] = pending_session();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Approve edit_file"), "{text}");
    assert!(text.contains("README.md"));
    assert!(text.contains("-Old intro line."));
    assert!(text.contains("+New intro line."));
    assert!(text.contains("y approve"));
    assert!(text.contains("n deny"));
}

fn tall_pending_session() -> Session {
    let root = workspace("tall");
    let body: String = (1..=200).map(|n| format!("line {n}\n")).collect();
    let card = approval::card(
        &root,
        &call("write_file", json!({"path": "long.txt", "content": body})),
    )
    .unwrap();
    let _ = fs::remove_dir_all(root);
    let mut session = Session::new("m".into());
    // A long earlier message gives PgUp something to scroll through.
    session.insert(&"earlier context line\n".repeat(60));
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    assert!(session.await_approval(attempt, card, Instant::now()));
    session
}

fn screen(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Mirrors the main loop: draw, then report whether the card's header is on screen.
fn draw_and_report(terminal: &mut Terminal<TestBackend>, app: &mut App, now: Instant) -> String {
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let session = &mut app.sessions[0];
    let drawn = session.approval_header_drawn();
    session.approval_visible(drawn, now);
    screen(terminal)
}

#[test]
fn a_tall_card_keeps_its_header_path_and_key_line_on_screen() {
    let mut app = App::new("m".into());
    app.sessions[0] = tall_pending_session();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let text = draw_and_report(&mut terminal, &mut app, Instant::now());
    assert!(text.contains("Approve write_file"), "{text}");
    assert!(text.contains("long.txt"), "{text}");
    assert!(text.contains("+line 1"), "{text}");
    assert!(text.contains("more diff lines"), "{text}");
    assert!(
        text.contains("y approve · a always allow write_file in this mission · n deny"),
        "{text}"
    );
    assert!(text.contains("Esc cancels the turn (deny)"), "{text}");
}

#[test]
fn a_card_scrolled_out_of_view_does_not_accept_keys_until_its_header_returns() {
    let mut app = App::new("m".into());
    app.sessions[0] = tall_pending_session();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let start = Instant::now();
    draw_and_report(&mut terminal, &mut app, start);
    let armed = start + approval::ARM + Duration::from_millis(1);
    app.sessions[0].scroll_rows(-40);
    let text = draw_and_report(&mut terminal, &mut app, armed);
    assert!(!text.contains("Approve write_file"), "{text}");
    let later = armed + approval::ARM * 4;
    assert_eq!(app.sessions[0].answer_approval('y', later), None);
    assert_eq!(app.sessions[0].answer_approval('n', later), None);
    // Back at the bottom: shown again, so armed again only after the delay.
    app.sessions[0].scroll_rows(40);
    let text = draw_and_report(&mut terminal, &mut app, later);
    assert!(text.contains("Approve write_file"), "{text}");
    assert_eq!(app.sessions[0].answer_approval('y', later), None);
    assert_eq!(
        app.sessions[0].answer_approval('n', later + approval::ARM),
        Some(Decision::Deny)
    );
}

#[test]
fn cancel_and_restore_drop_the_pending_action() {
    let mut session = pending_session();
    assert!(session.pending_approval().is_some());
    let saved = serde_json::to_string(&session).unwrap();
    let restored: Session = serde_json::from_str(&saved).unwrap();
    assert!(
        restored.pending_approval().is_none(),
        "restore never re-asks or runs"
    );
    let armed = Instant::now() + approval::ARM;
    assert_eq!(session.answer_approval('y', armed), Some(Decision::Approve));
    assert!(session.pending_approval().is_none());
    let mut session = pending_session();
    session.cancel();
    assert!(session.pending_approval().is_none());
    assert!(!session.status.active());
    // A retried turn never shows the old attempt's card.
    let mut session = pending_session();
    let attempt = session.attempt;
    session.apply(attempt, Update::Failed("boom".into()));
    session.retry().unwrap();
    assert!(session.status.active());
    assert!(session.pending_approval().is_none());
}

#[test]
fn the_prompt_title_says_how_to_answer_when_the_draft_has_text() {
    let mut app = App::new("m".into());
    app.sessions[0] = pending_session();
    app.sessions[0].type_char('h');
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("clear the draft to answer"), "{text}");
}

#[test]
fn y_or_n_never_decides_while_the_draft_has_text() {
    // Issue #264: typeahead into the prompt ("hey", "you", "hold on") must not decide.
    let mut session = pending_session();
    let t0 = Instant::now() + Duration::from_secs(1);
    for (i, ch) in "hey".chars().enumerate() {
        let at = t0 + Duration::from_secs(i as u64 * 2);
        session.note_key(at);
        if session.approval_key_answers(at) {
            // Only the leading h could be eligible, and it is not y or n.
            assert_eq!(session.answer_approval(ch, at), None);
        }
        session.type_char(ch);
    }
    assert_eq!(session.draft, "hey");
    let later = t0 + Duration::from_secs(60);
    session.note_key(later);
    assert!(!session.approval_key_answers(later));
    assert_eq!(session.answer_approval('y', later), None);
    assert_eq!(session.answer_approval('n', later), None);
    assert!(session.pending_approval().is_some());
}

#[test]
fn y_with_an_empty_draft_needs_a_quiet_half_second() {
    let mut session = pending_session();
    let t0 = Instant::now() + Duration::from_secs(1);
    // A key just before (for example Backspace on an empty draft) blocks the answer.
    session.note_key(t0);
    let fast = t0 + Duration::from_millis(100);
    session.note_key(fast);
    assert!(!session.approval_key_answers(fast));
    assert_eq!(session.answer_approval('y', fast), None);
    assert!(session.pending_approval().is_some());
    // After a quiet 0.5 s the key decides.
    let quiet = fast + approval::ARM + Duration::from_millis(1);
    session.note_key(quiet);
    assert!(session.approval_key_answers(quiet));
    assert_eq!(session.answer_approval('y', quiet), Some(Decision::Approve));
}

#[test]
fn keys_before_the_card_is_armed_do_not_decide() {
    let mut session = pending_session();
    let shown = Instant::now();
    // Typeahead: a key arriving right after the card appears is ignored.
    assert_eq!(session.answer_approval('y', shown), None);
    assert_eq!(
        session.answer_approval('n', shown + Duration::from_millis(100)),
        None
    );
    assert!(session.pending_approval().is_some());
    let armed = shown + approval::ARM + Duration::from_millis(1);
    assert_eq!(session.answer_approval('x', armed), None);
    assert!(session.pending_approval().is_some());
    assert_eq!(session.answer_approval('n', armed), Some(Decision::Deny));
    assert!(session.pending_approval().is_none());
}

#[test]
fn chat_system_prompt_no_longer_claims_chat_cannot_edit() {
    let Message {
        content: prompt, ..
    } = alfredo_tui::chat_context::system_message(None, |_| None);
    assert!(prompt.contains("edit_file"), "{prompt}");
    assert!(!prompt.to_lowercase().contains("cannot edit"), "{prompt}");
}
