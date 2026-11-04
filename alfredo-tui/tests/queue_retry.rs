//! Issue #584: queueing the owner's `/repair ID retry` waits for the startup task
//! refresh first, because a command is refused while that refresh is pending.
use alfredo_tui::{provider::Ollama, tasks::TaskStore, workstation::Workstation};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn open(tag: &str) -> (Root, Workstation) {
    let root =
        std::env::temp_dir().join(format!("alfredo-queue-retry-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("repo")).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q", "--template="])
        .arg(root.join("repo"))
        .status()
        .unwrap()
        .success());
    TaskStore::new(&root.join("state"), &root.join("repo"), "repo")
        .unwrap()
        .select_mission(true)
        .unwrap();
    let work = Workstation::open(
        &root.join("state"),
        &root.join("repo"),
        "repo",
        "default",
        "fixture",
        Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    (Root(root), work)
}

#[test]
fn the_retry_waits_for_the_pending_refresh_and_is_then_judged_on_its_own() {
    let (_root, mut work) = open("wait");
    let runtime = Runtime::new().unwrap();
    work.tasks.refresh(&runtime);
    assert!(work.tasks.pending, "the refresh is in flight");
    let started = Instant::now();
    let (origin, command) = work
        .queue_retry(99, "fixture", Duration::from_secs(30))
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "it stops waiting once the refresh landed"
    );
    assert!(!work.tasks.pending);
    assert_eq!(origin, work.app.selected);
    let saved = work.app.sessions[origin]
        .commands()
        .iter()
        .find(|saved| saved.id == command)
        .expect("the retry is recorded in the conversation");
    assert_eq!(saved.text, "/repair 99 retry");
}

#[test]
fn the_wait_gives_up_after_its_patience() {
    let (_root, mut work) = open("patience");
    let runtime = Runtime::new().unwrap();
    work.tasks.refresh(&runtime);
    // No time to wait: the refresh is still pending, so the command is refused as such.
    let error = work.queue_retry(99, "fixture", Duration::ZERO).unwrap_err();
    assert!(error.contains("pending"), "{error}");
    assert!(work.tasks.pending);
}
