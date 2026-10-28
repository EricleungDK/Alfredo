//! Issue #517: the bubblewrap sandbox must drop every capability. Run as root, bwrap
//! otherwise keeps CAP_SYS_ADMIN inside its user namespace, so a sandboxed command can
//! remount the read-only `.git` bind read-write and write through it to the host.
//! The argv-shape checks live in-crate (`src/execution.rs` tests), because the argv
//! builders are crate-private.
use alfredo_tui::{chat_tools, model::ToolCall};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static ID: AtomicU64 = AtomicU64::new(0);

/// Removes a test workspace when the test ends, pass or fail.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A Coding Workspace with a `.git` directory, as the chat sandbox binds it read-only.
fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-cap-drop-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join(".git/hooks")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    root.canonicalize().unwrap()
}

/// One sandboxed `run_command` through the real chat tool path (no approval gate runs
/// here; `dispatch` is what an approved call reaches).
fn run_command(root: &Path, command: &str) -> String {
    chat_tools::dispatch(
        root,
        &ToolCall {
            name: "run_command".into(),
            arguments: json!({ "command": command }),
        },
    )
}

/// True when bubblewrap can start a sandbox here; the tests skip otherwise.
fn sandbox_works() -> bool {
    let ok = std::process::Command::new("/usr/bin/bwrap")
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
        .is_ok_and(|status| status.success());
    if !ok {
        eprintln!("SKIP: bubblewrap unavailable; sandboxed run_command not exercised");
    }
    ok
}

fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn a_sandboxed_command_keeps_its_workspace_writable() {
    if !sandbox_works() {
        return;
    }
    let root = workspace("writes");
    let _cleanup = Cleanup(root.clone());
    let output = run_command(
        &root,
        "echo sandbox-ran && echo made > inside.txt && cat inside.txt",
    );
    assert!(
        output.contains("sandbox-ran"),
        "the command did not run: {output}"
    );
    assert_eq!(
        fs::read_to_string(root.join("inside.txt")).unwrap().trim(),
        "made",
        "{output}"
    );
}

#[test]
fn a_sandboxed_command_cannot_remount_the_git_bind_writable_as_root() {
    if !is_root() {
        eprintln!("SKIP: the capability escape needs uid 0 (CapEff is empty for other users)");
        return;
    }
    if !sandbox_works() {
        return;
    }
    let root = workspace("remount");
    let _cleanup = Cleanup(root.clone());
    let git = root.join(".git");
    let output = run_command(
        &root,
        &format!(
            "echo sandbox-ran; mount -o remount,rw,bind {git} {git}; touch .git/escaped; echo done",
            git = git.display()
        ),
    );
    // The marker proves the command ran inside the sandbox, so the checks below are
    // not satisfied by a sandbox that never started.
    assert!(
        output.contains("sandbox-ran"),
        "the command did not run: {output}"
    );
    assert!(
        !root.join(".git/escaped").exists(),
        "a sandboxed command wrote through a remounted .git bind: {output}"
    );
}
