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
    let root = workspace("exfil");
    write(&root, ".env", "API_TOKEN=s3cr3t\n");
    write(&root, "README.md", "hi\n");
    assert!(chat_tools::resolve(&root, ".env").is_err());
    let seed = Card {
        tool: "run_command".into(),
        arguments: json!({"command": "cat README.md", "escalate": false}),
        kind: CardKind::Shell { escalated: false },
        summary: String::new(),
        preview: String::new(),
        fingerprint: "/x".into(),
    };
    let rule = allow::rule_for(&seed).unwrap();
    let call = json!({"command": "cat README.md .env"});
    let card = approval::card(
        &root,
        &ToolCall {
            name: "run_command".into(),
            arguments: call.clone(),
        },
    )
    .unwrap();
    let auto = allow::matches(&rule, &card);
    let out = chat_tools::dispatch(
        &root,
        &ToolCall {
            name: "run_command".into(),
            arguments: call,
        },
    );
    assert!(
        !(auto && out.contains("s3cr3t")),
        "rule-approved command read a secret file: {out}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_rule_approved_recursive_grep_does_not_return_env_contents() {
    let root = workspace("grep");
    write(&root, ".env", &format!("API_TOKEN={SECRET}\n"));
    write(&root, "README.md", "the TOKEN_OK marker\n");
    let out = run(&root, "grep -r TOKEN . ; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(
        out.contains("TOKEN_OK"),
        "a visible file is not searched: {out}"
    );
    assert!(!out.contains(SECRET), "grep returned .env contents: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_secret_directory_is_hidden_from_the_sandbox() {
    let root = workspace("ssh");
    write(&root, ".ssh/id_rsa", &format!("PRIVATE {SECRET}\n"));
    write(&root, ".ssh/config", "Host example\n");
    let out = run(
        &root,
        "cat .ssh/id_rsa 2>/dev/null; grep -r PRIVATE . ; echo END",
    );
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a .ssh file leaked: {out}");
    // The listing alone: the command's own stderr may echo a name the model typed.
    let listing = run(&root, "ls -A .ssh; echo END");
    assert!(
        listing.contains("END"),
        "the listing did not run: {listing}"
    );
    assert!(!listing.contains("id_rsa"), "a .ssh name leaked: {listing}");
    assert!(!listing.contains("config"), "a .ssh name leaked: {listing}");
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
    assert!(
        out.contains("visible"),
        "a non-secret file is unreadable: {out}"
    );
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
    let git = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(&root)
        .status();
    assert!(git.is_ok_and(|s| s.success()), "git init failed");
    write(&root, ".gitignore", ".env\n");
    write(&root, ".env", &format!("IGNORED={SECRET}\n"));
    write(&root, "src/lib.txt", "tracked\n");
    let out = run(&root, "grep -r IGNORED . ; cat src/lib.txt; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(
        out.contains("tracked"),
        "a tracked file is unreadable: {out}"
    );
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
    assert!(
        out.contains("more than 128 secret files and directories"),
        "{out}"
    );
    assert!(
        out.contains("remove or rename them, or move them out of the Coding Workspace"),
        "{out}"
    );
    let _ = fs::remove_dir_all(root);
}

/// The note line of a result, if any.
fn note_line(out: &str) -> Option<&str> {
    out.lines()
        .find(|line| line.contains("hidden in this sandbox"))
}

#[test]
fn a_masked_npmrc_read_explains_the_hidden_file_with_a_count_only() {
    let root = workspace("note");
    write(&root, ".npmrc", "registry=x\n");
    let out = run(&root, "cat .npmrc");
    assert!(out.contains("Permission denied"), "{out}");
    assert!(out.contains("hidden in this sandbox"), "{out}");
    let note = note_line(&out).unwrap();
    assert_eq!(
        note,
        "note: 1 secret file(s)/directory(ies) are hidden in this sandbox and fail with Permission denied; they need the owner, do not retry on the host to read them"
    );
    assert!(!note.contains("npmrc"), "the note names the file: {note}");
    assert!(!out.contains("registry=x"), "contents leaked: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_hidden_note_counts_files_and_directories_and_shows_on_a_successful_run() {
    let root = workspace("note-count");
    write(&root, ".env", "A=1\n");
    write(&root, ".ssh/id_rsa", "k\n");
    let out = run(&root, "echo END");
    assert!(out.contains("exit code 0"), "{out}");
    assert!(
        note_line(&out)
            .is_some_and(|n| n.starts_with("note: 2 secret file(s)/directory(ies) are hidden")),
        "{out}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn no_hidden_note_when_there_are_no_secret_files() {
    let root = workspace("note-none");
    write(&root, "README.md", "hi\n");
    let out = run(&root, "echo Permission denied >&2; exit 1");
    assert!(out.contains("exit code 1"), "{out}");
    assert!(out.contains("Permission denied"), "{out}");
    assert!(!out.contains("hidden in this sandbox"), "{out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_unmasked_runs_result_text_is_unchanged() {
    let root = workspace("note-bytes");
    write(&root, "README.md", "hi\n");
    let ok = run(&root, "echo hi");
    assert!(
        ok.starts_with("exit code 0 \u{b7} sandboxed \u{b7} cwd "),
        "{ok}"
    );
    assert!(ok.ends_with("\nstdout:\nhi\nstderr:\n"), "{ok}");
    assert_eq!(ok.lines().count(), 4, "{ok}");
    let failed = run(&root, "exit 3");
    let lines: Vec<&str> = failed.lines().collect();
    assert_eq!(lines.len(), 5, "{failed}");
    assert!(
        lines[0].starts_with("exit code 3 \u{b7} sandboxed"),
        "{failed}"
    );
    assert!(
        lines[1].starts_with("note: ran sandboxed (network off;"),
        "{failed}"
    );
    assert!(
        lines[1].ends_with("the owner must approve running it on the host."),
        "{failed}"
    );
    assert_eq!(&lines[2..], ["stdout:", "", "stderr:"], "{failed}");
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

#[test]
fn a_nested_git_directory_is_hidden_from_the_sandbox() {
    let root = workspace("nested-git");
    write(
        &root,
        "notgit/.git/config",
        &format!("[remote]\nurl = https://u:{SECRET}@host\n"),
    );
    write(
        &root,
        "notgit/.git/secret.env",
        &format!("NESTED_GIT={SECRET}\n"),
    );
    let out = run(
        &root,
        "cat notgit/.git/config notgit/.git/secret.env; grep -r NESTED_GIT . ; echo END",
    );
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(
        !out.contains(SECRET),
        "a nested .git directory leaked: {out}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_nested_git_file_is_masked() {
    let root = workspace("nested-git-file");
    write(
        &root,
        "ptr/.git",
        &format!("gitdir: ../elsewhere {SECRET}\n"),
    );
    let out = run(&root, "cat ptr/.git; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a nested .git file leaked: {out}");
    let _ = fs::remove_dir_all(root);
}
