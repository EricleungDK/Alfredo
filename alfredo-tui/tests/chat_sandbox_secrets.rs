//! Issue #469: a sandboxed `run_command` must never hand a secret file to the model.
//! The chat sandbox binds the whole Coding Workspace read-write, so before this fix a
//! rule-approved prefix command (`cat README.md`) also ran `cat README.md .env` with no
//! card. Every workspace file the secret predicate matches is now masked inside the
//! sandbox (bound over with `/dev/null`), and every secret directory is masked with an
//! empty read-only directory, so the sandbox sees what the read tools see.
use alfredo_tui::{
    chat_tools::{
        self, allow,
        approval::{self, Card, CardKind},
    },
    model::ToolCall,
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static ID: AtomicU64 = AtomicU64::new(0);

const SECRET: &str = "s3cr3t";

/// A fresh canonical workspace; each test writes the files it needs into it.
fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-secrets-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Run `command` through the chat tool dispatch, as an approved call would.
fn run(root: &Path, command: &str) -> String {
    chat_tools::dispatch(
        root,
        &ToolCall {
            name: "run_command".into(),
            arguments: json!({ "command": command }),
        },
    )
}

#[test]
fn sandboxed_prefix_rule_never_hands_secret_files_to_the_model() {
    let root = std::env::temp_dir().join(format!("allow-exfil-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(root.join(".env"), "API_TOKEN=s3cr3t\n").unwrap();
    std::fs::write(root.join("README.md"), "hi\n").unwrap();
    assert!(chat_tools::resolve(&root, ".env").is_err());
    let seed = Card { tool: "run_command".into(), arguments: json!({"command": "cat README.md", "escalate": false}),
        kind: CardKind::Shell { escalated: false }, summary: String::new(), preview: String::new(), fingerprint: "/x".into() };
    let rule = allow::rule_for(&seed).unwrap();
    let call = json!({"command": "cat README.md .env"});
    let card = approval::card(&root, &ToolCall { name: "run_command".into(), arguments: call.clone() }).unwrap();
    let auto = allow::matches(&rule, &card);
    let out = chat_tools::dispatch(&root, &ToolCall { name: "run_command".into(), arguments: call });
    assert!(!(auto && out.contains("s3cr3t")), "rule-approved command read a secret file: {out}");
}

#[test]
fn a_rule_approved_recursive_grep_does_not_return_env_contents() {
    let root = workspace("grep");
    write(&root, ".env", &format!("API_TOKEN={SECRET}\n"));
    write(&root, "README.md", "the TOKEN_OK marker\n");
    let out = run(&root, "grep -r TOKEN . ; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(out.contains("TOKEN_OK"), "a visible file is not searched: {out}");
    assert!(!out.contains(SECRET), "grep returned .env contents: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_secret_directory_is_hidden_from_the_sandbox() {
    let root = workspace("ssh");
    write(&root, ".ssh/id_rsa", &format!("PRIVATE {SECRET}\n"));
    write(&root, ".ssh/config", "Host example\n");
    let out = run(&root, "cat .ssh/id_rsa; ls -A .ssh; grep -r PRIVATE . ; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a .ssh file leaked: {out}");
    assert!(!out.contains("id_rsa"), "a .ssh name leaked: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_nested_secret_file_is_hidden_and_the_host_copy_is_unchanged() {
    let root = workspace("nested");
    write(&root, "sub/.env", &format!("NESTED={SECRET}\n"));
    write(&root, "sub/app.txt", "visible\n");
    let out = run(
        &root,
        "cat sub/.env; grep -r NESTED . ; echo HACKED > sub/.env 2>/dev/null; cat sub/app.txt; echo END",
    );
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a nested .env leaked: {out}");
    assert!(out.contains("visible"), "a non-secret file is unreadable: {out}");
    assert_eq!(
        fs::read_to_string(root.join("sub/.env")).unwrap(),
        format!("NESTED={SECRET}\n"),
        "a masked secret file was overwritten"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_gitignored_secret_file_in_a_repository_is_hidden() {
    let root = workspace("ignored");
    let git = Command::new("git").arg("init").arg("-q").current_dir(&root).status();
    assert!(git.is_ok_and(|s| s.success()), "git init failed");
    write(&root, ".gitignore", ".env\n");
    write(&root, ".env", &format!("IGNORED={SECRET}\n"));
    write(&root, "src/lib.txt", "tracked\n");
    let out = run(&root, "grep -r IGNORED . ; cat src/lib.txt; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(out.contains("tracked"), "a tracked file is unreadable: {out}");
    assert!(!out.contains(SECRET), "a gitignored .env leaked: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_non_secret_file_stays_readable_and_writable() {
    let root = workspace("plain");
    write(&root, "README.md", "readme text\n");
    let out = run(&root, "cat README.md; echo made > out.txt; echo END");
    assert!(out.contains("readme text"), "{out}");
    assert!(out.contains("END"), "{out}");
    assert_eq!(fs::read_to_string(root.join("out.txt")).unwrap(), "made\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_secret_named_symlink_refuses_the_run_and_runs_nothing() {
    let root = workspace("symlink-secret");
    write(&root, "README.md", "hi\n");
    std::os::unix::fs::symlink("README.md", root.join(".env")).unwrap();
    let out = run(&root, "touch ran");
    assert!(out.contains("nothing ran"), "{out}");
    assert!(!out.contains("exit code"), "the command ran: {out}");
    assert!(!root.join("ran").exists(), "the command ran: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_symlink_to_a_masked_file_reads_the_mask() {
    let root = workspace("symlink-file");
    write(&root, ".env", &format!("LINKED={SECRET}\n"));
    std::os::unix::fs::symlink(".env", root.join("link.txt")).unwrap();
    let out = run(&root, "cat link.txt; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a symlink to .env leaked: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_symlink_into_a_secret_directory_reads_the_mask() {
    let root = workspace("symlink-dir");
    write(&root, ".ssh/id_rsa", &format!("DIR {SECRET}\n"));
    std::os::unix::fs::symlink(".ssh", root.join("creds")).unwrap();
    let out = run(&root, "cat creds/id_rsa; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a symlink into .ssh leaked: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn too_many_secret_masks_refuse_the_run_and_run_nothing() {
    let root = workspace("cap");
    for index in 0..129 {
        write(&root, &format!("many/.env{index}"), "X\n");
    }
    let out = run(&root, "touch ran");
    assert!(out.contains("nothing ran"), "{out}");
    assert!(!out.contains("exit code"), "the command ran: {out}");
    assert!(!root.join("ran").exists(), "the command ran: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn secret_masks_up_to_the_cap_still_run() {
    let root = workspace("cap-ok");
    for index in 0..128 {
        write(&root, &format!("many/.env{index}"), "X\n");
    }
    let out = run(&root, "echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(out.contains("exit code 0"), "{out}");
    let _ = fs::remove_dir_all(root);
}
