//! Issue #236: run_command runs approved shell commands in the bubblewrap Sandbox
//! (Coding Workspace writable, network off); `escalate` runs on the host after a
//! visibly different approval card. Fixture-driven demos: "run the tests" and
//! "push this branch".
mod ollama_fixture;

use alfredo_tui::{
    activity,
    chat_tools::{
        self,
        approval::{self, CardKind, Decision, Gate, Log, Outcome},
        Tier, TOOLS,
    },
    model::{App, Event, Session, ToolCall, Update},
    provider::Ollama,
    ui,
};
use ollama_fixture::{done, Reply};
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

static ID: AtomicU64 = AtomicU64::new(0);

fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-shell-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("test.sh"),
        "echo TESTS_PASSED\necho made > out.txt\n",
    )
    .unwrap();
    root.canonicalize().unwrap()
}

/// True when bubblewrap can start a sandbox here; tests needing it skip otherwise.
fn sandbox_works() -> bool {
    let ok = Command::new("/usr/bin/bwrap")
        .args([
            "--unshare-user",
            "--unshare-net",
            "--ro-bind",
            "/",
            "/",
            "--",
            "/bin/true",
        ])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("SKIP: bubblewrap unavailable; sandboxed run_command not exercised");
    }
    ok
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

fn call(arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        name: "run_command".into(),
        arguments,
    }
}

fn gate(decision: Decision, log: Option<Log>) -> (Gate, Arc<Mutex<Vec<approval::Card>>>) {
    let (sender, mut receiver) = mpsc::channel::<approval::Request>(4);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let cards = seen.clone();
    tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            cards.lock().unwrap().push(request.card.clone());
            assert_eq!(
                request.escalated(),
                matches!(request.card.kind, CardKind::Shell { escalated: true })
            );
            let _ = request.reply.send(decision);
        }
    });
    (Gate::new(sender, log), seen)
}

/// One turn: the model asks for `first`, then answers "done". Returns the tool
/// result the model got back.
async fn turn(root: &Path, first: Reply, gate: Gate) -> (Session, String) {
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
    session.insert("run it");
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
    while let Some(event) = receiver.recv().await {
        session.apply(event.attempt, event.update);
    }
    let bodies: Vec<serde_json::Value> = fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path == "POST /api/chat")
        .map(|r| r.body.clone())
        .collect();
    let result = bodies
        .get(1)
        .and_then(|b| b["messages"].as_array())
        .and_then(|m| m.last())
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string();
    (session, result)
}

#[test]
fn run_command_is_registered_mutating_with_an_escalate_flag() {
    let tool = TOOLS.iter().find(|t| t.name == "run_command").unwrap();
    assert_eq!(tool.tier, Tier::Mutating);
    let definition = chat_tools::definitions()
        .into_iter()
        .find(|d| d["function"]["name"] == "run_command")
        .unwrap();
    let properties = &definition["function"]["parameters"]["properties"];
    assert_eq!(properties["escalate"]["type"], "boolean");
    assert_eq!(properties["command"]["type"], "string");
    assert!(properties["cwd"].is_object());
    assert!(properties["timeout_seconds"].is_object());
}

#[test]
fn cards_show_command_cwd_and_sandbox_or_host() {
    let root = workspace("cards");
    fs::create_dir(root.join("sub")).unwrap();
    let card =
        approval::card(&root, &call(json!({"command": "sh test.sh", "cwd": "sub"}))).unwrap();
    assert_eq!(card.kind, CardKind::Shell { escalated: false });
    assert!(!card.escalated());
    assert!(card.summary.contains("sh test.sh"), "{}", card.summary);
    assert!(card.preview.contains("cwd sub"), "{}", card.preview);
    assert!(card.preview.contains("sandboxed"), "{}", card.preview);
    assert!(!card.preview.contains("HOST"));
    let host = approval::card(
        &root,
        &call(json!({"command": "git push", "escalate": true})),
    )
    .unwrap();
    assert_eq!(host.kind, CardKind::Shell { escalated: true });
    assert!(host.escalated());
    assert!(
        host.preview.contains("HOST · network on"),
        "{}",
        host.preview
    );
    assert!(host.preview.contains("cwd ."), "{}", host.preview);
    // Refused before asking: empty command, cwd outside the workspace, bad timeout.
    for bad in [
        json!({"command": "  "}),
        json!({"command": "ls", "cwd": "../"}),
        json!({"command": "ls", "cwd": "missing"}),
        json!({"command": "ls", "escalate": "yes"}),
        json!({"command": "ls", "timeout_seconds": 0}),
        json!({"command": "ls", "timeout_seconds": 100000}),
    ] {
        assert!(approval::card(&root, &call(bad.clone())).is_err(), "{bad}");
    }
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn run_the_tests_runs_sandboxed_after_approval() {
    if !sandbox_works() {
        return;
    }
    let root = workspace("tests");
    let dir = workspace("tests-log");
    let log = Log::new(&dir);
    let (gate, cards) = gate(Decision::Approve, Some(log.clone()));
    let (_, result) = turn(
        &root,
        tool_call("run_command", json!({"command": "sh test.sh"})),
        gate,
    )
    .await;
    assert!(result.contains("TESTS_PASSED"), "{result}");
    assert!(result.contains("exit code 0"), "{result}");
    assert!(result.contains("sandboxed"), "{result}");
    assert_eq!(fs::read_to_string(root.join("out.txt")).unwrap(), "made\n");
    assert_eq!(
        cards.lock().unwrap()[0].kind,
        CardKind::Shell { escalated: false }
    );
    let records = log.records();
    assert!(matches!(records[0].outcome, Outcome::Applied));
    assert!(!records[0].escalated);
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_sandbox_keeps_network_off_and_the_host_read_only() {
    if !sandbox_works() {
        return;
    }
    let root = workspace("walls");
    let card = approval::card(
        &root,
        &call(
            json!({"command": "cat /proc/net/dev | grep -c : ; touch /etc/alfredo-x; echo after"}),
        ),
    )
    .unwrap();
    let result = approval::apply(&root, &card).unwrap();
    // Only the loopback interface exists inside the network namespace.
    assert!(
        result.contains("\n1\n") || result.contains("stdout:\n1\n"),
        "{result}"
    );
    assert!(result.contains("after"), "{result}");
    let card = approval::card(&root, &call(json!({"command": "exit 3"}))).unwrap();
    let result = approval::apply(&root, &card).unwrap();
    assert!(result.contains("exit code 3"), "{result}");
    // A failing sandboxed command hints at escalation instead of retrying on host.
    assert!(result.contains("escalate"), "{result}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn push_this_branch_runs_on_host_after_approval_and_is_logged() {
    let base = workspace("push");
    let remote = base.join("remote.git");
    let repo = base.join("repo");
    let git = |dir: &Path, args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(status.status.success(), "{args:?} {status:?}");
    };
    git(&base, &["init", "-q", "--bare", "remote.git"]);
    git(&base, &["init", "-q", "-b", "feature", "repo"]);
    fs::write(repo.join("a.txt"), "a\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "a"]);
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let dir = workspace("push-log");
    let log = Log::new(&dir);
    let (gate, cards) = gate(Decision::Approve, Some(log.clone()));
    let (_, result) = turn(
        &repo,
        tool_call(
            "run_command",
            json!({"command": "git push origin feature", "escalate": true}),
        ),
        gate,
    )
    .await;
    assert!(result.contains("exit code 0"), "{result}");
    assert!(result.contains("host"), "{result}");
    assert_eq!(
        cards.lock().unwrap()[0].kind,
        CardKind::Shell { escalated: true }
    );
    let branches = Command::new("git")
        .args(["branch", "--list", "feature"])
        .current_dir(&remote)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&branches.stdout).contains("feature"));
    let records = log.records();
    assert!(records[0].escalated);
    assert!(matches!(records[0].outcome, Outcome::Applied));
    let entries = activity::chat_entries(&records, "host");
    assert_eq!(entries.len(), 1, "escalated runs are findable in F4");
    let _ = fs::remove_dir_all(base);
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_denied_escalation_runs_nothing_and_the_model_gets_a_refusal() {
    let root = workspace("deny");
    let dir = workspace("deny-log");
    let log = Log::new(&dir);
    let (gate, _) = gate(Decision::Deny, Some(log.clone()));
    let (_, result) = turn(
        &root,
        tool_call(
            "run_command",
            json!({"command": "touch host-marker", "escalate": true}),
        ),
        gate,
    )
    .await;
    assert!(!root.join("host-marker").exists());
    assert!(result.contains("denied"), "{result}");
    assert!(result.contains("nothing"), "{result}");
    let records = log.records();
    assert!(records[0].escalated);
    assert!(matches!(records[0].outcome, Outcome::Denied));
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_timeout_kills_the_whole_process_tree() {
    let root = workspace("timeout");
    let card = approval::card(
        &root,
        &call(json!({
            "command": "sleep 30 & echo $! > child.pid; wait",
            "escalate": true,
            "timeout_seconds": 1
        })),
    )
    .unwrap();
    let started = Instant::now();
    let error = approval::apply(&root, &card).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(error.contains("timed out"), "{error}");
    let pid = fs::read_to_string(root.join("child.pid")).unwrap();
    let alive = Path::new(&format!("/proc/{}", pid.trim())).exists()
        && !fs::read_to_string(format!("/proc/{}/stat", pid.trim()))
            .unwrap_or_default()
            .contains(") Z ");
    assert!(!alive, "the background child must be killed with the group");
    let _ = fs::remove_dir_all(root);
}

/// Issue #304: a host `run_command` child must not share Alfredo's controlling terminal
/// (sudo/ssh/git open `/dev/tty` directly and would draw over the TUI and read its keys).
/// The test re-runs itself as a child whose session has a real controlling tty (a pty
/// slave attached with TIOCSCTTY), because the test harness itself has none.
#[cfg(target_os = "linux")]
#[test]
fn host_command_has_no_controlling_terminal_even_when_alfredo_has_one() {
    use std::os::unix::process::CommandExt;
    const NAME: &str = "host_command_has_no_controlling_terminal_even_when_alfredo_has_one";
    if std::env::var_os("ALFREDO_TEST_CTTY_CHILD").is_some() {
        // Sanity: this process really has a controlling terminal.
        assert!(
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")
                .is_ok(),
            "the re-run did not get a controlling terminal"
        );
        let root = workspace("ctty");
        let card = approval::card(
            &root,
            &call(json!({
                "command": "ps -o tty= -p $$; echo x > /dev/tty; echo rc=$?; \
                            echo 'ssh: cannot open /dev/tty' >&2; exit 3",
                "escalate": true
            })),
        )
        .unwrap();
        let result = approval::apply(&root, &card).unwrap();
        let _ = fs::remove_dir_all(&root);
        assert!(!result.contains("rc=0"), "/dev/tty was writable: {result}");
        assert!(result.contains("rc="), "{result}");
        assert!(
            result.lines().any(|line| line.trim() == "?"),
            "child has a tty: {result}"
        );
        assert!(!result.contains("pts/"), "child has a tty: {result}");
        assert!(result.contains("exit code 3"), "{result}");
        assert!(result.contains("ssh-agent"), "no terminal hint: {result}");
        return;
    }
    let (mut master, mut slave) = (0, 0);
    // SAFETY: openpty fills two descriptors we then own.
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    assert_eq!(opened, 0, "openpty failed");
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([NAME, "--exact", "--nocapture", "--test-threads=1"])
        .env("ALFREDO_TEST_CTTY_CHILD", "1");
    // SAFETY: only async-signal-safe calls (setsid, ioctl, close) between fork and exec.
    unsafe {
        child.pre_exec(move || {
            libc::setsid();
            if libc::ioctl(slave, libc::TIOCSCTTY as _, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(slave);
            libc::close(master);
            Ok(())
        });
    }
    let status = child.status().unwrap();
    // SAFETY: closing our own descriptors.
    unsafe {
        libc::close(master);
        libc::close(slave);
    }
    assert!(status.success(), "the re-run with a controlling tty failed");
}

#[test]
fn output_is_capped_with_a_truncation_marker() {
    let root = workspace("cap");
    let card = approval::card(
        &root,
        &call(json!({"command": "head -c 200000 /dev/zero | tr '\\0' x", "escalate": true})),
    )
    .unwrap();
    let result = approval::apply(&root, &card).unwrap();
    assert!(
        result.len() <= chat_tools::MAX_OUTPUT + 200,
        "{}",
        result.len()
    );
    assert!(result.contains("[cut"), "{}", &result[result.len() - 100..]);
    assert!(result.contains("exit code 0"));
    let _ = fs::remove_dir_all(root);
}

fn draw(card: approval::Card) -> String {
    let mut session = Session::new("m".into());
    session.insert("run it");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    assert!(session.await_approval(attempt, card, Instant::now()));
    let mut app = App::new("m".into());
    app.sessions[0] = session;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn header_color(card: approval::Card) -> ratatui::style::Color {
    let mut session = Session::new("m".into());
    session.insert("run it");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    assert!(session.await_approval(attempt, card, Instant::now()));
    let mut app = App::new("m".into());
    app.sessions[0] = session;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width as usize;
    let text: Vec<_> = buffer
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();
    let line = (0..buffer.area.height as usize)
        .find(|row| {
            text[row * width..(row + 1) * width]
                .concat()
                .contains("? Approve")
        })
        .unwrap();
    let column = text[line * width..(line + 1) * width]
        .iter()
        .position(|s| s == "?")
        .unwrap();
    buffer.content()[line * width + column].fg
}

#[test]
fn escalated_cards_look_different_from_sandboxed_ones() {
    let root = workspace("ui");
    let sandboxed = approval::card(&root, &call(json!({"command": "sh test.sh"}))).unwrap();
    let host = approval::card(
        &root,
        &call(json!({"command": "git push origin HEAD", "escalate": true})),
    )
    .unwrap();
    let text = draw(sandboxed.clone());
    assert!(text.contains("Approve run_command"), "{text}");
    assert!(text.contains("sh test.sh"));
    assert!(text.contains("sandboxed"));
    assert!(!text.contains("HOST"));
    let text = draw(host.clone());
    assert!(text.contains("HOST · network on"), "{text}");
    assert!(
        text.contains("credentials"),
        "the banner names what escalation exposes"
    );
    assert!(text.contains("git push origin HEAD"));
    assert_ne!(header_color(sandboxed), header_color(host));
    let _ = fs::remove_dir_all(root);
}

/// Issue #305: format (Cf) characters must never reach the terminal from a card or a
/// tool-step row; they show as `<U+XXXX>` so the owner sees the text differs.
const CF: [char; 4] = ['\u{202E}', '\u{200B}', '\u{2066}', '\u{E0041}'];

fn assert_no_cf(text: &str) {
    for c in CF.into_iter().chain(['\u{FEFF}', '\u{2069}']) {
        assert!(!text.contains(c), "raw U+{:04X} drawn: {text}", c as u32);
    }
}

fn draw_steps(step: alfredo_tui::model::ToolStep) -> String {
    let mut session = Session::new("m".into());
    session.insert("run it");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    session.apply(attempt, Update::ToolStep(step));
    session.apply(attempt, Update::Token("done".into()));
    session.apply(attempt, Update::Done);
    let mut app = App::new("m".into());
    app.sessions[0] = session;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn a_run_command_card_never_draws_format_characters() {
    let root = workspace("cf-run");
    let command = "echo \u{202E}fdsa\u{200B}x \u{2066}y\u{E0041}z";
    let card = approval::card(&root, &call(json!({"command": command}))).unwrap();
    let text = draw(card);
    assert_no_cf(&text);
    for shown in ["<U+202E>", "<U+200B>", "<U+2066>", "<U+E0041>"] {
        assert!(text.contains(shown), "{shown} missing: {text}");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn file_and_git_cards_never_draw_format_characters() {
    let root = workspace("cf-edit");
    let mut card = approval::card(&root, &call(json!({"command": "true"}))).unwrap();
    card.kind = CardKind::Edit;
    card.tool = "write_file".into();
    card.summary = "write n\u{202E}ew\u{200B}.txt".into();
    card.preview = "+x\u{2066}y\u{E0041}\n-o\u{202E}ld".into();
    card.arguments = json!({"path": "n\u{202E}ew\u{200B}.txt", "content": "x\u{2066}y"});
    let text = draw(card.clone());
    assert_no_cf(&text);
    assert!(text.contains("<U+202E>"), "{text}");
    card.tool = "git_commit".into();
    card.kind = CardKind::Git;
    card.summary = "commit \u{202E}fix\u{E0041}".into();
    let text = draw(card);
    assert_no_cf(&text);
    assert!(text.contains("<U+E0041>"), "{text}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn tool_step_rows_never_draw_format_characters() {
    let text = draw_steps(alfredo_tui::model::ToolStep {
        calls: vec![ToolCall {
            name: "run_command".into(),
            arguments: json!({"command": "echo \u{202E}fdsa\u{200B}x \u{2066}y\u{E0041}z"}),
        }],
        results: vec!["ok \u{202E}rev\u{FEFF}".into()],
    });
    assert_no_cf(&text);
    assert!(text.contains("<U+202E>"), "{text}");
}

#[test]
fn cards_and_rows_keep_cjk_accents_and_emoji() {
    let root = workspace("cf-plain");
    let card = approval::card(&root, &call(json!({"command": "echo 你好 café 🙂"}))).unwrap();
    let text = draw(card);
    assert!(text.contains("café"), "{text}");
    assert!(text.contains('🙂') && text.contains('你'), "{text}");
    assert!(!text.contains("<U+"), "{text}");
    let text = draw_steps(alfredo_tui::model::ToolStep {
        calls: vec![ToolCall {
            name: "run_command".into(),
            arguments: json!({"command": "echo 你好 café 🙂"}),
        }],
        results: vec!["你好 café 🙂".into()],
    });
    assert!(
        text.contains('🙂') && text.contains("café") && text.contains('你'),
        "{text}"
    );
    assert!(!text.contains("<U+"), "{text}");
    let _ = fs::remove_dir_all(root);
}

/// Issue #302: while an approved command runs the chat says so and that Esc stops it.
#[test]
fn a_running_command_shows_its_status_and_that_esc_stops_it() {
    let root = workspace("running");
    let card = approval::card(&root, &call(json!({"command": "sleep 5"}))).unwrap();
    let mut session = Session::new("m".into());
    session.insert("run it");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    let shown = Instant::now();
    assert!(session.await_approval(attempt, card, shown));
    session.approval_visible(true, shown);
    let armed = shown + approval::ARM + Duration::from_millis(1);
    assert_eq!(session.answer_approval('y', armed), Some(Decision::Approve));
    let mut app = App::new("m".into());
    app.sessions[0] = session;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("running run_command"), "{text}");
    assert!(text.contains("Esc stops"), "{text}");
    assert!(!text.contains("waiting for model"), "{text}");
    // The next update (the result arriving) ends the status.
    app.sessions[0].apply(attempt, Update::Thinking);
    assert_ne!(app.sessions[0].wait_phase(), Some("running run_command"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn chat_system_prompt_mentions_run_command_and_escalation() {
    let prompt = alfredo_tui::chat_context::system_message(None, |_| None).content;
    assert!(prompt.contains("run_command"), "{prompt}");
    assert!(prompt.contains("escalate"), "{prompt}");
    assert!(!prompt.contains("cannot run commands"), "{prompt}");
}

#[tokio::test]
async fn timeout_errors_reaching_the_model_are_capped() {
    let root = workspace("errcap");
    let (gate, _) = gate(Decision::Approve, None);
    let (_, result) = turn(
        &root,
        tool_call(
            "run_command",
            json!({"command": "head -c 300000 /dev/zero | tr '\\0' x; sleep 30", "escalate": true, "timeout_seconds": 1}),
        ),
        gate,
    )
    .await;
    assert!(
        result.contains("timed out"),
        "{}",
        &result[..200.min(result.len())]
    );
    assert!(
        result.len() <= chat_tools::MAX_OUTPUT + 200,
        "{}",
        result.len()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stderr_and_output_tails_survive_long_stdout() {
    let root = workspace("tails");
    let card = approval::card(
        &root,
        &call(json!({
            "command": "head -c 100000 /dev/zero | tr '\\0' x; echo STDOUT_END; head -c 50000 /dev/zero | tr '\\0' y >&2; echo ERR_TAIL >&2",
            "escalate": true
        })),
    )
    .unwrap();
    let result = approval::apply(&root, &card).unwrap();
    assert!(
        result.len() <= chat_tools::MAX_OUTPUT + 200,
        "{}",
        result.len()
    );
    assert!(result.starts_with("exit code 0"), "{}", &result[..100]);
    assert!(result.contains("STDOUT_END"));
    assert!(result.contains("ERR_TAIL"));
    assert!(result.contains("[cut"), "truncation is marked");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sandboxed_cargo_works_when_a_rustup_toolchain_exists() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    if !home.join(".cargo/bin/cargo").exists() || !sandbox_works() {
        eprintln!("SKIP: no ~/.cargo toolchain");
        return;
    }
    let root = workspace("cargo");
    let card = approval::card(&root, &call(json!({"command": "cargo --version"}))).unwrap();
    let result = approval::apply(&root, &card).unwrap();
    assert!(result.starts_with("exit code 0"), "{result}");
    assert!(result.contains("cargo "), "{result}");
    let card = approval::card(&root, &call(json!({"command": "echo $CARGO_NET_OFFLINE"}))).unwrap();
    assert!(approval::apply(&root, &card).unwrap().contains("true"));
    // Offline build and test of a crate with no dependencies, through the rustup proxies
    // (issue #310: only the toolchain entries are bound, not the whole homes).
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub fn two() -> u8 { 2 }\n#[test]\nfn it_works() { assert_eq!(two(), 2); }\n",
    )
    .unwrap();
    let card = approval::card(
        &root,
        &call(
            json!({"command": "rustc --version && cargo build --offline && cargo test --offline"}),
        ),
    )
    .unwrap();
    let result = approval::apply(&root, &card).unwrap();
    assert!(result.starts_with("exit code 0"), "{result}");
    assert!(result.contains("rustc "), "{result}");
    assert!(result.contains("test result: ok. 1 passed"), "{result}");
    let _ = fs::remove_dir_all(root);
}

/// Marker the planted cargo credentials carry; the sandbox must never print it.
const CARGO_SECRET: &str = "SECRET_TOKEN_310";
/// Set in the child process of [`sandbox_cannot_read_cargo_credentials`] to the fake
/// home it runs with (`HOME` is read at call time, so the parent process is not changed).
const FAKE_HOME_ENV: &str = "ALFREDO_TEST_FAKE_HOME";

/// Plant a fake `$HOME` with cargo credentials (issue #310). It lives under the cargo
/// target tmpdir, not `/tmp`: the sandbox mounts a tmpfs over `/tmp`, which would hide it.
fn fake_home(tag: &str) -> PathBuf {
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "fake-home-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&home);
    let cargo = home.join(".cargo");
    fs::create_dir_all(cargo.join("registry/index")).unwrap();
    fs::create_dir_all(cargo.join("bin")).unwrap();
    fs::write(cargo.join("registry/index/marker"), "REGISTRY_OK\n").unwrap();
    for name in [
        "credentials.toml",
        "credentials",
        "config.toml",
        "config",
        "Credentials.toml",
        "anything",
    ] {
        fs::write(cargo.join(name), format!("token = \"{CARGO_SECRET}\"\n")).unwrap();
    }
    // A symlinked entry that points at a secret elsewhere must not be bound through.
    fs::create_dir_all(home.join("secrets")).unwrap();
    fs::write(home.join("secrets/token"), format!("{CARGO_SECRET}\n")).unwrap();
    std::os::unix::fs::symlink(home.join("secrets"), cargo.join("git")).unwrap();
    home
}

/// The attack from issue #310: a prompt-injected model reads the owner's cargo
/// credentials with a sandboxed `run_command`. The body runs in a child process whose
/// `HOME` is the fake home (see [`FAKE_HOME_ENV`]).
#[test]
fn sandbox_cannot_read_cargo_credentials() {
    if !sandbox_works() {
        return;
    }
    let home = fake_home("creds");
    let planted = fs::read_to_string(home.join(".cargo/credentials.toml")).unwrap();
    assert!(
        planted.contains(CARGO_SECRET),
        "the planted secret is on disk"
    );
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "sandbox_cannot_read_cargo_credentials_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("HOME", &home)
        .env(FAKE_HOME_ENV, &home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn sandbox_cannot_read_cargo_credentials_child() {
    let Some(home) = std::env::var_os(FAKE_HOME_ENV).map(PathBuf::from) else {
        return;
    };
    let cargo = home.join(".cargo");
    let root = workspace("creds");
    let shell = |command: String| {
        let card = approval::card(&root, &call(json!({ "command": command }))).unwrap();
        approval::apply(&root, &card).unwrap()
    };
    for name in [
        "credentials.toml",
        "credentials",
        "config.toml",
        "config",
        "Credentials.toml",
        "anything",
        "git/token",
    ] {
        let result = shell(format!("cat '{}/{name}' 2>&1; echo END", cargo.display()));
        // END proves the command ran; a sandbox failure string must not pass vacuously.
        assert!(result.contains("END"), "{name} did not run: {result}");
        assert!(
            !result.contains(CARGO_SECRET),
            "{name} leaked the planted token"
        );
    }
    // Allowed entries still work: the registry is bound read-only.
    let result = shell(format!("cat '{}/registry/index/marker'", cargo.display()));
    assert!(result.contains("REGISTRY_OK"), "{result}");
    // The cargo home root is not listed either.
    let listing = shell(format!("ls -A '{}'", cargo.display()));
    for name in ["credentials", "config", "anything"] {
        assert!(!listing.contains(name), "{listing}");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn git_works_sandboxed_in_a_linked_worktree() {
    if !sandbox_works() {
        return;
    }
    let base = workspace("linked");
    let git = |dir: &Path, args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?} {out:?}");
    };
    git(&base, &["init", "-q", "-b", "main", "main"]);
    let main = base.join("main");
    fs::write(main.join("a.txt"), "a\n").unwrap();
    git(&main, &["add", "a.txt"]);
    git(&main, &["commit", "-q", "-m", "a"]);
    git(&main, &["worktree", "add", "-q", "-b", "wt", "../wt"]);
    let wt = base.join("wt");
    let card = approval::card(&wt, &call(json!({"command": "git log --oneline -1"}))).unwrap();
    let result = approval::apply(&wt, &card).unwrap();
    assert!(result.starts_with("exit code 0"), "{result}");
    assert!(result.contains(" a"), "{result}");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn a_cwd_swapped_after_the_preview_is_refused() {
    let root = workspace("swap");
    fs::create_dir(root.join("sub")).unwrap();
    fs::create_dir(root.join("other")).unwrap();
    let card = approval::card(
        &root,
        &call(json!({"command": "touch ran", "cwd": "sub", "escalate": true})),
    )
    .unwrap();
    fs::remove_dir(root.join("sub")).unwrap();
    std::os::unix::fs::symlink(root.join("other"), root.join("sub")).unwrap();
    let error = approval::apply(&root, &card).unwrap_err();
    assert!(error.contains("cwd"), "{error}");
    assert!(!root.join("other/ran").exists());
    let _ = fs::remove_dir_all(root);
}
