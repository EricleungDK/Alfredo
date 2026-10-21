//! Issue #302: Esc, /stop, a chat switch or a retry abort the turn that runs a
//! `run_command`; the command (sandboxed or HOST) must die with its process group, the
//! approval log must say `cancelled`, and nothing the command would have done later
//! may happen. Kept apart from `chat_shell.rs` because the quit test cancels every
//! live command in the process and so must not share a binary with other runs.
mod ollama_fixture;

use alfredo_tui::{
    chat_tools::{
        self,
        approval::{self, Decision, Gate, Log, Outcome},
    },
    model::{Event, Session, Update},
    provider::Ollama,
};
use ollama_fixture::{done, Reply};
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
/// Tests here share the process-wide command registry; run them one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-cancel-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

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

/// Live (non-zombie) processes whose command line contains `token`.
fn live_with(token: &str) -> Vec<String> {
    let mut found = Vec::new();
    for entry in fs::read_dir("/proc").unwrap().flatten() {
        let pid = entry.file_name().to_string_lossy().into_owned();
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(raw) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if !String::from_utf8_lossy(&raw).contains(token) {
            continue;
        }
        let stat = fs::read_to_string(entry.path().join("stat")).unwrap_or_default();
        if !stat.contains(") Z ") {
            found.push(format!("{pid}: {}", String::from_utf8_lossy(&raw)));
        }
    }
    found
}

fn approving_gate(log: &Log) -> Gate {
    let (sender, mut receiver) = mpsc::channel::<approval::Request>(4);
    tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            let _ = request.reply.send(Decision::Approve);
        }
    });
    Gate::new(sender, Some(log.clone()))
}

fn tool_call(arguments: serde_json::Value) -> Reply {
    Reply::Stream(
        vec![
            format!(
                "{}\n",
                json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"run_command","arguments":arguments}}]},"done":false})
            )
            .into_bytes(),
            format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )
}

/// Start a turn whose model asks for `command`, approve it, and return once the
/// command is running (it touches `started`).
struct Running {
    handle: tokio::task::JoinHandle<()>,
    root: PathBuf,
    log: Log,
    token: String,
    /// Tool results the turn reported.
    results: Arc<Mutex<Vec<String>>>,
    _fixture: ollama_fixture::Fixture,
    _dir: PathBuf,
}

async fn start(escalate: bool, seconds: u32) -> Running {
    let root = workspace("run");
    let dir = workspace("log");
    let log = Log::new(&dir);
    let token = format!(
        "CANCEL_TOKEN_{}_{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    );
    let command = format!(": {token}; echo $$ > pid; touch started; sleep {seconds}; touch marker");
    let addr = ollama_fixture::reserve();
    let first = Mutex::new(Some(tool_call(
        json!({"command": command, "escalate": escalate}),
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
    let (sender, receiver) = mpsc::channel::<Event>(256);
    // Keep the receiver alive and drained, as the terminal does.
    let results = Arc::new(Mutex::new(Vec::new()));
    let seen = results.clone();
    tokio::spawn(async move {
        let mut receiver = receiver;
        while let Some(event) = receiver.recv().await {
            if let Update::ToolStep(step) = event.update {
                seen.lock().unwrap().extend(step.results);
            }
        }
    });
    let gate = approving_gate(&log);
    let workspace = root.clone();
    let handle = tokio::spawn(async move {
        chat_tools::run_turn(
            &provider,
            &workspace,
            0,
            1,
            "fixture".into(),
            messages,
            sender,
            Some(gate),
        )
        .await
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    while !root.join("started").exists() {
        assert!(Instant::now() < deadline, "the command never started");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Running {
        handle,
        root,
        log,
        token,
        results,
        _fixture: fixture,
        _dir: dir,
    }
}

async fn assert_cancelled_cleanly(run: &Running, stopped_at: Instant, seconds: u32) {
    // The whole process group is gone about a second after the stop, long before the
    // sleep would end.
    let deadline = stopped_at + Duration::from_secs(2);
    loop {
        let alive = live_with(&run.token);
        if alive.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "still alive after 2s: {alive:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // The log records the outcome on the blocking thread, shortly after the kill.
    let deadline = Instant::now() + Duration::from_secs(5);
    let records = loop {
        let records = run.log.records();
        if !records.is_empty() || Instant::now() > deadline {
            break records;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(records.len(), 1, "{records:?}");
    assert!(
        matches!(records[0].outcome, Outcome::Cancelled),
        "a cancelled run must not be logged as applied: {:?}",
        records[0].outcome
    );
    // Let the sleep run out, then prove the later part of the command never ran.
    tokio::time::sleep(Duration::from_secs(seconds as u64 + 1)).await;
    assert!(
        !run.root.join("marker").exists(),
        "the command ran on after the stop"
    );
    assert!(live_with(&run.token).is_empty());
}

#[tokio::test]
async fn esc_kills_a_sandboxed_run_command_and_logs_cancelled() {
    if !sandbox_works() {
        return;
    }
    let _serial = SERIAL.lock().await;
    let run = start(false, 3).await;
    let stopped_at = Instant::now();
    run.handle.abort();
    assert_cancelled_cleanly(&run, stopped_at, 3).await;
    let _ = fs::remove_dir_all(&run.root);
}

#[tokio::test]
async fn esc_kills_a_host_run_command_and_logs_cancelled() {
    let _serial = SERIAL.lock().await;
    let run = start(true, 3).await;
    let pid = fs::read_to_string(run.root.join("pid")).unwrap();
    assert!(Path::new(&format!("/proc/{}", pid.trim())).exists());
    let stopped_at = Instant::now();
    run.handle.abort();
    assert_cancelled_cleanly(&run, stopped_at, 3).await;
    let _ = fs::remove_dir_all(&run.root);
}

#[tokio::test]
async fn quitting_stops_a_running_host_command_before_exit() {
    let _serial = SERIAL.lock().await;
    let run = start(true, 3).await;
    let stopped_at = Instant::now();
    // What the terminal does on quit: abort the turns, then stop every command and wait.
    run.handle.abort();
    let idle =
        tokio::task::spawn_blocking(|| chat_tools::stop_running_commands(Duration::from_secs(3)))
            .await
            .unwrap();
    assert!(idle, "a command was still running after the wait");
    assert!(stopped_at.elapsed() < Duration::from_secs(2));
    assert!(
        live_with(&run.token).is_empty(),
        "the process group outlived quit"
    );
    // The wait covers the logging too.
    let records = run.log.records();
    assert!(
        matches!(records[0].outcome, Outcome::Cancelled),
        "{records:?}"
    );
    assert!(!run.root.join("marker").exists());
    let _ = fs::remove_dir_all(&run.root);
}

#[tokio::test]
async fn a_stopped_run_reports_error_cancelled_to_the_model() {
    let _serial = SERIAL.lock().await;
    let run = start(true, 5).await;
    // Stop the command but let the turn go on, as a quit that is still winding down does.
    let idle =
        tokio::task::spawn_blocking(|| chat_tools::stop_running_commands(Duration::from_secs(3)))
            .await
            .unwrap();
    assert!(idle);
    let _ = tokio::time::timeout(Duration::from_secs(5), run.handle).await;
    let results = run.results.lock().unwrap().clone();
    assert!(
        results.iter().any(|r| r.starts_with("error: cancelled")),
        "{results:?}"
    );
    let _ = fs::remove_dir_all(&run.root);
}
