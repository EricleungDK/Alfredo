//! Issue #240: `--chat PROMPT --headless [--approve workspace-edits]` runs one Chat
//! Agent turn without a terminal; scripted approvals answer the Approval gate.
mod ollama_fixture;

use ollama_fixture::{done, scripted_chat, tool_calls, Fixture, Reply};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

fn repository(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-headless-chat-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(
        workspace.join("greet.py"),
        "def greet(name):\n    return 'Hello ' + name\n",
    )
    .unwrap();
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

fn edit_script() -> Vec<Reply> {
    vec![
        tool_calls(&[("grep", json!({"pattern": "def greet"}))]),
        tool_calls(&[("read_file", json!({"path": "greet.py"}))]),
        tool_calls(&[(
            "edit_file",
            json!({"path": "greet.py", "old_text": "'Hello '", "new_text": "'Hi '"}),
        )]),
        done("Changed the greeting to Hi."),
    ]
}

fn run(root: &Path, fixture: &Fixture, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--model", "fixture", "--endpoint", &fixture.endpoint])
        .arg("--workspace")
        .arg(root.join("workspace"))
        .arg("--state-dir")
        .arg(root.join("state"))
        .args(extra)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn result(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim().lines().count(),
        1,
        "one JSON line: {stdout:?}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str(stdout.trim()).unwrap()
}

fn approvals_log(root: &Path) -> Vec<Value> {
    let mut found = Vec::new();
    let mut stack = vec![root.join("state")];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .is_some_and(|n| n == "chat-approvals.jsonl")
            {
                let text = fs::read_to_string(&path).unwrap();
                found.extend(text.lines().map(|l| serde_json::from_str(l).unwrap()));
            }
        }
    }
    found
}

#[test]
fn scripted_policy_approves_workspace_edits_and_logs_them() {
    let root = repository("approve");
    let fixture = scripted_chat(edit_script());
    let output = run(
        &root,
        &fixture,
        &[
            "--chat",
            "Make greet say Hi",
            "--headless",
            "--approve",
            "workspace-edits",
        ],
    );
    let value = result(&output);
    assert_eq!(output.status.code(), Some(0), "{value}");
    assert_eq!(value["state"], "done");
    assert_eq!(value["reply"], "Changed the greeting to Hi.");
    assert_eq!(value["policy"], "workspace-edits");
    let tools: Vec<_> = value["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|call| call["tool"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(tools, ["grep", "read_file", "edit_file"]);
    assert!(value["calls"]
        .as_array()
        .unwrap()
        .iter()
        .all(|call| call["error"] == false));
    assert_eq!(value["approvals"][0]["tool"], "edit_file");
    assert_eq!(value["approvals"][0]["outcome"], "applied");
    assert_eq!(
        fs::read_to_string(root.join("workspace/greet.py")).unwrap(),
        "def greet(name):\n    return 'Hi ' + name\n"
    );
    let log = approvals_log(&root);
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0]["tool"], "edit_file");
    assert_eq!(log[0]["outcome"], "applied");
    // The model saw the tools and the edit result.
    let requests = fixture.requests.lock().unwrap();
    let chats: Vec<_> = requests
        .iter()
        .filter(|r| r.path == "POST /api/chat")
        .collect();
    assert_eq!(chats.len(), 4);
    assert!(chats[0].body["tools"]
        .as_array()
        .is_some_and(|t| !t.is_empty()));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn without_a_policy_every_mutating_call_is_denied_and_logged() {
    let root = repository("deny");
    let fixture = scripted_chat(edit_script());
    let output = run(
        &root,
        &fixture,
        &["--chat", "Make greet say Hi", "--headless"],
    );
    let value = result(&output);
    assert_eq!(output.status.code(), Some(0), "{value}");
    assert_eq!(value["policy"], "deny");
    assert_eq!(value["approvals"][0]["outcome"], "denied");
    assert_eq!(value["calls"][2]["error"], true);
    assert_eq!(
        fs::read_to_string(root.join("workspace/greet.py")).unwrap(),
        "def greet(name):\n    return 'Hello ' + name\n"
    );
    assert_eq!(approvals_log(&root)[0]["outcome"], "denied");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn approve_is_headless_chat_only_and_validated() {
    let root = repository("usage");
    let fixture = scripted_chat(vec![done("unused")]);
    for (args, needle) in [
        (
            &["--chat", "hi", "--approve", "workspace-edits"][..],
            "--chat needs --headless",
        ),
        (&["--approve", "workspace-edits"][..], "--approve"),
        (
            &["--go", "x", "--headless", "--approve", "workspace-edits"][..],
            "--approve",
        ),
        (
            &["--chat", "hi", "--headless", "--approve", "all"][..],
            "workspace-edits",
        ),
        (
            &["--chat", "hi", "--go", "x", "--headless"][..],
            "either --go or --chat",
        ),
        (&["--chat", " ", "--headless"][..], "nonempty"),
    ] {
        let output = run(&root, &fixture, args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(stderr.contains(needle), "{args:?}: {stderr}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unreachable_model_server_is_a_setup_error() {
    let root = repository("down");
    let output = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--model", "fixture", "--endpoint", "http://127.0.0.1:9"])
        .args(["--connect-retries", "0"])
        .arg("--workspace")
        .arg(root.join("workspace"))
        .arg("--state-dir")
        .arg(root.join("state"))
        .args(["--chat", "hi", "--headless"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn interrupt_after_an_approved_edit_leaves_exactly_one_log_record() {
    let root = repository("interrupt");
    let held = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let fixture = scripted_chat(vec![
        tool_calls(&[(
            "edit_file",
            json!({"path": "greet.py", "old_text": "'Hello '", "new_text": "'Hi '"}),
        )]),
        Reply::Hold(held),
    ]);
    let child = Command::new(env!("CARGO_BIN_EXE_alfredo-tui"))
        .args(["--model", "fixture", "--endpoint", &fixture.endpoint])
        .arg("--workspace")
        .arg(root.join("workspace"))
        .arg("--state-dir")
        .arg(root.join("state"))
        .args(["--chat", "Make greet say Hi", "--headless"])
        .args(["--approve", "workspace-edits"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while fixture.count("POST /api/chat") < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "second request never came"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let output = child.wait_with_output().unwrap();
    let value = result(&output);
    assert_eq!(output.status.code(), Some(1), "{value}");
    assert_eq!(value["state"], "failed");
    assert_eq!(value["error"], "interrupted");
    assert_eq!(value["approvals"][0]["outcome"], "applied");
    let log = approvals_log(&root);
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0]["outcome"], "applied");
    let _ = fs::remove_dir_all(&root);
}
