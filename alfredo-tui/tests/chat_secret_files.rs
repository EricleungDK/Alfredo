//! Issue #312: common credential files are never read, listed, searched, edited,
//! committed or sent to the planner, whatever their case or directory.
use alfredo_tui::{
    chat_tools::{self, approval},
    model::ToolCall,
    planning_context,
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static ID: AtomicU64 = AtomicU64::new(0);

/// Credential files with a marker line each. Every one must stay out of every tool.
const SECRET_FILES: &[&str] = &[
    ".env",
    "a.env",
    "prod.env",
    "config/.env.local",
    "server.key",
    "cert.pem",
    ".npmrc",
    "web/.npmrc",
    ".netrc",
    ".pgpass",
    ".git-credentials",
    "id_rsa",
    "id_ed25519",
    "keys/id_ecdsa",
    ".ssh/config",
    ".aws/credentials",
    "infra/.kube/config",
    "credentials.json",
    "config/credentials-prod.json",
    "app.keystore",
    "release/app.jks",
    "terraform.tfvars",
    "secrets.yml",
    "config/secrets.yaml",
    "Prod.ENV",
    "CONFIG/.AWS/Credentials",
    "Id_RSA",
];

fn marker(index: usize) -> String {
    format!("SECRET-MARKER-{index}-7f3a")
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "commit.gpgsign=false"])
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
    /// A repository on `main` with README.md committed and nothing else.
    fn plain() -> Self {
        let base = std::env::temp_dir().join(format!(
            "alfredo-secret-files-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("repo");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Owner"]);
        git(&root, &["config", "user.email", "owner@example.com"]);
        fs::write(root.join("README.md"), "# Demo\n").unwrap();
        git(&root, &["add", "README.md"]);
        git(&root, &["commit", "-q", "-m", "initial commit"]);
        Self { base, root }
    }

    /// A repository with README.md committed and every SECRET_FILES entry written
    /// (untracked) with its marker line.
    fn with_secrets() -> Self {
        let repo = Self::plain();
        for (index, path) in SECRET_FILES.iter().enumerate() {
            repo.write(path, &format!("{}\n", marker(index)));
        }
        repo
    }

    fn write(&self, path: &str, content: &str) {
        let full = self.root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
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

    fn card(&self, name: &str, arguments: serde_json::Value) -> Result<approval::Card, String> {
        approval::card(
            &self.root,
            &ToolCall {
                name: name.into(),
                arguments,
            },
        )
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// Regression for issue #312: read_file, grep, list_files, the edit tools and
/// git_commit each refuse every planted credential file and never return its marker.
#[test]
fn every_credential_file_is_refused_by_each_chat_tool() {
    let repo = Repo::with_secrets();
    for (index, path) in SECRET_FILES.iter().enumerate() {
        let secret = marker(index);

        let read = repo.run("read_file", json!({"path": path}));
        assert!(!read.contains(&secret), "read_file {path}: {read}");
        assert!(read.contains("excluded"), "read_file {path}: {read}");

        let grep = repo.run("grep", json!({"pattern": secret}));
        assert_eq!(grep, "no matches", "grep {path}");
        let grep_file = repo.run("grep", json!({"pattern": secret, "path": path}));
        assert!(!grep_file.contains(&secret), "grep in {path}: {grep_file}");

        let edit = repo
            .card(
                "edit_file",
                json!({"path": path, "old_text": secret, "new_text": "x"}),
            )
            .unwrap_err();
        assert!(edit.contains("excluded"), "edit_file {path}: {edit}");
        let write = repo
            .card("write_file", json!({"path": path, "content": "x\n"}))
            .unwrap_err();
        assert!(write.contains("excluded"), "write_file {path}: {write}");

        let commit = repo
            .card("git_commit", json!({"message": "add it", "files": [path]}))
            .unwrap_err();
        assert!(
            commit.contains("by hand") || commit.contains("excluded"),
            "git_commit {path}: {commit}"
        );
    }

    let listing = repo.run("list_files", json!({}));
    assert!(listing.lines().any(|line| line == "README.md"), "{listing}");
    for path in SECRET_FILES {
        assert!(
            !listing.lines().any(|line| line.eq_ignore_ascii_case(path)),
            "list_files shows {path}: {listing}"
        );
    }
    let everything = repo.run("grep", json!({"pattern": "SECRET-MARKER"}));
    assert_eq!(everything, "no matches");
}

/// Detection: the planner's committed-text reader and planning context never carry
/// a credential file's content, even when a prompt or check names it.
#[tokio::test]
async fn planner_context_returns_no_credential_file_content() {
    let repo = Repo::with_secrets();
    git(&repo.root, &["add", "-f", "--", "."]);
    git(&repo.root, &["commit", "-q", "-m", "commit everything"]);
    let names: Vec<String> = SECRET_FILES.iter().map(|p| p.to_string()).collect();

    let texts = planning_context::committed_text(&repo.root, &names)
        .await
        .unwrap();
    assert_eq!(texts.len(), names.len());
    for (index, entry) in texts.iter().enumerate() {
        let secret = marker(index);
        assert!(entry.content.is_err(), "committed_text read {}", entry.path);
        if let Ok(content) = &entry.content {
            assert!(!content.contains(&secret), "{}", entry.path);
        }
    }

    let prompt = format!("Rotate the token in {}", SECRET_FILES.join(" and "));
    let context = planning_context::capture(&repo.root, &prompt)
        .await
        .unwrap();
    let serialized = serde_json::to_string(&context).unwrap();
    for index in 0..SECRET_FILES.len() {
        assert!(!serialized.contains(&marker(index)), "{serialized}");
    }
}

/// Review of #312: git_diff shows no secret file's content, whether the file is
/// tracked and modified or committed and changed between refs, in every diff mode.
#[test]
fn git_diff_never_shows_tracked_or_committed_secret_content() {
    let repo = Repo::plain();
    let tracked = [".npmrc", ".ssh/id_rsa", "infra/.kube/config", "prod.env"];
    for path in tracked {
        repo.write(path, "SECRET-DIFF v1\n");
    }
    repo.write("unchanged.key", "SECRET-DIFF stable\n");
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-q", "-m", "secrets v1"]);

    // Modified in the working tree, with a visible non-secret change beside them.
    for path in tracked {
        repo.write(path, "SECRET-DIFF v2\n");
    }
    repo.write("README.md", "# Demo\nchanged\n");
    for args in [json!({}), json!({"base": "main"})] {
        let output = repo.run("git_diff", args.clone());
        assert!(!output.contains("SECRET-DIFF"), "git_diff {args}: {output}");
        assert!(output.contains("changed"), "git_diff {args}: {output}");
    }
    let scoped = repo.run("git_diff", json!({"path": "infra"}));
    assert!(!scoped.contains("SECRET-DIFF"), "git_diff infra: {scoped}");
    assert_eq!(scoped, "(no differences)\n");

    // Staged, including a secret renamed unchanged to a non-secret name.
    git(&repo.root, &["mv", "unchanged.key", "notes.txt"]);
    git(&repo.root, &["add", "-A"]);
    let staged = repo.run("git_diff", json!({"staged": true}));
    assert!(!staged.contains("SECRET-DIFF"), "staged git_diff: {staged}");
    assert!(staged.contains("changed"), "staged git_diff: {staged}");

    // Committed on a branch: base...target shows the change from the committed secret.
    git(&repo.root, &["checkout", "-q", "-b", "feat"]);
    git(&repo.root, &["commit", "-q", "-m", "secrets v2"]);
    let range = repo.run("git_diff", json!({"base": "main", "target": "feat"}));
    assert!(
        !range.contains("SECRET-DIFF"),
        "base/target git_diff: {range}"
    );
    assert!(range.contains("changed"), "base/target git_diff: {range}");
}

/// Review of #312: a secret staged and renamed out of its name is refused for a
/// pathspec commit that does not name the old path, not only in whole-index mode.
#[test]
fn commit_of_a_secret_renamed_to_a_plain_name_is_refused() {
    let repo = Repo::plain();
    repo.write(".npmrc", "_authToken=SECRET-RENAME\n");
    git(&repo.root, &["add", "-f", ".npmrc"]);
    git(&repo.root, &["commit", "-q", "-m", "add npmrc"]);
    fs::create_dir_all(repo.root.join("docs")).unwrap();
    git(&repo.root, &["mv", ".npmrc", "docs/notes.txt"]);
    let error = repo
        .card("git_commit", json!({"message": "move", "files": ["docs"]}))
        .unwrap_err();
    assert!(error.contains(".npmrc"), "{error}");
    assert!(error.contains("by hand"), "{error}");
}
