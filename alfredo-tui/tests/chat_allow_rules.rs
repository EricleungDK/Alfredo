//! Issue #238: always-allow rules. Per mission, by tool or run_command command prefix,
//! persisted in the State Directory; push never; escalated runs need escalated rules;
//! auto-approved calls are still fingerprint-checked and logged with their rule.
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
    assert_eq!(again.list(), vec![edit.clone(), test.clone()]);
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
    assert_eq!(Rules::new(&state).list(), vec![test]);
    allow::slash(&again, "clear").unwrap();
    assert!(Rules::new(&state).list().is_empty());
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
    assert_eq!(rules.list().len(), 3);
    assert_eq!(rules.matching(&push), None);
    // A run_command rule without a command never matches.
    assert_eq!(rules.matching(&shell("ls", false)), None);
    // A git_push card disguised as another kind still never matches a git_push rule.
    assert_eq!(rules.matching(&plain("git_push", CardKind::Git)), None);
    let _ = fs::remove_dir_all(state);
}

#[test]
fn escalated_runs_need_an_explicitly_escalated_rule() {
    let state = dir("escalated");
    let rules = Rules::new(&state);
    let sandboxed = rules.add_for(&shell("npm install", false)).unwrap();
    assert!(!sandboxed.escalated);
    assert_eq!(rules.matching(&shell("npm install", true)), None);
    let host = rules.add_for(&shell("npm install", true)).unwrap();
    assert!(host.escalated);
    assert_eq!(
        rules.matching(&shell("npm install", true)),
        Some(host.clone())
    );
    assert!(
        allow::describe(&host).contains("HOST"),
        "{}",
        allow::describe(&host)
    );
    // An escalated rule does not cover sandboxed runs either.
    rules.remove(sandboxed.id).unwrap();
    assert_eq!(rules.matching(&shell("npm install", false)), None);
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
fn the_card_offers_always_allow_except_for_push_and_says_host_when_escalated() {
    assert!(screen(plain("edit_file", CardKind::Edit)).contains("a always allow"));
    assert!(!screen(plain("git_push", CardKind::Push)).contains("a always allow"));
    let host = screen(shell("npm install", true));
    assert!(host.contains("a always allow on HOST"), "{host}");
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
    let arguments =
        json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "New intro line."});
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
    let addr = ollama_fixture::reserve();
    let first = Mutex::new(Some(edit_call()));
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
    let rule = rules.list().pop().expect("rule created");
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
    fs::write(
        state.join(allow::FILE),
        json!([{"id": 1, "tool": "run_command", "command": long}]).to_string(),
    )
    .unwrap();
    rules.add_for(&plain("edit_file", CardKind::Edit)).unwrap();
    rules.add_for(&shell("echo onhost true", true)).unwrap();
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

#[test]
fn escalated_rules_match_only_the_exact_command() {
    let state = dir("exact-host");
    let rules = Rules::new(&state);
    let host = rules.add_for(&shell("npm install", true)).unwrap();
    assert_eq!(rules.matching(&shell("npm install", true)), Some(host));
    assert_eq!(
        rules.matching(&shell("npm install attacker-pkg", true)),
        None
    );
    assert!(!allow::describe(&rules.list()[0]).contains("longer"));
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
    for escalated in [false, true] {
        let hint = allow::offer(&shell(&at_limit, escalated)).unwrap();
        assert!(hint.contains(&format!("`{at_limit}`")), "{hint}");
    }
    let over = format!("{at_limit}B");
    for escalated in [false, true] {
        let card = shell(&over, escalated);
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
    }
}

fn rules_for_test() -> Rules {
    Rules::new(&dir("scratch"))
}

#[test]
fn the_hint_shows_a_tab_as_an_arrow_and_a_newline_as_a_return_arrow() {
    let tabbed = "echo AAAAAAAAAAAAAAAAAAAA ok;\techo SECOND; echo THIRD";
    let hint = allow::offer(&shell(tabbed, true)).unwrap();
    assert!(hint.contains("→"), "{hint}");
    assert!(hint.contains("echo SECOND; echo THIRD"), "{hint}");
    assert!(
        hint.contains("`echo AAAAAAAAAAAAAAAAAAAA ok;→echo SECOND; echo THIRD`"),
        "{hint}"
    );
    assert!(hint.ends_with(" exactly"), "{hint}");
    // The issue's repro (over 80 shown) is no longer cut: `a` is not offered.
    let long = format!("echo {} ok;\techo SECOND_PART; echo THIRD", "A".repeat(60));
    assert!(allow::offer(&shell(&long, true)).is_none());
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
    card.preview = format!("cwd . · sandboxed · timeout 30s");
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
        for escalated in [false, true] {
            let card = shell_with_preview(command, escalated);
            let offer = allow::offer(&card).unwrap();
            assert!(offer.contains(&format!("`{hint}`")), "{command:?}: {offer}");
            assert_eq!(allow::show_command(command), hint);
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
        let card = shell(command, true);
        assert!(allow::offer(&card).is_none(), "{command}");
        assert!(allow::unavailable_reason(&card)
            .unwrap()
            .contains("looks like an escape"));
    }
}

#[test]
fn the_escalated_card_shows_the_whole_command_and_slash_allow_shows_it_the_same_way() {
    let command = "make\tbuild;\necho done → ok";
    let card = shell(command, true);
    let text = screen(card.clone());
    let shown = allow::show_command(command);
    assert!(text.contains("on HOST (escalated rule)"), "{text}");
    assert!(text.contains(&shown), "{text}");
    let state = dir("same-view");
    let rules = Rules::new(&state);
    let rule = rules.add_for(&card).unwrap();
    assert_eq!(rule.command.as_deref(), Some(command));
    let described = allow::describe(&rule);
    assert!(described.contains(&format!("`{shown}`")), "{described}");
    assert!(allow::offer(&card).unwrap().contains(&format!("`{shown}`")));
    let _ = fs::remove_dir_all(state);
}

#[test]
fn a_sandboxed_prefix_card_says_any_arguments_are_allowed() {
    let text = allow::offer(&shell("cargo test", false)).unwrap();
    assert!(text.contains("ANY arguments"), "{text}");
    assert!(text.contains("sandboxed only"), "{text}");
}
