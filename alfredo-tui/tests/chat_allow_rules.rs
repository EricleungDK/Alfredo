//! Issue #238: always-allow rules. Per mission, by tool or run_command command prefix,
//! persisted in the State Directory; push never; HOST (escalated) runs always ask
//! (issue #311); auto-approved calls are still fingerprint-checked and logged with their rule.
mod ollama_fixture;

use alfredo_tui::{
    activity,
    chat_tools::{
        self, allow,
        allow::Rules,
        approval::{self, Card, CardKind, Decision, Gate, Log, Outcome},
    },
    model::{App, Event, Session, Update},
    provider::Ollama,
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

fn dir(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-allow-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), README).unwrap();
    root
}

fn shell(command: &str, escalated: bool) -> Card {
    Card {
        tool: "run_command".into(),
        arguments: json!({"command": command, "escalate": escalated}),
        kind: CardKind::Shell { escalated },
        summary: format!("run {command}"),
        preview: String::new(),
        fingerprint: "/x".into(),
    }
}

fn plain(tool: &str, kind: CardKind) -> Card {
    Card {
        tool: tool.into(),
        arguments: json!({}),
        kind,
        summary: tool.into(),
        preview: String::new(),
        fingerprint: String::new(),
    }
}

/// The approval card of a HOST (`escalate: true`) run_command, built as the tool loop builds it.
fn host_card(root: &Path, command: &str, cwd: Option<&str>) -> Card {
    let mut arguments = json!({"command": command, "escalate": true});
    if let Some(cwd) = cwd {
        arguments["cwd"] = json!(cwd);
    }
    approval::card(
        root,
        &alfredo_tui::model::ToolCall {
            name: "run_command".into(),
            arguments,
        },
    )
    .unwrap()
}

#[test]
fn command_prefixes_match_whole_words_and_refuse_shell_operators() {
    assert!(allow::command_matches("cargo test", "cargo test"));
    assert!(allow::command_matches("cargo test", "cargo test --lib"));
    assert!(allow::command_matches("cargo test", "  cargo   test  x "));
    assert!(!allow::command_matches("cargo test", "cargo testx"));
    assert!(!allow::command_matches("cargo test", "cargo"));
    for bad in [
        "cargo test; rm -rf ~",
        "cargo test && rm x",
        "cargo test | sh",
        "cargo test `rm x`",
        "cargo test $(rm x)",
        "cargo test > out",
        "cargo test < in",
        "cargo test\nrm x",
        "cargo test & rm x",
    ] {
        assert!(!allow::command_matches("cargo test", bad), "{bad:?}");
    }
    // A rule with operators matches only that exact command.
    assert!(allow::command_matches(
        "make && make test",
        "make && make test"
    ));
    assert!(!allow::command_matches(
        "make && make test",
        "make && make test; rm x"
    ));
    assert!(!allow::command_matches("", "anything"));
}

#[test]
fn rules_persist_and_restore_and_slash_command_lists_and_clears() {
    let state = dir("persist");
    let rules = Rules::new(&state);
    assert!(allow::slash(&rules, "")
        .unwrap()
        .contains("No always-allow rules"));
    let edit = rules.add_for(&plain("edit_file", CardKind::Edit)).unwrap();
    let test = rules.add_for(&shell("cargo test", false)).unwrap();
    // Adding the same rule twice keeps one.
    assert_eq!(rules.add_for(&shell("cargo test", false)).unwrap(), test);
    // Restored on restart: a fresh handle reads the file.
    let again = Rules::new(&state);
    assert_eq!(again.list().unwrap(), vec![edit.clone(), test.clone()]);
    assert_eq!(
        again.matching(&plain("edit_file", CardKind::Edit)),
        Some(edit.clone())
    );
    assert_eq!(
        again.matching(&shell("cargo test --lib", false)),
        Some(test.clone())
    );
    assert_eq!(again.matching(&plain("write_file", CardKind::Edit)), None);
    let listed = allow::slash(&again, "").unwrap();
    assert!(
        listed.contains(&format!("#{} edit_file", edit.id)),
        "{listed}"
    );
    assert!(listed.contains("cargo test"), "{listed}");
    assert!(allow::slash(&again, "clear 99").is_err());
    assert!(allow::slash(&again, "bogus")
        .unwrap_err()
        .starts_with("Usage: "));
    allow::slash(&again, &format!("clear {}", edit.id)).unwrap();
    assert_eq!(Rules::new(&state).list().unwrap(), vec![test]);
    allow::slash(&again, "clear").unwrap();
    assert!(Rules::new(&state).list().unwrap().is_empty());
    let _ = fs::remove_dir_all(state);
}

/// Issue #319: a present but unreadable rules file is an error, never "no rules".
fn assert_refused_and_untouched(rules: &Rules, file: &Path, bytes: &[u8]) {
    let path = file.display().to_string();
    for error in [
        rules.add_for(&shell("make", false)).unwrap_err(),
        rules.remove(1).unwrap_err(),
        rules.clear().unwrap_err(),
    ] {
        assert!(error.contains(&path), "{error}");
        assert!(
            error.contains(&format!("mv \"{path}\" \"{path}.bad\"")),
            "{error}"
        );
        assert!(error.contains("original preserved"), "{error}");
    }
    let shown = allow::slash(rules, "").unwrap_err();
    assert!(shown.contains(&path), "{shown}");
    assert!(allow::slash(rules, "clear").is_err());
    assert!(allow::slash(rules, "clear 1").is_err());
    // Fail closed: nothing matches, even a call the old rules covered.
    assert_eq!(rules.matching(&shell("cargo test", false)), None);
    assert_eq!(fs::read(file).unwrap(), bytes, "the file was rewritten");
}

#[test]
fn a_corrupt_rules_file_is_an_error_naming_the_path_and_is_never_overwritten() {
    let state = dir("corrupt");
    let rules = Rules::new(&state);
    rules.add_for(&shell("cargo test", false)).unwrap();
    let file = state.join(allow::FILE);
    // One stray character, as a hand edit leaves it.
    let corrupt = format!("{}x", fs::read_to_string(&file).unwrap());
    fs::write(&file, &corrupt).unwrap();
    assert_refused_and_untouched(&rules, &file, corrupt.as_bytes());
    let _ = fs::remove_dir_all(state);
}

#[test]
fn an_oversized_rules_file_is_an_error_naming_the_path_and_is_never_overwritten() {
    let state = dir("big");
    let rules = Rules::new(&state);
    rules.add_for(&shell("cargo test", false)).unwrap();
    let file = state.join(allow::FILE);
    // Valid JSON, but one byte over 1 MiB.
    let mut big = b"[".to_vec();
    big.extend(vec![b' '; 1024 * 1024]);
    big.extend(b"]");
    assert!(big.len() > 1024 * 1024);
    fs::write(&file, &big).unwrap();
    assert_refused_and_untouched(&rules, &file, &big);
    let _ = fs::remove_dir_all(state);
}

#[test]
fn an_empty_rules_file_means_no_rules_and_a_missing_one_is_silent() {
    let state = dir("empty-missing");
    let rules = Rules::new(&state);
    // Missing: no error anywhere.
    assert!(allow::slash(&rules, "")
        .unwrap()
        .contains("No always-allow rules"));
    assert_eq!(rules.matching(&shell("cargo test", false)), None);
    // Empty (0 bytes): no rules either, and it is not an error. Adding works.
    fs::write(state.join(allow::FILE), b"").unwrap();
    assert!(allow::slash(&rules, "")
        .unwrap()
        .contains("No always-allow rules"));
    assert_eq!(rules.matching(&shell("cargo test", false)), None);
    assert_eq!(rules.clear().unwrap(), 0);
    let added = rules.add_for(&shell("cargo test", false)).unwrap();
    assert_eq!(rules.matching(&shell("cargo test", false)), Some(added));
    let _ = fs::remove_dir_all(state);
}

#[test]
fn push_is_never_allowlistable_even_from_a_hand_edited_file() {
    let state = dir("push");
    let rules = Rules::new(&state);
    let push = plain("git_push", CardKind::Push);
    assert!(!allow::offered(&push));
    assert!(rules.add_for(&push).is_err());
    // Read tools are not allowlistable either.
    assert!(rules.add_for(&plain("read_file", CardKind::Edit)).is_err());
    fs::write(
        state.join(allow::FILE),
        r#"[{"id":1,"tool":"git_push"},{"id":2,"tool":"run_command"},{"id":3,"tool":"git_push","command":"x","escalated":true}]"#,
    )
    .unwrap();
    assert_eq!(rules.list().unwrap().len(), 3);
    assert_eq!(rules.matching(&push), None);
    // A run_command rule without a command never matches.
    assert_eq!(rules.matching(&shell("ls", false)), None);
    // A git_push card disguised as another kind still never matches a git_push rule.
    assert_eq!(rules.matching(&plain("git_push", CardKind::Git)), None);
    let _ = fs::remove_dir_all(state);
}

/// Issue #311: a HOST run gets no rule, and a sandboxed rule never covers it. A rule with
/// `escalated` (written before the fix) never matches, host or sandboxed.
#[test]
fn escalated_runs_never_match_a_rule() {
    let state = dir("escalated");
    let rules = Rules::new(&state);
    let sandboxed = rules.add_for(&shell("npm install", false)).unwrap();
    assert!(!sandboxed.escalated);
    // A sandboxed rule does not cover a host run.
    assert_eq!(rules.matching(&shell("npm install", true)), None);
    assert!(rules.add_for(&shell("npm install", true)).is_err());
    fs::write(
        state.join(allow::FILE),
        r#"[{"id":1,"tool":"run_command","command":"npm install","escalated":true}]"#,
    )
    .unwrap();
    let host = rules.list().unwrap().remove(0);
    assert!(host.escalated);
    assert_eq!(rules.matching(&shell("npm install", true)), None);
    assert_eq!(rules.matching(&shell("npm install", false)), None);
    assert!(
        allow::describe(&host).contains("inactive, HOST runs always ask"),
        "{}",
        allow::describe(&host)
    );
    let _ = fs::remove_dir_all(state);
}

fn pending(card: Card) -> Session {
    let mut session = Session::new("m".into());
    session.insert("go");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    let now = Instant::now();
    assert!(session.await_approval(attempt, card, now));
    // Stands in for a draw that showed the whole card (issue #303).
    session.set_approval_whole(true);
    session.approval_visible(true, now);
    session
}

fn screen(card: Card) -> String {
    let mut app = App::new("m".into());
    app.sessions[0] = pending(card);
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn the_card_offers_always_allow_except_for_push_and_host_runs() {
    assert!(screen(plain("edit_file", CardKind::Edit)).contains("a always allow"));
    assert!(!screen(plain("git_push", CardKind::Push)).contains("a always allow"));
    assert!(screen(shell("npm install", false)).contains("a always allow"));
    // Issue #311: a HOST run is never offered `a`, and its key line says why.
    let host = screen(shell("npm install", true));
    assert!(!host.contains("a always allow"), "{host}");
    assert!(
        host.contains("a not offered: HOST runs always ask"),
        "{host}"
    );
}

#[test]
fn a_waits_for_the_arm_delay_and_never_answers_a_push_card() {
    let mut session = pending(plain("edit_file", CardKind::Edit));
    let shown = Instant::now();
    assert_eq!(session.answer_approval('a', shown), None);
    let armed = shown + approval::ARM + Duration::from_millis(1);
    assert_eq!(
        session.answer_approval('a', armed),
        Some(Decision::AlwaysAllow)
    );
    let mut push = pending(plain("git_push", CardKind::Push));
    assert_eq!(push.answer_approval('a', armed), None);
    assert!(push.pending_approval().is_some());
}

fn edit_call() -> Reply {
    edit_call_with(
        json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "New intro line."}),
    )
}

fn edit_call_with(arguments: serde_json::Value) -> Reply {
    Reply::Stream(
        vec![
            format!(
                "{}\n",
                json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"edit_file","arguments":arguments}}]},"done":false})
            )
            .into_bytes(),
            format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )
}

async fn turn(root: &Path, gate: Gate) -> Vec<Event> {
    turn_with(root, gate, edit_call()).await
}

async fn turn_with(root: &Path, gate: Gate, call: Reply) -> Vec<Event> {
    let addr = ollama_fixture::reserve();
    let first = Mutex::new(Some(call));
    let fixture = ollama_fixture::serve(addr, move |_, index| {
        if index == 0 {
            first.lock().unwrap().take().unwrap()
        } else {
            done("done")
        }
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let mut session = Session::new("fixture".into());
    session.insert("edit");
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
        Some(gate),
    )
    .await;
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    events
}

fn gate(decision: Decision, log: &Log, rules: &Rules) -> (Gate, Arc<Mutex<usize>>) {
    let (sender, mut receiver) = mpsc::channel::<approval::Request>(4);
    let asked = Arc::new(Mutex::new(0));
    let count = asked.clone();
    tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            *count.lock().unwrap() += 1;
            let _ = request.reply.send(decision);
        }
    });
    (
        Gate::new(sender, Some(log.clone())).with_rules(rules.clone()),
        asked,
    )
}

#[tokio::test]
async fn always_allow_then_an_identical_action_runs_without_asking_and_is_logged_by_rule() {
    let root = dir("loop");
    let state = dir("loop-state");
    let log = Log::new(&state);
    let rules = Rules::new(&state);
    let (first, asked) = gate(Decision::AlwaysAllow, &log, &rules);
    turn(&root, first).await;
    assert_eq!(*asked.lock().unwrap(), 1);
    let rule = rules.list().unwrap().pop().expect("rule created");
    assert_eq!(rule.tool, "edit_file");
    fs::write(root.join("README.md"), README).unwrap();
    let (second, asked) = gate(Decision::Deny, &log, &rules);
    turn(&root, second).await;
    assert_eq!(*asked.lock().unwrap(), 0, "auto-approved, never asked");
    assert!(fs::read_to_string(root.join("README.md"))
        .unwrap()
        .contains("New intro line."));
    let records = log.records();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|r| r.outcome == Outcome::Applied));
    assert_eq!(records[0].rule, Some(rule.id), "created by a");
    assert_eq!(records[1].rule, Some(rule.id));
    let entries = activity::chat_entries(&records, "");
    assert!(
        entries[0].summary.contains(&format!("rule #{}", rule.id)),
        "{}",
        entries[0].summary
    );
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(state);
}

/// The session after a turn's events, as the terminal applies them.
fn replay(events: Vec<Event>) -> Session {
    let mut session = Session::new("fixture".into());
    session.insert("edit");
    session.begin().unwrap();
    let attempt = session.attempt;
    for event in events {
        session.apply(attempt, event.update);
    }
    session
}

fn rows(session: Session, width: u16) -> String {
    let mut app = App::new("m".into());
    app.sessions[0] = session;
    let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    buffer
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Issue #320: only a rule-approved call's row says which rule approved it.
#[tokio::test]
async fn only_a_rule_approved_call_says_rule_n_on_its_row() {
    let root = dir("mark");
    let state = dir("mark-state");
    let log = Log::new(&state);
    let rules = Rules::new(&state);
    let (first, _) = gate(Decision::Approve, &log, &rules);
    let keypress = replay(turn(&root, first).await);
    assert!(
        !rows(keypress, 120).contains("rule #"),
        "a y-approved call has no rule marker"
    );
    let rule = rules.add_for(&plain("edit_file", CardKind::Edit)).unwrap();
    fs::write(root.join("README.md"), README).unwrap();
    let (second, asked) = gate(Decision::Deny, &log, &rules);
    let by_rule = replay(turn(&root, second).await);
    assert_eq!(*asked.lock().unwrap(), 0);
    let screen = rows(by_rule, 120);
    assert!(screen.contains(&format!("(rule #{})", rule.id)), "{screen}");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(state);
}

/// Issue #320: a rule-approved run_command shows `running run_command` while it runs.
#[tokio::test]
async fn a_rule_approved_run_command_reads_running_run_command_until_it_ends() {
    if !Path::new("/usr/bin/bwrap").exists() && !Path::new("/bin/bwrap").exists() {
        eprintln!("SKIP: bubblewrap unavailable");
        return;
    }
    let root = dir("running");
    let state = dir("running-state");
    let log = Log::new(&state);
    let rules = Rules::new(&state);
    let command = "touch started; sleep 3";
    rules.add_for(&shell(command, false)).unwrap();
    let addr = ollama_fixture::reserve();
    let arguments = json!({"command": command});
    let first = Mutex::new(Some(Reply::Stream(
        vec![
            format!(
                "{}\n",
                json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"run_command","arguments":arguments}}]},"done":false})
            )
            .into_bytes(),
            format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )));
    let fixture = ollama_fixture::serve(addr, move |_, index| {
        if index == 0 {
            first.lock().unwrap().take().unwrap()
        } else {
            done("done")
        }
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let mut session = Session::new("fixture".into());
    session.insert("run it");
    let messages = session.begin().unwrap();
    let attempt = session.attempt;
    let (sender, mut receiver) = mpsc::channel::<Event>(256);
    let (gate, asked) = gate(Decision::Deny, &log, &rules);
    let workspace = root.clone();
    let handle = tokio::spawn(async move {
        chat_tools::run_turn(
            &provider,
            &workspace,
            0,
            attempt,
            "fixture".into(),
            messages,
            sender,
            Some(gate),
        )
        .await
    });
    let mut seen = false;
    while let Some(event) = receiver.recv().await {
        let running = matches!(event.update, Update::RuleRunning { .. });
        session.apply(attempt, event.update);
        if running {
            seen = true;
            assert_eq!(session.wait_phase(), Some("running run_command"));
            assert_eq!(session.short_status(), "running");
            // Esc clears it at once; the turn was cancelled by the owner.
            let mut stopped = session.clone();
            stopped.cancel();
            assert_eq!(stopped.wait_phase(), None);
        }
    }
    handle.await.unwrap();
    assert!(seen, "the rule path reports the running command");
    assert_eq!(*asked.lock().unwrap(), 0, "no card was shown");
    assert_eq!(session.wait_phase(), None, "cleared once the result is in");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(state);
}

/// A rule announced for a call that never reported (Esc) marks no later call.
#[test]
fn a_cancelled_rule_run_does_not_mark_the_next_turns_card_approved_call() {
    let mut session = Session::new("m".into());
    session.insert("go");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(
        attempt,
        Update::RuleRunning {
            tool: "run_command".into(),
            rule: 7,
        },
    );
    session.cancel();
    session.insert("again");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(
        attempt,
        Update::ToolResult {
            call: alfredo_tui::model::ToolCall {
                name: "edit_file".into(),
                arguments: json!({}),
            },
            result: "ok".into(),
            new_round: true,
        },
    );
    let at = session.messages.len() - 1;
    assert_eq!(session.rule_of(at, 0), None);
}

/// Issue #320: `/allow` lists every rule in full on its own wrapped row.
#[test]
fn allow_lists_every_rule_in_full_at_100_columns() {
    let state = dir("list");
    let rules = Rules::new(&state);
    let long = format!(
        "cargo test --features {}",
        "very-long-feature-name-".repeat(6)
    );
    // Over 80 characters no card offers `a` any more (#409), but an older rule file can
    // still hold such a rule, and `/allow` shows it in full.
    // A HOST rule from before issue #311 (no card can add one now) is listed as inactive.
    fs::write(
        state.join(allow::FILE),
        json!([
            {"id": 1, "tool": "run_command", "command": long},
            {"id": 2, "tool": "run_command", "command": "echo onhost true", "escalated": true}
        ])
        .to_string(),
    )
    .unwrap();
    rules.add_for(&plain("edit_file", CardKind::Edit)).unwrap();
    let listed = allow::slash(&rules, "").unwrap();
    assert_eq!(listed.lines().count(), 4, "heading and one row per rule");
    assert!(!listed.contains('…'), "{listed}");
    let mut app = App::new("m".into());
    app.transient_rows = listed.lines().map(str::to_string).collect();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    // Only the chat pane (right of the side pane) is read, row by row.
    let screen = buffer
        .content()
        .chunks(100)
        .map(|row| {
            let row: String = row.iter().map(|cell| cell.symbol()).collect();
            row.split("││")
                .nth(1)
                .unwrap_or("")
                .trim_end_matches('│')
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let squashed: String = screen.chars().filter(|c| !c.is_whitespace()).collect();
    for line in listed.lines().skip(1) {
        let wanted: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            squashed.contains(&wanted),
            "missing in full: {line}\n{screen}"
        );
    }
    assert!(listed.contains("on HOST"), "{listed}");
    assert!(
        listed.contains("inactive, HOST runs always ask"),
        "{listed}"
    );
    let _ = fs::remove_dir_all(state);
}

#[tokio::test]
async fn an_auto_approved_call_is_still_refused_when_the_preview_no_longer_applies() {
    // The rule never bypasses preview refusal: an edit whose old_text is gone is refused
    // before asking or running, and nothing is logged as applied.
    let root = dir("stale");
    let state = dir("stale-state");
    let log = Log::new(&state);
    let rules = Rules::new(&state);
    rules.add_for(&plain("edit_file", CardKind::Edit)).unwrap();
    fs::write(root.join("README.md"), "# other\n").unwrap();
    let (gate, asked) = gate(Decision::Deny, &log, &rules);
    turn(&root, gate).await;
    assert_eq!(*asked.lock().unwrap(), 0);
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "# other\n"
    );
    assert!(log.records().is_empty());
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(state);
}

/// Issue #272: a write_file rule never covers a write that creates directories.
#[test]
fn a_write_file_rule_does_not_cover_directory_creating_writes() {
    let root = dir("write-dirs");
    fs::create_dir(root.join("docs")).unwrap();
    let card = |path: &str| {
        approval::card(
            &root,
            &alfredo_tui::model::ToolCall {
                name: "write_file".into(),
                arguments: json!({"path": path, "content": "x\n"}),
            },
        )
        .unwrap()
    };
    let rules = Rules::new(&dir("write-dirs-state"));
    let existing = card("docs/a.md");
    rules.add_for(&existing).unwrap();
    assert!(rules.matching(&existing).is_some());
    assert!(
        rules.matching(&card("docs/b.md")).is_some(),
        "existing dir still auto-approved"
    );
    let creating = card("docs/new/deep/c.md");
    assert!(creating.summary.contains("new directories"));
    assert!(!allow::offered(&creating), "no always-allow offered");
    assert!(allow::offer(&creating).is_none());
    assert!(allow::rule_for(&creating).is_err());
    assert!(
        rules.matching(&creating).is_none(),
        "a missing dir still asks"
    );
    let _ = fs::remove_dir_all(root);
}

const PROTECTED_REASON: &str = "CI and tool config can run code or change git; always asks";

fn path_card(tool: &str, path: &str) -> Card {
    Card {
        tool: tool.into(),
        arguments: json!({"path": path, "content": "x\n", "old_text": "a", "new_text": "b"}),
        kind: CardKind::Edit,
        summary: format!("{tool} {path}"),
        preview: String::new(),
        fingerprint: format!("/w/{path}:present"),
    }
}

const PROTECTED: &[&str] = &[
    ".github/workflows/ci.yml",
    ".husky/pre-commit",
    ".vscode/tasks.json",
    ".idea/workspace.xml",
    ".devcontainer/devcontainer.json",
    ".cargo/config.toml",
    ".gitlab-ci.yml",
    ".pre-commit-config.yaml",
    ".gitattributes",
    ".gitmodules",
];

/// Issue #791: the classifier. Every directory and file name, each spelling.
#[test]
fn the_protected_classifier_matches_names_in_any_spelling() {
    for path in PROTECTED {
        assert!(allow::protected(path), "{path}");
        assert!(allow::protected(&format!("./{path}")), "./{path}");
        assert!(
            allow::protected(&format!("sub/dir/{path}")),
            "sub/dir/{path}"
        );
        assert!(allow::protected(&path.to_uppercase()), "{path} upper");
        assert!(allow::protected(&format!("  {path}  ")), "{path} padded");
    }
    for path in [
        "./.github/x.yml",
        ".GitHub/x.yml",
        "sub/.husky/pre-commit",
        "a/.gitattributes",
        "a//.github///x",
        "./a/./.vscode/x",
        "a/.GITMODULES",
        ".github",
        "src/.vscode",
        "/w/.github/x.yml",
        "a/.Gitlab-CI.yml",
        // resolve_target walks Path::components(), which drops `.` and trailing slashes.
        ".gitattributes/",
        ".gitattributes/.",
        "a/.gitmodules//",
        "./.gitlab-ci.yml/./",
        "./a/./.gitattributes",
        "./a/./.pre-commit-config.yaml/",
    ] {
        assert!(allow::protected(path), "{path}");
    }
}

/// Issue #791: names that look close but are not in the set stay ordinary.
#[test]
fn the_protected_classifier_leaves_ordinary_paths_alone() {
    for path in [
        "calc.py",
        "src/x.rs",
        "docs/github.md",
        "github/x.yml",
        ".githubx/a",
        "x.github/a",
        ".gith/a",
        ".gitignore",
        ".gitattributes.bak",
        "a.gitmodules",
        "src/.gitattributes/x",
        ".gitlab-ci.yml/x",
        "husky/pre-commit",
        "cargo/config.toml",
        "vscode/tasks.json",
        ".env.example",
        "",
        ".",
        "./",
    ] {
        assert!(!allow::protected(path), "{path:?}");
    }
}

/// Issue #791: a protected path never gets `a` and no rule matches it, even a rule typed
/// into chat-allow.json by hand; the card says why and `a` decides nothing.
#[test]
fn protected_paths_are_never_offered_or_matched() {
    let state = dir("protected-state");
    fs::write(
        state.join(allow::FILE),
        r#"[{"id":1,"tool":"write_file"},{"id":2,"tool":"edit_file"}]"#,
    )
    .unwrap();
    let rules = Rules::new(&state);
    assert_eq!(rules.list().unwrap().len(), 2, "an old file still loads");
    let spellings = [
        ".gitattributes/",
        ".gitattributes/.",
        "a/.gitmodules//",
        "./.gitlab-ci.yml/./",
        "./a/./.gitattributes",
    ];
    for tool in ["write_file", "edit_file"] {
        for path in PROTECTED.iter().chain(spellings.iter()) {
            let card = path_card(tool, path);
            assert!(!allow::offered(&card), "{tool} {path}");
            assert!(allow::offer(&card).is_none(), "{tool} {path}");
            assert!(!allow::available(&card), "{tool} {path}");
            assert!(allow::rule_for(&card).is_err(), "{tool} {path}");
            assert!(rules.add_for(&card).is_err(), "{tool} {path}");
            assert!(rules.matching(&card).is_none(), "{tool} {path}");
            assert_eq!(
                allow::unavailable_reason(&card).as_deref(),
                Some(PROTECTED_REASON)
            );
            assert_eq!(
                allow::undecided_notice('a', Some(&card)),
                format!("a not offered: {PROTECTED_REASON}; press y or n")
            );
        }
    }
    assert_eq!(rules.list().unwrap().len(), 2, "no rule was added");
    let _ = fs::remove_dir_all(state);
}

/// Issue #791: the card's key line and the `a` key on a protected path.
#[test]
fn a_protected_card_says_why_and_a_decides_nothing() {
    let card = path_card("edit_file", ".github/workflows/ci.yml");
    let text = screen(card.clone());
    assert!(!text.contains("a always allow"), "{text}");
    assert!(
        text.contains("n deny · Esc cancels the turn (deny) · a not offered: CI and tool"),
        "{text}"
    );
    assert!(text.contains("always asks"), "{text}");
    let generic =
        "Read the approval card first; press y or n again (a always allows, not for push)";
    for key in ['y', 'n', 'x'] {
        assert_eq!(allow::undecided_notice(key, Some(&card)), generic, "{key}");
    }
    let mut session = pending(card.clone());
    let armed = Instant::now() + approval::ARM + Duration::from_millis(1);
    assert!(session.approval_key_answers(armed));
    assert_eq!(session.answer_approval('a', armed), None);
    assert!(session.pending_approval().is_some(), "card still pending");
    assert_eq!(session.answer_approval('y', armed), Some(Decision::Approve));
    // A protected path inside a new directory names the protected reason, not the
    // directory one.
    let mut nested = path_card("write_file", ".github/new/ci.yml");
    nested.fingerprint = "/w/.github/new/ci.yml:absent+dirs:new".into();
    assert_eq!(
        allow::unavailable_reason(&nested).as_deref(),
        Some(PROTECTED_REASON)
    );
}

/// Issue #791: with a rule present, ordinary files still match and keep their offer.
#[test]
fn ordinary_paths_still_match_rules() {
    let state = dir("ordinary-state");
    let rules = Rules::new(&state);
    rules.add_for(&path_card("write_file", "calc.py")).unwrap();
    rules.add_for(&path_card("edit_file", "calc.py")).unwrap();
    for tool in ["write_file", "edit_file"] {
        for path in ["calc.py", "src/x.rs", "docs/github.md", "./README.md"] {
            let card = path_card(tool, path);
            assert!(allow::offered(&card), "{tool} {path}");
            assert!(allow::offer(&card).is_some(), "{tool} {path}");
            assert_eq!(allow::unavailable_reason(&card), None, "{tool} {path}");
            assert!(rules.matching(&card).is_some(), "{tool} {path}");
            assert_eq!(
                allow::undecided_notice('a', Some(&card)),
                "Read the approval card first; press y or n again (a always allows, not for push)"
            );
        }
    }
    assert!(screen(path_card("edit_file", "docs/github.md")).contains("a always allow"));
    // The path rule is for the file tools only: another tool's card is untouched.
    let commit = path_card("git_commit", ".github/x");
    assert!(allow::offered(&commit));
    let _ = fs::remove_dir_all(state);
}

/// A workspace with a real card builder: `ci`/`wf` alias protected directories, `d` an
/// ordinary one, `.pre-commit-config.yaml`/`.husky` are symlinks to ordinary targets.
fn alias_root(tag: &str) -> PathBuf {
    use std::os::unix::fs::symlink;
    let root = dir(tag);
    for sub in [".github/workflows", "docs", "tools", "hooks"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join(".github/workflows/ci.yml"), "name: ci\n").unwrap();
    fs::write(root.join("docs/a.md"), "name: ci\n").unwrap();
    fs::write(root.join("tools/pc.yaml"), "repos: []\n").unwrap();
    fs::write(root.join("tools/other.yaml"), "x: 1\n").unwrap();
    fs::write(root.join("hooks/pre-commit"), "#!/bin/sh\n").unwrap();
    symlink(".github", root.join("ci")).unwrap();
    symlink(".github/workflows", root.join("wf")).unwrap();
    symlink("docs", root.join("d")).unwrap();
    symlink("tools/pc.yaml", root.join(".pre-commit-config.yaml")).unwrap();
    symlink("hooks", root.join(".husky")).unwrap();
    root
}

fn real_card(root: &Path, tool: &str, path: &str) -> Card {
    approval::card(
        root,
        &alfredo_tui::model::ToolCall {
            name: tool.into(),
            arguments: json!({"path": path, "content": "z\n", "old_text": "name: ci", "new_text": "name: z"}),
        },
    )
    .unwrap()
}

fn both_tools_rules(tag: &str) -> Rules {
    let state = dir(tag);
    fs::write(
        state.join(allow::FILE),
        r#"[{"id":1,"tool":"write_file"},{"id":2,"tool":"edit_file"}]"#,
    )
    .unwrap();
    Rules::new(&state)
}

/// Issue #791 review: a symlinked directory alias of a protected directory is judged by
/// where the write lands, not by the name the model gave.
#[test]
fn a_symlinked_alias_of_a_protected_directory_still_asks() {
    let root = alias_root("alias");
    let rules = both_tools_rules("alias-state");
    for (tool, path) in [
        ("edit_file", "ci/workflows/ci.yml"),
        ("edit_file", "./ci/workflows/ci.yml"),
        ("write_file", "ci/workflows/ci.yml"),
        ("write_file", "wf/ci.yml"),
        ("write_file", "wf/evil.yml"),
    ] {
        let card = real_card(&root, tool, path);
        assert!(!allow::offered(&card), "{tool} {path}");
        assert!(rules.matching(&card).is_none(), "{tool} {path}");
        assert_eq!(
            allow::unavailable_reason(&card).as_deref(),
            Some(PROTECTED_REASON),
            "{tool} {path}"
        );
    }
    // An alias of an ordinary directory stays ordinary.
    for (tool, path) in [
        ("write_file", "d/a.md"),
        ("write_file", "d/new.md"),
        ("edit_file", "docs/a.md"),
    ] {
        let card = real_card(&root, tool, path);
        assert!(rules.matching(&card).is_some(), "{tool} {path}");
    }
    let _ = fs::remove_dir_all(root);
}

/// Issue #791 review: a protected name that is a symlink to an ordinary file or directory
/// at the workspace root protects what it points at, and the card still applies and undoes.
#[test]
fn a_root_protected_name_that_links_elsewhere_protects_its_target() {
    let root = alias_root("rootlink");
    let rules = both_tools_rules("rootlink-state");
    for (tool, path) in [
        ("write_file", "tools/pc.yaml"),
        ("write_file", "./tools/pc.yaml"),
        ("write_file", "hooks/pre-commit"),
        ("write_file", "hooks/new-hook"),
        ("write_file", "d/../hooks/pre-commit"),
    ] {
        let Ok(card) = approval::card(
            &root,
            &alfredo_tui::model::ToolCall {
                name: tool.into(),
                arguments: json!({"path": path, "content": "z\n"}),
            },
        ) else {
            continue; // `..` is refused before any card
        };
        assert!(!allow::offered(&card), "{tool} {path}");
        assert!(rules.matching(&card).is_none(), "{tool} {path}");
        assert_eq!(
            allow::unavailable_reason(&card).as_deref(),
            Some(PROTECTED_REASON)
        );
    }
    let ordinary = real_card(&root, "write_file", "tools/other.yaml");
    assert!(rules.matching(&ordinary).is_some());
    // y still writes it: the fingerprint re-check agrees with the card.
    let card = real_card(&root, "write_file", "tools/pc.yaml");
    assert!(
        chat_tools::undo::before(&root, &card).is_some(),
        "undo keeps working"
    );
    approval::apply(&root, &card).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("tools/pc.yaml")).unwrap(),
        "z\n"
    );
    let _ = fs::remove_dir_all(root);
}

/// Issue #791 review: a protected root name that links to a file or directory that does
/// not exist yet still protects it, through chained links, and a link loop fails closed.
#[test]
fn dangling_and_chained_root_links_protect_their_targets() {
    use std::os::unix::fs::symlink;
    let root = dir("dangling");
    let rules = both_tools_rules("dangling-state");
    fs::create_dir_all(root.join("tools")).unwrap();
    fs::create_dir_all(root.join("hk")).unwrap();
    fs::write(root.join("calc.py"), "name: ci\n").unwrap();
    symlink("tools/pc.yaml", root.join(".pre-commit-config.yaml")).unwrap();
    symlink("hk/real", root.join(".husky")).unwrap();
    symlink("hop", root.join(".gitattributes")).unwrap();
    symlink("tools/ga", root.join("hop")).unwrap();
    for path in ["tools/pc.yaml", "./tools/pc.yaml", "hk/real", "tools/ga"] {
        let card = real_card(&root, "write_file", path);
        assert!(!allow::offered(&card), "{path}");
        assert!(rules.matching(&card).is_none(), "{path}");
        assert_eq!(
            allow::unavailable_reason(&card).as_deref(),
            Some(PROTECTED_REASON),
            "{path}"
        );
    }
    let ordinary = real_card(&root, "write_file", "tools/other.yaml");
    assert!(rules.matching(&ordinary).is_some(), "an unrelated file");
    let _ = fs::remove_dir_all(root);
}

/// Issue #791 review: a link loop at the root never hangs and fails closed: every
/// write_file/edit_file asks.
#[test]
fn a_root_link_loop_fails_closed() {
    use std::os::unix::fs::symlink;
    let root = dir("loop-links");
    let rules = both_tools_rules("loop-links-state");
    fs::write(root.join("calc.py"), "name: ci\n").unwrap();
    symlink(".gitmodules", root.join(".gitmodules")).unwrap();
    symlink("b", root.join("a")).unwrap();
    symlink("a", root.join("b")).unwrap();
    symlink("a", root.join(".husky")).unwrap();
    for tool in ["write_file", "edit_file"] {
        let card = real_card(&root, tool, "calc.py");
        assert!(!allow::offered(&card), "{tool}");
        assert!(rules.matching(&card).is_none(), "{tool}");
    }
    let _ = fs::remove_dir_all(root);
}

/// Issue #791 review: a regular file under a protected name, and no protected names at
/// all, behave as before.
#[test]
fn regular_root_config_files_and_no_links_change_nothing() {
    let root = dir("regular");
    let rules = both_tools_rules("regular-state");
    fs::write(root.join(".pre-commit-config.yaml"), "name: ci\n").unwrap();
    fs::write(root.join("calc.py"), "name: ci\n").unwrap();
    let own = real_card(&root, "edit_file", ".pre-commit-config.yaml");
    assert!(rules.matching(&own).is_none());
    let calc = real_card(&root, "edit_file", "calc.py");
    assert!(rules.matching(&calc).is_some());
    let _ = fs::remove_dir_all(root);
}

/// Issue #791 review: the resolved path is read back from the card without being fooled by
/// `+dirs:`, `:` or the alias marker inside a file name.
#[test]
fn odd_file_names_cannot_hide_a_protected_directory() {
    use std::os::unix::fs::symlink;
    let root = dir("odd-names");
    let rules = both_tools_rules("odd-names-state");
    fs::create_dir_all(root.join("sub/.cargo")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    for name in ["x+dirs:y.toml", "z+aliases-protected", "a:b+dirs:c:d"] {
        fs::write(root.join("sub/.cargo").join(name), "name: ci\n").unwrap();
        fs::write(root.join("docs").join(name), "name: ci\n").unwrap();
    }
    symlink("sub/.cargo", root.join("cfg")).unwrap();
    symlink("docs", root.join("d")).unwrap();
    for name in ["x+dirs:y.toml", "z+aliases-protected", "a:b+dirs:c:d"] {
        for tool in ["edit_file", "write_file"] {
            let card = real_card(&root, tool, &format!("cfg/{name}"));
            assert!(rules.matching(&card).is_none(), "{tool} cfg/{name}");
            // (A write_file whose name holds `+dirs:` is asked anyway: the old directory
            // check reads the fingerprint text.)
            let card = real_card(&root, tool, &format!("d/{name}"));
            let asked_anyway = tool == "write_file" && name.contains("+dirs:");
            assert_eq!(
                rules.matching(&card).is_some(),
                !asked_anyway,
                "{tool} d/{name}"
            );
        }
    }
    let _ = fs::remove_dir_all(root);
}

/// Issue #791 review: only the path below the workspace root counts, so a workspace that
/// itself lives under `.idea` or `.cargo` keeps its rules for ordinary files.
#[test]
fn a_workspace_under_a_protected_directory_name_still_matches_rules() {
    let base = dir("under");
    let root = base.join(".idea/.cargo/ws");
    fs::create_dir_all(root.join(".github")).unwrap();
    fs::write(root.join("calc.py"), "name: ci\n").unwrap();
    fs::write(root.join(".github/ci.yml"), "name: ci\n").unwrap();
    let rules = both_tools_rules("under-state");
    for tool in ["write_file", "edit_file"] {
        let card = real_card(&root, tool, "calc.py");
        assert!(allow::offered(&card), "{tool}");
        assert!(rules.matching(&card).is_some(), "{tool}");
        let card = real_card(&root, tool, ".github/ci.yml");
        assert!(rules.matching(&card).is_none(), "{tool} protected inside");
    }
    let _ = fs::remove_dir_all(base);
}

/// Issue #791 through the tool loop: with a rule for the tool, a protected edit is asked
/// (unchanged on `n`, written on `y` with no rule id); an ordinary edit is not asked.
#[tokio::test]
async fn a_rule_never_skips_the_card_for_a_protected_file() {
    let root = dir("protected-loop");
    fs::create_dir_all(root.join(".github/workflows")).unwrap();
    let ci = root.join(".github/workflows/ci.yml");
    fs::write(&ci, "name: ci\n").unwrap();
    let state = dir("protected-loop-state");
    let log = Log::new(&state);
    fs::write(state.join(allow::FILE), r#"[{"id":1,"tool":"edit_file"}]"#).unwrap();
    let rules = Rules::new(&state);
    let call = || {
        edit_call_with(
            json!({"path": ".github/workflows/ci.yml", "old_text": "ci", "new_text": "pwned"}),
        )
    };
    let (denied, asked) = gate(Decision::Deny, &log, &rules);
    turn_with(&root, denied, call()).await;
    assert_eq!(*asked.lock().unwrap(), 1, "the card was shown");
    assert_eq!(fs::read_to_string(&ci).unwrap(), "name: ci\n", "unchanged");
    let (approved, asked) = gate(Decision::Approve, &log, &rules);
    turn_with(&root, approved, call()).await;
    assert_eq!(*asked.lock().unwrap(), 1);
    assert_eq!(fs::read_to_string(&ci).unwrap(), "name: pwned\n");
    assert_eq!(log.records().last().unwrap().rule, None, "answered by y");
    let (ordinary, asked) = gate(Decision::Deny, &log, &rules);
    turn(&root, ordinary).await;
    assert_eq!(
        *asked.lock().unwrap(),
        0,
        "README.md still runs by the rule"
    );
    assert!(fs::read_to_string(root.join("README.md"))
        .unwrap()
        .contains("New intro line."));
    assert_eq!(log.records().last().unwrap().rule, Some(1));
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(state);
}

/// Issue #311: a HOST rule written before the fix matches neither its exact command nor a
/// longer one.
#[test]
fn escalated_rules_from_before_issue_311_never_match_their_exact_command() {
    let state = dir("exact-host");
    let rules = Rules::new(&state);
    fs::write(
        state.join(allow::FILE),
        r#"[{"id":1,"tool":"run_command","command":"npm install","escalated":true}]"#,
    )
    .unwrap();
    assert_eq!(rules.matching(&shell("npm install", true)), None);
    assert_eq!(
        rules.matching(&shell("npm install attacker-pkg", true)),
        None
    );
    assert!(rules.add_for(&shell("npm install", true)).is_err());
    assert!(!allow::describe(&rules.list().unwrap()[0]).contains("longer"));
    let _ = fs::remove_dir_all(state);
}

/// Issue #311: a HOST (escalated) run_command is never offered `a`, so no rule can make a
/// later host run skip its card. On main these five all got `a`: four are push commands
/// the `git push` word check misses, and `./build.sh` is a repeatable host command.
#[test]
fn host_runs_are_never_offered_always_allow_so_no_rule_can_skip_their_card() {
    let root = dir("host-never");
    fs::create_dir(root.join("sub")).unwrap();
    let state = dir("host-never-state");
    let rules = Rules::new(&state);
    for command in [
        "g''it push origin main",
        "git pu''sh origin main",
        "G=git; $G push origin main",
        "git -c alias.p=push p origin main",
        "./build.sh",
    ] {
        for cwd in [None, Some("sub")] {
            let card = host_card(&root, command, cwd);
            assert!(card.escalated(), "{command} {cwd:?}");
            assert!(!allow::offered(&card), "{command} {cwd:?}: offered");
            assert_eq!(allow::offer(&card), None, "{command} {cwd:?}: hinted");
            assert!(
                rules.add_for(&card).is_err(),
                "{command} {cwd:?}: a rule was added"
            );
        }
    }
    let _ = fs::remove_dir_all(state);
    let _ = fs::remove_dir_all(root);
}

/// Issue #311: an escalated rule in `chat-allow.json` (written before the fix) never
/// matches a host run, whatever its command or cwd. Each of the five commands from the
/// issue is a legacy rule, and a legacy tool-wide escalated rule is included.
#[test]
fn an_old_escalated_rule_never_matches_a_host_run_from_any_cwd() {
    let root = dir("host-old-rule");
    fs::create_dir(root.join("sub")).unwrap();
    let state = dir("host-old-rule-state");
    let commands = [
        "./build.sh",
        "g''it push origin main",
        "git pu''sh origin main",
        "G=git; $G push origin main",
        "git -c alias.p=push p origin main",
    ];
    let mut legacy: Vec<String> = commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            format!(
                r#"{{"id":{},"tool":"run_command","command":{},"escalated":true}}"#,
                index + 1,
                json!(command)
            )
        })
        .collect();
    legacy.push(r#"{"id":9,"tool":"run_command","escalated":true}"#.into());
    fs::write(state.join(allow::FILE), format!("[{}]", legacy.join(","))).unwrap();
    let rules = Rules::new(&state);
    assert_eq!(rules.list().unwrap().len(), commands.len() + 1);
    for command in commands {
        for cwd in [None, Some("sub")] {
            let card = host_card(&root, command, cwd);
            assert_eq!(rules.matching(&card), None, "{command} {cwd:?}");
            assert!(allow::offer(&card).is_none(), "{command} {cwd:?}");
        }
    }
    let _ = fs::remove_dir_all(state);
    let _ = fs::remove_dir_all(root);
}

/// Issue #311: a legacy escalated rule is listed as inactive in every form `/allow` shows,
/// including a tool-wide one (`command` absent), which would otherwise read as coverage.
#[test]
fn every_legacy_escalated_rule_is_listed_inactive() {
    let state = dir("legacy-inactive");
    let rules = Rules::new(&state);
    fs::write(
        state.join(allow::FILE),
        r#"[{"id":1,"tool":"run_command","command":"./build.sh","escalated":true},{"id":2,"tool":"run_command","escalated":true}]"#,
    )
    .unwrap();
    let listed = rules.list().unwrap();
    assert_eq!(listed.len(), 2);
    for rule in &listed {
        let text = allow::describe(rule);
        assert!(text.contains("inactive, HOST runs always ask"), "{text}");
    }
    let _ = fs::remove_dir_all(state);
}

#[test]
fn git_push_through_run_command_is_never_allowlistable() {
    let state = dir("shell-push");
    let rules = Rules::new(&state);
    for command in [
        "git push origin feat --force",
        "git -C x push",
        "git -c k=v push origin feat:main",
    ] {
        for escalated in [false, true] {
            let card = shell(command, escalated);
            assert!(!allow::offered(&card), "{command}");
            assert!(allow::offer(&card).is_none());
            assert!(rules.add_for(&card).is_err());
        }
    }
    fs::write(
        state.join(allow::FILE),
        r#"[{"id":1,"tool":"run_command","command":"git","escalated":true},{"id":2,"tool":"run_command","command":"git push origin feat --force","escalated":true},{"id":3,"tool":"run_command","command":"git"}]"#,
    )
    .unwrap();
    assert_eq!(
        rules.matching(&shell("git push origin feat --force", true)),
        None
    );
    assert_eq!(rules.matching(&shell("git push", false)), None);
    // Other git commands stay allowlistable.
    assert!(rules.matching(&shell("git status", false)).is_some());
    let host = screen(shell("git push origin feat", true));
    assert!(host.contains("push: asked every time"), "{host}");
    let _ = fs::remove_dir_all(state);
}

/// Issue #409: the `a` hint shows the whole command as the card's `$` lines do, or `a` is
/// not offered; it never shows a cut or stripped command as what the rule matches.
#[test]
fn the_hint_shows_commands_of_80_characters_and_drops_a_at_81() {
    let at_limit = format!("echo {}", "A".repeat(75));
    assert_eq!(at_limit.chars().count(), 80);
    // Sandboxed: the hint shows the whole command. (A HOST run is never offered, #311.)
    let hint = allow::offer(&shell(&at_limit, false)).unwrap();
    assert!(hint.contains(&format!("`{at_limit}`")), "{hint}");
    let over = format!("{at_limit}B");
    let card = shell(&over, false);
    assert!(allow::offer(&card).is_none());
    assert!(!allow::available(&card));
    assert!(rules_for_test().add_for(&card).is_err());
    let text = screen(card);
    assert!(!text.contains("a always allow"), "{text}");
    assert!(
        text.contains("a not offered: too long to always-allow"),
        "{text}"
    );
    assert!(
        text.contains("y approve") && text.contains("n deny"),
        "{text}"
    );
    assert!(!text.contains("exactly"), "{text}");
    // A HOST run of any length is not offered, and the reason is that it always asks.
    let host = shell(&at_limit, true);
    assert!(allow::offer(&host).is_none());
    assert_eq!(
        allow::unavailable_reason(&host).as_deref(),
        Some("HOST runs always ask")
    );
}

fn rules_for_test() -> Rules {
    Rules::new(&dir("scratch"))
}

#[test]
fn the_hint_shows_a_tab_as_an_arrow_and_a_newline_as_a_return_arrow() {
    let tabbed = "echo AAAAAAAAAAAAAAAAAAAA ok;\techo SECOND; echo THIRD";
    let hint = allow::offer(&shell(tabbed, false)).unwrap();
    assert!(hint.contains("→"), "{hint}");
    assert!(hint.contains("echo SECOND; echo THIRD"), "{hint}");
    assert!(
        hint.contains("`echo AAAAAAAAAAAAAAAAAAAA ok;→echo SECOND; echo THIRD`"),
        "{hint}"
    );
    assert!(hint.ends_with(" exactly"), "{hint}");
    // The issue's repro (over 80 shown) is no longer cut: `a` is not offered.
    let long = format!("echo {} ok;\techo SECOND_PART; echo THIRD", "A".repeat(60));
    assert!(allow::offer(&shell(&long, false)).is_none());
    // A HOST run of the same command is not offered at all (issue #311).
    assert!(allow::offer(&shell(tabbed, true)).is_none());
    let multi = "printf 'a\tb\n'\tcat t.py";
    let hint = allow::offer(&shell(multi, false)).unwrap();
    assert!(hint.contains("'a→"), "{hint}");
    assert!(!hint.contains('\n') && hint.contains("↵"), "{hint}");
    assert!(hint.contains("cat t.py"), "{hint}");
    let lines = allow::offer(&shell("echo one\necho two", false)).unwrap();
    assert!(lines.contains("`echo one↵echo two`"), "{lines}");
    // A literal return arrow cannot pass for a newline; other control bytes are not
    // offered at all (the card's `$` lines would drop them).
    assert!(allow::show_command("a↵b").contains("<U+21B5>"));
    assert!(allow::offer(&shell("echo a\u{1b}[2Jb", false)).is_none());
    assert!(allow::offer(&shell("echo a\rb", false)).is_none());
}

/// A run_command card with the preview built as `shell.rs` does (one `$ ` line per line).
fn shell_with_preview(command: &str, escalated: bool) -> Card {
    let mut card = shell(command, escalated);
    card.preview = "cwd . · sandboxed · timeout 30s".to_string();
    for line in command.lines() {
        card.preview.push_str(&format!("\n$ {line}"));
    }
    card
}

#[test]
fn the_card_s_dollar_lines_and_the_hint_show_the_same_command_text() {
    // (command, the card's `$` lines, the command as the hint shows it)
    let cases: [(&str, &[&str], &str); 4] = [
        ("echo a\tb", &["$ echo a→   b"], "echo a→   b"),
        ("echo → x", &["$ echo <U+2192> x"], "echo <U+2192> x"),
        ("echo ↵ x", &["$ echo <U+21B5> x"], "echo <U+21B5> x"),
        (
            "echo one\necho two",
            &["$ echo one", "$ echo two"],
            "echo one↵echo two",
        ),
    ];
    for (command, rows, hint) in cases {
        assert_eq!(allow::show_command(command), hint);
        for escalated in [false, true] {
            let card = shell_with_preview(command, escalated);
            if escalated {
                // Issue #311: a HOST card offers no hint; its `$` lines still show the command.
                assert!(allow::offer(&card).is_none(), "{command:?}");
            } else {
                let offer = allow::offer(&card).unwrap();
                assert!(offer.contains(&format!("`{hint}`")), "{command:?}: {offer}");
            }
            let text = screen(card);
            let mut from = 0;
            for row in rows {
                let at = text[from..]
                    .find(row)
                    .unwrap_or_else(|| panic!("{command:?}: {row:?} missing in {text}"));
                from += at + row.len();
            }
        }
    }
}

#[test]
fn a_control_character_or_escape_lookalike_is_checked_before_trimming() {
    for command in [
        "ls\r", "\rls", "ls \u{b}", "\u{85}ls", "ls\u{85}", "ls\u{1b}",
    ] {
        assert!(
            allow::offer(&shell(command, false)).is_none(),
            "{command:?}"
        );
        assert!(allow::offer(&shell(command, true)).is_none(), "{command:?}");
        assert!(rules_for_test().add_for(&shell(command, false)).is_err());
    }
    // Plain surrounding whitespace is still trimmed away.
    assert!(allow::offer(&shell("  ls\n", false))
        .unwrap()
        .contains("`ls`"));
    // Text that looks like an escape could pass for a different command.
    for command in ["echo <U+21B5> x", "echo <U+2192>"] {
        let card = shell(command, false);
        assert!(allow::offer(&card).is_none(), "{command}");
        assert!(allow::unavailable_reason(&card)
            .unwrap()
            .contains("looks like an escape"));
        // A HOST run says why first (issue #311).
        assert_eq!(
            allow::unavailable_reason(&shell(command, true)).as_deref(),
            Some("HOST runs always ask")
        );
    }
}

/// Issue #409 / #320: the hint and `/allow` show a sandboxed command the same way. (Was: an
/// escalated card; a HOST run is never offered `a` since issue #311.)
#[test]
fn the_hint_and_slash_allow_show_the_whole_command_the_same_way() {
    let command = "make\tbuild;\necho done → ok";
    let card = shell(command, false);
    let text = screen(card.clone());
    let shown = allow::show_command(command);
    assert!(text.contains(&shown), "{text}");
    let state = dir("same-view");
    let rules = Rules::new(&state);
    let rule = rules.add_for(&card).unwrap();
    assert_eq!(rule.command.as_deref(), Some(command));
    let described = allow::describe(&rule);
    assert!(described.contains(&format!("`{shown}`")), "{described}");
    assert!(allow::offer(&card).unwrap().contains(&format!("`{shown}`")));
    // The same command on HOST is neither offered nor added (issue #311).
    let host = shell(command, true);
    assert!(allow::offer(&host).is_none());
    assert!(rules.add_for(&host).is_err());
    let _ = fs::remove_dir_all(state);
}

#[test]
fn a_sandboxed_prefix_card_says_any_arguments_are_allowed() {
    let text = allow::offer(&shell("cargo test", false)).unwrap();
    assert!(text.contains("ANY arguments"), "{text}");
    assert!(text.contains("sandboxed only"), "{text}");
}

fn dirs_card(tool: &str) -> Card {
    Card {
        tool: tool.into(),
        arguments: json!({"path": "new/c.md", "content": "x\n"}),
        kind: CardKind::Edit,
        summary: format!("{tool} new/c.md (creates new directories)"),
        preview: String::new(),
        fingerprint: "/x/new/c.md:absent+dirs:new".into(),
    }
}

/// Issue #780: a write_file card that creates directories says why `a` is missing, and
/// pressing `a` on it names the same reason instead of "a always allows".
#[test]
fn a_directory_creating_write_card_says_why_a_is_not_offered() {
    let reason = "creating directories always asks";
    let dirs = dirs_card("write_file");
    assert_eq!(allow::unavailable_reason(&dirs).as_deref(), Some(reason));
    let text = screen(dirs.clone());
    assert!(
        text.contains(&format!(
            "n deny · Esc cancels the turn (deny) · a not offered: {reason}"
        )),
        "{text}"
    );
    assert!(!text.contains("a always allow"), "{text}");
    // The same call without new directories, and other tools, keep their offer.
    let mut plain_write = dirs_card("write_file");
    plain_write.fingerprint = "/x/new/c.md:absent".into();
    assert_eq!(allow::unavailable_reason(&plain_write), None);
    assert!(screen(plain_write.clone()).contains("a always allow"));
    assert_eq!(allow::dirs_refusal(&plain_write), None);
    let mut edit = dirs_card("edit_file");
    edit.fingerprint = "/x/new/c.md:absent+dirs:new".into();
    assert_eq!(allow::unavailable_reason(&edit), None);
    assert_eq!(allow::dirs_refusal(&edit), None);
    // A HOST reason wins over the directory reason.
    let mut host = dirs_card("write_file");
    host.kind = CardKind::Shell { escalated: true };
    assert_eq!(
        allow::unavailable_reason(&host).as_deref(),
        Some("HOST runs always ask")
    );
    assert_eq!(allow::dirs_refusal(&host), None);
    // Push and over-long commands keep their own key lines and notices.
    let push = plain("git_push", CardKind::Push);
    assert_eq!(allow::unavailable_reason(&push), None);
    assert_eq!(allow::dirs_refusal(&push), None);
    assert_eq!(allow::dirs_refusal(&shell(&"x".repeat(200), false)), None);
    assert_eq!(allow::dirs_refusal(&shell("npm install", true)), None);
    // The notice for `a` names the reason and never promises an always-allow.
    let notice = allow::dirs_refusal(&dirs).unwrap();
    assert_eq!(notice, format!("a not offered: {reason}; press y or n"));
    assert!(!notice.contains("always allows"));
}

/// Issue #780: `a` on an armed, fully shown directory-creating card decides nothing: the
/// card stays pending, no rule is added, and `y` still approves.
#[test]
fn a_on_a_directory_creating_card_decides_nothing_and_adds_no_rule() {
    let mut session = pending(dirs_card("write_file"));
    let armed = Instant::now() + approval::ARM + Duration::from_millis(1);
    assert!(session.approval_key_answers(armed));
    assert_eq!(session.answer_approval('a', armed), None);
    assert!(session.pending_approval().is_some(), "card still pending");
    let rules = rules_for_test();
    assert!(rules.add_for(&dirs_card("write_file")).is_err());
    assert!(rules.matching(&dirs_card("write_file")).is_none());
    assert_eq!(session.answer_approval('y', armed), Some(Decision::Approve));
}

/// Issue #780: the notice for a key that decided nothing, every branch with its exact text.
#[test]
fn the_undecided_key_notice_names_the_directory_reason_only_for_a_on_that_card() {
    let generic =
        "Read the approval card first; press y or n again (a always allows, not for push)";
    let dirs = dirs_card("write_file");
    assert_eq!(
        allow::undecided_notice('a', Some(&dirs)),
        "a not offered: creating directories always asks; press y or n"
    );
    for key in ['y', 'n', 'x'] {
        assert_eq!(allow::undecided_notice(key, Some(&dirs)), generic, "{key}");
    }
    let mut host = dirs_card("write_file");
    host.kind = CardKind::Shell { escalated: true };
    for card in [
        shell("npm install", true),
        host,
        shell("npm install", false),
        plain("git_push", CardKind::Push),
        plain("edit_file", CardKind::Edit),
    ] {
        assert_eq!(allow::undecided_notice('a', Some(&card)), generic);
    }
    assert_eq!(allow::undecided_notice('a', None), generic);
}
