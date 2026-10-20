//! Issue #234: read-only git tools for the Chat Agent, on temp git repositories.
use alfredo_tui::{
    chat_tools::{self, Tier, MAX_OUTPUT},
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

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@e",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

struct Repo {
    base: PathBuf,
    root: PathBuf,
}
impl Repo {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "alfredo-chat-git-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.join("repo");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        fs::write(root.join("README.md"), "# Demo\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "initial commit"]);
        Self { base, root }
    }
    fn run(&self, name: &str, arguments: serde_json::Value) -> String {
        chat_tools::dispatch(
            &self.root,
            &ToolCall {
                name: name.into(),
                arguments,
            },
        )
    }
    fn go_branch(&self) {
        git(&self.root, &["checkout", "-q", "-b", "alfredo/go-7"]);
        fs::write(self.root.join("README.md"), "# Demo\nGo branch line\n").unwrap();
        git(&self.root, &["commit", "-q", "-am", "go work"]);
        git(&self.root, &["checkout", "-q", "main"]);
    }
    fn refs_and_index(&self) -> (String, Vec<u8>) {
        (
            git(
                &self.root,
                &["for-each-ref", "--format=%(refname) %(objectname)"],
            ),
            fs::read(self.root.join(".git/index")).unwrap(),
        )
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn git_tools_are_registered_read_tier() {
    for name in ["git_status", "git_branch", "git_log", "git_diff"] {
        let tool = chat_tools::TOOLS
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(tool.tier, Tier::Read);
    }
}

#[test]
fn git_branch_names_the_current_branch_from_real_state() {
    let repo = Repo::new();
    repo.go_branch();
    let output = repo.run("git_branch", json!({}));
    assert!(output.contains("current: main"), "{output}");
    assert!(output.contains("alfredo/go-7"), "{output}");
    git(&repo.root, &["checkout", "-q", "alfredo/go-7"]);
    let output = repo.run("git_branch", json!({}));
    assert!(output.contains("current: alfredo/go-7"), "{output}");
}

#[test]
fn git_status_shows_branch_and_changes() {
    let repo = Repo::new();
    fs::write(repo.root.join("README.md"), "changed\n").unwrap();
    fs::write(repo.root.join("new.txt"), "x\n").unwrap();
    let output = repo.run("git_status", json!({}));
    assert!(output.contains("main"), "{output}");
    assert!(output.contains("M README.md"), "{output}");
    assert!(output.contains("?? new.txt"), "{output}");
}

#[test]
fn git_log_lists_commits_of_a_ref() {
    let repo = Repo::new();
    repo.go_branch();
    let output = repo.run("git_log", json!({}));
    assert!(output.contains("initial commit"), "{output}");
    assert!(!output.contains("go work"), "{output}");
    let output = repo.run("git_log", json!({"ref": "alfredo/go-7", "max_count": 1}));
    assert!(output.contains("go work"), "{output}");
    assert!(!output.contains("initial commit"), "{output}");
}

#[test]
fn git_diff_describes_go_branch_against_base() {
    let repo = Repo::new();
    repo.go_branch();
    let output = repo.run(
        "git_diff",
        json!({"base": "main", "target": "alfredo/go-7"}),
    );
    assert!(output.contains("+Go branch line"), "{output}");
    assert!(output.contains("README.md"), "{output}");
}

#[test]
fn git_diff_shows_working_tree_and_staged_changes() {
    let repo = Repo::new();
    fs::write(repo.root.join("README.md"), "# Demo\nunstaged\n").unwrap();
    assert!(repo.run("git_diff", json!({})).contains("+unstaged"));
    git(&repo.root, &["add", "README.md"]);
    assert!(!repo.run("git_diff", json!({})).contains("+unstaged"));
    assert!(repo
        .run("git_diff", json!({"staged": true}))
        .contains("+unstaged"));
}

#[test]
fn git_diff_hides_secret_files() {
    let repo = Repo::new();
    fs::write(repo.root.join(".env"), "TOKEN=hunter2\n").unwrap();
    fs::write(repo.root.join("id.key"), "PRIVATE\n").unwrap();
    git(&repo.root, &["add", "."]);
    let output = repo.run("git_diff", json!({"staged": true}));
    assert!(!output.contains("hunter2"), "{output}");
    assert!(!output.contains("PRIVATE"), "{output}");
    let output = repo.run("git_diff", json!({"staged": true, "path": ".env"}));
    assert!(output.starts_with("error: "), "{output}");
}

#[test]
fn option_like_and_unsafe_arguments_are_refused() {
    let repo = Repo::new();
    let pwned = repo.base.join("pwned");
    let out_flag = format!("--output={}", pwned.display());
    for (tool, arguments) in [
        ("git_log", json!({ "ref": out_flag })),
        ("git_log", json!({"ref": "-p"})),
        ("git_diff", json!({ "base": out_flag })),
        ("git_diff", json!({"target": "-R", "base": "main"})),
        ("git_diff", json!({"base": "main:README.md"})),
        ("git_diff", json!({"path": "../outside"})),
        ("git_diff", json!({"path": ":(top)x"})),
        ("git_diff", json!({"path": "-x"})),
        ("git_log", json!({"max_count": "5"})),
        ("git_diff", json!({"staged": "yes"})),
    ] {
        let output = repo.run(tool, arguments.clone());
        assert!(
            output.starts_with("error: "),
            "{tool} {arguments}: {output}"
        );
    }
    assert!(!pwned.exists());
}

#[test]
fn git_tools_change_no_refs_or_index() {
    let repo = Repo::new();
    repo.go_branch();
    fs::write(repo.root.join("README.md"), "dirty\n").unwrap();
    let before = repo.refs_and_index();
    for (tool, arguments) in [
        ("git_status", json!({})),
        ("git_branch", json!({})),
        ("git_log", json!({"ref": "alfredo/go-7"})),
        ("git_diff", json!({})),
        (
            "git_diff",
            json!({"base": "main", "target": "alfredo/go-7"}),
        ),
    ] {
        let output = repo.run(tool, arguments);
        assert!(!output.starts_with("error: "), "{tool}: {output}");
    }
    assert_eq!(repo.refs_and_index(), before);
    assert!(!repo.root.join(".git/index.lock").exists());
}

#[test]
fn git_output_is_capped_with_the_marker() {
    let repo = Repo::new();
    fs::write(repo.root.join("big.txt"), "line of text\n".repeat(5000)).unwrap();
    git(&repo.root, &["add", "big.txt"]);
    let output = repo.run("git_diff", json!({"staged": true}));
    assert!(output.len() <= MAX_OUTPUT + 100, "{}", output.len());
    assert!(output.contains("[truncated: "), "{output}");
}

#[test]
fn non_repository_is_an_error() {
    let base = std::env::temp_dir().join(format!("alfredo-chat-nogit-{}", std::process::id()));
    fs::create_dir_all(&base).unwrap();
    let output = chat_tools::dispatch(
        &base,
        &ToolCall {
            name: "git_status".into(),
            arguments: json!({}),
        },
    );
    let _ = fs::remove_dir_all(&base);
    assert!(output.starts_with("error: "), "{output}");
}
