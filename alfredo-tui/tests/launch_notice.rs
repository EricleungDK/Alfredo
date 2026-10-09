//! The launch footer names the missing first commit only for a Git work tree
//! whose HEAD does not resolve; other workspaces get no such notice.
use alfredo_tui::{provider::Ollama, workstation::Workstation};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::runtime::Runtime;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn notice(init: bool, commit: bool) -> String {
    let root = std::env::temp_dir().join(format!(
        "alfredo-launch-notice-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(args)
            .status()
            .unwrap()
            .success());
    };
    if init {
        git(&["init", "-q", "--template="]);
    }
    if commit {
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--allow-empty",
            "-qm",
            "baseline",
        ]);
    }
    alfredo_tui::tasks::TaskStore::new(&root.join("state"), &workspace, "default")
        .unwrap()
        .select_mission(true)
        .unwrap();
    let runtime = Runtime::new().unwrap();
    let _enter = runtime.enter();
    let work = open(&root.join("state"), &workspace);
    let text = work.app.notice.clone();
    drop(work);
    let _ = fs::remove_dir_all(&root);
    text
}

fn open(state: &Path, workspace: &Path) -> Workstation {
    Workstation::open(
        state,
        workspace,
        "default",
        "default",
        "fixture",
        Ollama::new("http://127.0.0.1:1", Duration::from_secs(1)).unwrap(),
    )
    .unwrap()
}

#[test]
fn repository_without_commits_gets_the_fix_notice() {
    assert!(notice(true, false).contains("git commit --allow-empty"));
}

#[test]
fn non_git_workspace_gets_no_commit_notice() {
    assert_eq!(notice(false, false), "");
}

#[test]
fn committed_repository_gets_no_notice() {
    assert_eq!(notice(true, true), "");
}
