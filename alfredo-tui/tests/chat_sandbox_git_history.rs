//! Issue #622: a sandboxed `run_command` must never read a secret through git's object
//! store. Before this fix the workspace's root `.git` was bound read-only into the sandbox,
//! so a secret committed and later removed (or stashed, or only in a linked worktree's
//! common Git directory) was readable with `git show`, `git log -p`, `git cat-file` or
//! `git stash show`, although the same file was masked in the working tree (#469).
use alfredo_tui::{chat_tools, model::ToolCall};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static ID: AtomicU64 = AtomicU64::new(0);

const SECRET: &str = "s3cr3t";

/// A fresh canonical directory for one test, removed by the caller.
fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-hist-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

/// Runs `git` in `root` with the fixture settings the runbook requires (no background gc or
/// maintenance that could prune objects mid-test), and returns its standard output.
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// Commits `.env` with the secret, then removes it from the index: the secret is no longer
/// in HEAD but is still in the object store.
fn commit_then_remove_secret(root: &Path) {
    fs::write(root.join(".env"), format!("API_TOKEN={SECRET}\n")).unwrap();
    git(root, &["add", "-f", ".env"]);
    git(root, &["commit", "-qm", "oops"]);
    git(root, &["rm", "-q", "--cached", ".env"]);
    git(root, &["commit", "-qm", "remove"]);
}

/// Runs `command` through the chat tool dispatch, as an approved call would.
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
fn a_committed_then_removed_secret_is_not_readable_from_git_history_in_the_sandbox() {
    let root = workspace("repro");
    git(&root, &["init", "-q"]);
    commit_then_remove_secret(&root);
    let out = run(&root, "cat .env; git show HEAD~1:.env; echo END");
    let _ = fs::remove_dir_all(&root);
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(
        !out.contains(SECRET),
        "a committed secret leaked from .git history: {out}"
    );
}

#[test]
fn git_log_patch_does_not_show_a_removed_secret_in_the_sandbox() {
    let root = workspace("log-patch");
    git(&root, &["init", "-q"]);
    commit_then_remove_secret(&root);
    let out = run(&root, "git log -p --all; echo END");
    let _ = fs::remove_dir_all(&root);
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(
        !out.contains(SECRET),
        "git log -p leaked a removed secret: {out}"
    );
}

#[test]
fn a_secret_blob_is_not_readable_by_id_in_the_sandbox() {
    let root = workspace("blob");
    git(&root, &["init", "-q"]);
    commit_then_remove_secret(&root);
    let blob = git(&root, &["rev-parse", "HEAD~1:.env"]).trim().to_string();
    let commands = [
        format!("git cat-file -p {blob}; echo END"),
        "git cat-file -p HEAD~1:.env; echo END".to_string(),
    ];
    for command in commands {
        let out = run(&root, &command);
        assert!(
            out.contains("END"),
            "{command}: the command did not run: {out}"
        );
        assert!(
            !out.contains(SECRET),
            "{command}: a removed secret blob was readable: {out}"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_stash_holding_a_secret_is_not_readable_in_the_sandbox() {
    let root = workspace("stash");
    git(&root, &["init", "-q"]);
    fs::write(root.join(".env"), "API_TOKEN=placeholder\n").unwrap();
    git(&root, &["add", "-f", ".env"]);
    git(&root, &["commit", "-qm", "base"]);
    fs::write(root.join(".env"), format!("API_TOKEN={SECRET}\n")).unwrap();
    git(&root, &["stash", "-q"]);
    let commands = [
        "git stash show -p; echo END",
        "git show refs/stash; echo END",
    ];
    for command in commands {
        let out = run(&root, command);
        assert!(
            out.contains("END"),
            "{command}: the command did not run: {out}"
        );
        assert!(
            !out.contains(SECRET),
            "{command}: a stashed secret was readable: {out}"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_linked_worktree_does_not_show_its_common_repository_history_in_the_sandbox() {
    let base = workspace("linked");
    git(&base, &["init", "-q", "-b", "main", "main"]);
    let main = base.join("main");
    commit_then_remove_secret(&main);
    git(&main, &["worktree", "add", "-q", "-b", "wt", "../wt"]);
    let wt = base.join("wt");
    let out = run(&wt, "git show HEAD~1:.env; git log -p --all; echo END");
    let _ = fs::remove_dir_all(&base);
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(
        !out.contains(SECRET),
        "a linked worktree leaked its common repository's history: {out}"
    );
}

#[test]
fn git_under_a_parent_repository_cannot_reach_the_parent_history() {
    let parent = workspace("parent");
    git(&parent, &["init", "-q"]);
    commit_then_remove_secret(&parent);
    let ws = parent.join("ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("notes.txt"), "plain\n").unwrap();
    // Variant one: the workspace has no repository of its own.
    let no_repo = run(
        &ws,
        "git rev-parse --git-dir; git log -p --all; git show HEAD~1:.env; echo END",
    );
    assert!(
        no_repo.contains("END"),
        "the command did not run: {no_repo}"
    );
    assert!(
        !no_repo.contains(SECRET),
        "a parent repository leaked: {no_repo}"
    );
    assert!(
        !no_repo.contains(&format!("{}/.git", parent.display())),
        "git found the parent repository: {no_repo}"
    );
    // Variant two: the workspace has its own repository.
    git(&ws, &["init", "-q"]);
    fs::write(ws.join("notes.txt"), "plain\n").unwrap();
    git(&ws, &["add", "notes.txt"]);
    git(&ws, &["commit", "-qm", "own"]);
    let own = run(
        &ws,
        "git rev-parse --git-dir; git log -p --all; git show HEAD~1:.env; echo END",
    );
    let _ = fs::remove_dir_all(&parent);
    assert!(own.contains("END"), "the command did not run: {own}");
    assert!(!own.contains(SECRET), "a parent repository leaked: {own}");
    assert!(
        !own.contains(&format!("{}/.git", parent.display())),
        "git found the parent repository: {own}"
    );
}

/// Issue #622: the owner's `.git` is hidden, not bound writable or read-only, so a sandboxed
/// command cannot replace, move, link or write it. Git's own state stays exactly as it was.
#[test]
fn an_owner_git_directory_is_left_unchanged_by_sandboxed_commands() {
    let root = workspace("owner-untouched");
    git(&root, &["init", "-q"]);
    commit_then_remove_secret(&root);
    let head = fs::read_to_string(root.join(".git/HEAD")).unwrap();
    let before = git(&root, &["rev-parse", "HEAD"]);
    for command in [
        "git init -q . ; echo END",
        "rm -rf .git ; echo END",
        "mv .git moved ; echo END",
        "ln -sfn /nonexistent .git ; echo END",
        "echo x > .git/HEAD ; echo END",
    ] {
        let out = run(&root, command);
        assert!(
            out.contains("END"),
            "{command}: the command did not run: {out}"
        );
        assert_eq!(
            fs::read_to_string(root.join(".git/HEAD")).unwrap(),
            head,
            "{command}: {out}"
        );
        assert_eq!(
            git(&root, &["rev-parse", "HEAD"]),
            before,
            "{command}: {out}"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

/// Issue #622: the owner's gitfile in a linked worktree is hidden too, and the repository it
/// points to is left unchanged by sandboxed commands.
#[test]
fn an_owner_gitfile_is_left_unchanged_by_sandboxed_commands() {
    let base = workspace("owner-gitfile");
    git(&base, &["init", "-q", "-b", "main", "main"]);
    let main = base.join("main");
    commit_then_remove_secret(&main);
    git(&main, &["worktree", "add", "-q", "-b", "wt", "../wt"]);
    let wt = base.join("wt");
    let gitfile = fs::read_to_string(wt.join(".git")).unwrap();
    for command in [
        "rm -f .git ; echo END",
        "echo 'gitdir: /tmp/x' > .git ; echo END",
        "mv .git moved ; echo END",
    ] {
        let out = run(&wt, command);
        assert!(
            out.contains("END"),
            "{command}: the command did not run: {out}"
        );
        assert_eq!(
            fs::read_to_string(wt.join(".git")).unwrap(),
            gitfile,
            "{command}: {out}"
        );
    }
    let _ = fs::remove_dir_all(&base);
}

/// Issue #622: a `.git` symbolic link to another repository's git directory is not followed
/// into that repository. The sandbox cannot hide a link safely, so the run is refused and
/// nothing runs, and no secret is printed.
#[test]
fn a_root_git_symlink_to_another_repository_refuses_the_run() {
    let other = workspace("symlink-other");
    git(&other, &["init", "-q"]);
    commit_then_remove_secret(&other);
    let root = workspace("symlink-root");
    fs::write(root.join("notes.txt"), "plain\n").unwrap();
    std::os::unix::fs::symlink(other.join(".git"), root.join(".git")).unwrap();
    let out = run(
        &root,
        "git log -p --all; git show HEAD~1:.env; cat .git/config; echo END",
    );
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&other);
    assert!(out.starts_with("error: "), "the run was not refused: {out}");
    assert!(!out.contains("END"), "the command ran: {out}");
    assert!(
        !out.contains(SECRET),
        "a symlinked .git leaked another repository's history: {out}"
    );
}

/// Issue #622: a sandboxed command that runs git and succeeds still says that git history
/// is not available there, so the model uses the chat git tools instead.
#[test]
fn a_successful_git_command_says_git_history_is_hidden_in_the_sandbox() {
    let root = workspace("git-note");
    git(&root, &["init", "-q"]);
    let out = run(&root, "git --version; echo END");
    let _ = fs::remove_dir_all(&root);
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(
        out.contains("git history and git commands are not available in this sandbox"),
        "{out}"
    );
    assert!(
        out.contains("use the git_status, git_branch, git_log and git_diff tools"),
        "{out}"
    );
}
