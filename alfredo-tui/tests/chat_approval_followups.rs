//! Issue #235 follow-ups: arming starts when the card is seen, an approval aborted
//! before it runs is still logged, concurrent log appends lose nothing, and symlink
//! swaps after the preview are refused.
mod ollama_fixture;

use alfredo_tui::{
    chat_tools::{
        self,
        approval::{self, Decision, Gate, Log, Outcome},
    },
    model::{Session, ToolCall, Update},
    provider::Ollama,
};
use ollama_fixture::{done, Reply};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
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
        "alfredo-approval-fu-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), README).unwrap();
    root
}

fn edit_call() -> ToolCall {
    ToolCall {
        name: "edit_file".into(),
        arguments: json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "New intro line."}),
    }
}

fn card(root: &std::path::Path) -> approval::Card {
    approval::card(root, &edit_call()).unwrap()
}

fn waiting_session(root: &std::path::Path) -> Session {
    let mut session = Session::new("m".into());
    session.insert("redraft README");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    assert!(session.await_approval(attempt, card(root), Instant::now()));
    session
}

#[test]
fn arming_starts_when_the_card_is_first_visible_and_restarts_after_hiding() {
    let root = workspace("arm");
    let mut session = waiting_session(&root);
    let received = Instant::now();
    let later = received + approval::ARM * 4;
    // Never shown (owner typing in another session): keys decide nothing.
    assert_eq!(session.answer_approval('y', later), None);
    session.approval_visible(true, later);
    assert_eq!(session.answer_approval('y', later), None, "just shown");
    // Hidden again (switched away), then shown: the arm restarts.
    session.approval_visible(false, later + approval::ARM);
    let back = later + approval::ARM * 3;
    session.approval_visible(true, back);
    assert_eq!(
        session.answer_approval('y', back + Duration::from_millis(100)),
        None
    );
    // Staying visible keeps the first visible time.
    session.approval_visible(true, back + approval::ARM);
    assert_eq!(
        session.answer_approval('y', back + approval::ARM + Duration::from_millis(1)),
        Some(Decision::Approve)
    );
    let _ = fs::remove_dir_all(root);
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

#[tokio::test]
async fn an_approval_aborted_before_it_runs_is_logged_once() {
    let root = workspace("abort");
    let logs = workspace("abort-log");
    let log = Log::new(&logs);
    let addr = ollama_fixture::reserve();
    let first = Mutex::new(Some(tool_call("edit_file", edit_call().arguments)));
    let fixture = ollama_fixture::serve(addr, move |_, index| {
        if index == 0 {
            first.lock().unwrap().take().unwrap()
        } else {
            done("done")
        }
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let (requests, mut incoming) = mpsc::channel::<approval::Request>(4);
    let (events, mut sink) = mpsc::channel(256);
    let handle: Arc<Mutex<Option<tokio::task::AbortHandle>>> = Default::default();
    let abort = handle.clone();
    // y then Esc before the turn is polled again: reply sent, then the job aborted.
    let owner = tokio::spawn(async move {
        let request = incoming.recv().await.unwrap();
        let _ = request.reply.send(Decision::Approve);
        abort.lock().unwrap().take().unwrap().abort();
    });
    let gate = Gate::new(requests, Some(log.clone()));
    let workspace = root.clone();
    let job = tokio::spawn(async move {
        chat_tools::run_turn(
            &provider,
            &workspace,
            0,
            1,
            "fixture".into(),
            vec![alfredo_tui::model::Message {
                role: "user".into(),
                content: "redraft README".into(),
            }],
            events,
            Some(gate),
        )
        .await
    });
    *handle.lock().unwrap() = Some(job.abort_handle());
    owner.await.unwrap();
    assert!(job.await.unwrap_err().is_cancelled());
    while sink.try_recv().is_ok() {}
    let records = log.records();
    assert_eq!(records.len(), 1, "{records:?}");
    assert!(matches!(&records[0].outcome, Outcome::Failed(reason) if reason.contains("cancelled")));
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(logs);
}

#[test]
fn concurrent_appends_lose_no_records() {
    let root = workspace("concurrent");
    let log = Log::new(&root);
    let card = card(&root);
    let per = approval::MAX_LOG + 100;
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let (log, card) = (log.clone(), card.clone());
            std::thread::spawn(move || {
                for _ in 0..per {
                    log.append(&card, &Outcome::Denied);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    // Serialized: 2*MAX_LOG records, one rotation to MAX_LOG, then the rest.
    let total = 2 * per;
    let expected = approval::MAX_LOG + (total - 2 * approval::MAX_LOG - 1);
    assert_eq!(log.records().len(), expected);
    let leftovers: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn a_symlink_swapped_in_after_the_preview_is_refused() {
    let root = workspace("swap");
    fs::write(root.join("other.md"), README).unwrap();
    let card = card(&root);
    fs::remove_file(root.join("README.md")).unwrap();
    std::os::unix::fs::symlink(root.join("other.md"), root.join("README.md")).unwrap();
    let result = approval::apply(&root, &card);
    assert!(result.is_err(), "{result:?}");
    assert_eq!(fs::read_to_string(root.join("other.md")).unwrap(), README);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_planted_at_a_new_path_is_refused() {
    let root = workspace("dangling");
    let outside = workspace("outside");
    std::os::unix::fs::symlink(outside.join("planted.md"), root.join("new.md")).unwrap();
    let call = ToolCall {
        name: "write_file".into(),
        arguments: json!({"path": "new.md", "content": "x\n"}),
    };
    assert!(approval::card(&root, &call).is_err());
    assert!(chat_tools::dispatch(&root, &call).starts_with("error"));
    assert!(!outside.join("planted.md").exists());
    // Planted after an "absent" preview: refused at approval too.
    let fresh = ToolCall {
        name: "write_file".into(),
        arguments: json!({"path": "fresh.md", "content": "x\n"}),
    };
    let card = approval::card(&root, &fresh).unwrap();
    std::os::unix::fs::symlink(outside.join("late.md"), root.join("fresh.md")).unwrap();
    assert!(approval::apply(&root, &card).is_err());
    assert!(!outside.join("late.md").exists());
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}
