//! Issue #753: a startup launch error keeps the selection-history pointer, except a busy
//! conversation, which is reported plainly. `cli_support::launch_error_message` is
//! crate-private, so this drives the binary on a pseudo-terminal (the launch path needs one).
mod ollama_fixture;

use ollama_fixture::{done, Fixture};
use std::{
    fs,
    os::fd::{FromRawFd, OwnedFd},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const POINTER: &str = "Inspect selection history at";

fn repository(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-launch-errors-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("a.txt"), "a\n").unwrap();
    for args in [
        &["init", "-q", "--template=", "--initial-branch=main"][..],
        &["config", "user.name", "Fixture"],
        &["config", "user.email", "fixture@example.invalid"],
        &["add", "-A"],
        &["-c", "commit.gpgSign=false", "commit", "-qm", "base"],
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap();
        assert!(status.success());
    }
    root
}

fn model_server() -> Fixture {
    ollama_fixture::serve(ollama_fixture::reserve(), |request, _| {
        if request.path == "POST /api/chat" {
            done("ok")
        } else {
            ollama_fixture::running(&["fixture"])
        }
    })
}

fn pty() -> (OwnedFd, OwnedFd) {
    let (mut master, mut slave) = (0, 0);
    // SAFETY: openpty fills two descriptors that we then own.
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
    // SAFETY: both descriptors are fresh and owned by nobody else.
    unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) }
}

/// Opens the workspace and mission through the interactive launch path: stdin and stdout
/// are terminals, so `--workspace` with `--mission` skips the selector and launches at once.
fn interactive_launch(root: &Path, fixture: &Fixture, mission: &[&str]) -> Output {
    let (_master, slave) = pty();
    Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--model", "fixture", "--endpoint", &fixture.endpoint])
        .args(["--connect-retries", "0"])
        .args(mission)
        .arg("--workspace")
        .arg(root.join("workspace"))
        .arg("--state-dir")
        .arg(root.join("state"))
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave))
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn launch_errors_keep_the_selection_history_pointer_except_a_busy_conversation() {
    let fixture = model_server();

    // A genuine failure: the selection journal cannot be opened.
    let root = repository("genuine");
    fs::create_dir_all(root.join("state")).unwrap();
    fs::write(root.join("state/rust-selection-v1"), "not a directory").unwrap();
    let genuine = interactive_launch(&root, &fixture, &["--new-mission", "alpha"]);
    assert_eq!(genuine.status.code(), Some(1), "{}", stderr(&genuine));
    let message = stderr(&genuine);
    assert!(message.contains(POINTER), "{message}");
    assert!(
        message.contains("rust-selection-v1/selections.json"),
        "{message}"
    );
    let _ = fs::remove_dir_all(&root);

    // A busy conversation: another owner holds the lock, so the refusal is plain.
    let root = repository("busy");
    let warm = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--model", "fixture", "--endpoint", &fixture.endpoint])
        .args(["--chat", "hi", "--headless"])
        .arg("--workspace")
        .arg(root.join("workspace"))
        .arg("--state-dir")
        .arg(root.join("state"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(warm.status.code(), Some(0), "{}", stderr(&warm));
    let mission_directory = fs::read_dir(root.join("state/rust-tasks-v1"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("mission.json").exists())
        .expect("the warm-up run created the mission directory");
    let digest = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(b"default"))
    };
    let owner = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(mission_directory.join(format!("conversations-{digest}.lock")))
        .unwrap();
    owner.try_lock().expect("nobody else holds it yet");
    let busy = interactive_launch(&root, &fixture, &["--mission", "default"]);
    let message = stderr(&busy);
    assert_eq!(busy.status.code(), Some(1), "{message}");
    assert!(message.contains("already owns conversation"), "{message}");
    assert!(!message.contains(POINTER), "{message}");
    drop(owner);
    let _ = fs::remove_dir_all(&root);
}
