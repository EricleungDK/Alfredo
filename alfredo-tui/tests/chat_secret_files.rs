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
    "secrets.yaml.yml",
    "deploy/secrets.y-prod.yml",
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

/// Detection: the planner's named-file reader and planning context never carry
/// a credential file's content, even when a prompt or check names it.
#[tokio::test]
async fn planner_context_returns_no_credential_file_content() {
    let repo = Repo::with_secrets();
    git(&repo.root, &["add", "-f", "--", "."]);
    git(&repo.root, &["commit", "-q", "-m", "commit everything"]);
    let baseline = git(&repo.root, &["rev-parse", "HEAD"]).trim().to_string();
    let names = SECRET_FILES.join(" ");

    let texts = planning_context::named_sources(&repo.root, &baseline, &[], &[&names], &[])
        .await
        .unwrap();
    assert_eq!(texts.len(), SECRET_FILES.len());
    for entry in &texts {
        assert!(entry.content.is_err(), "named read {}", entry.path);
    }
    for (index, entry) in texts.iter().enumerate() {
        if let Ok(content) = &entry.content {
            assert!(!content.contains(&marker(index)), "{}", entry.path);
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

/// Review of #312: a changed secret whose file name is not UTF-8 is not shown by
/// git_diff; such a name is refused rather than matched after lossy decoding.
#[cfg(unix)]
#[test]
fn git_diff_refuses_a_changed_secret_with_a_non_utf8_name() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let repo = Repo::plain();
    let name = OsStr::from_bytes(b"k\xff.pem");
    fs::write(repo.root.join(name), "SECRET-BYTES v1\n").unwrap();
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-q", "-m", "key v1"]);
    fs::write(repo.root.join(name), "SECRET-BYTES v2\n").unwrap();
    let output = repo.run("git_diff", json!({}));
    assert!(!output.contains("SECRET-BYTES"), "git_diff: {output}");
}

/// Review of #312: a secret copied unchanged to a plain name (`cp .npmrc config.txt`,
/// then `git add`) is not shown by git_diff and is refused by git_commit, with or
/// without a pathspec.
#[test]
fn copy_of_an_unchanged_secret_is_hidden_and_refused() {
    let repo = Repo::plain();
    repo.write(".npmrc", "_authToken=SECRET-COPY\n");
    git(&repo.root, &["add", "-f", ".npmrc"]);
    git(&repo.root, &["commit", "-q", "-m", "add npmrc"]);
    fs::copy(repo.root.join(".npmrc"), repo.root.join("config.txt")).unwrap();
    git(&repo.root, &["add", "config.txt"]);
    let staged = repo.run("git_diff", json!({"staged": true}));
    assert!(!staged.contains("SECRET-COPY"), "staged git_diff: {staged}");
    let whole = repo
        .card("git_commit", json!({"message": "copy"}))
        .unwrap_err();
    assert!(whole.contains(".npmrc"), "{whole}");
    let named = repo
        .card(
            "git_commit",
            json!({"message": "copy", "files": ["config.txt"]}),
        )
        .unwrap_err();
    assert!(named.contains(".npmrc"), "{named}");
}

/// Review of #312: a repository whose index holds more than 1000 files, with 1100 new
/// files staged beside an inexact copy of an unchanged `.npmrc` (`cp`, then an edit).
/// Git only searches for inexact copies up to its rename limit: past it, it prints
/// `only found copies from modified paths due to too many files` on stderr and still
/// exits 0, so the copy shows as a plain add unless the check passes `-l0` and refuses
/// any such warning.
fn large_index_with_an_inexact_secret_copy_staged() -> Repo {
    let repo = Repo::plain();
    repo.write(".npmrc", "_authToken=SECRET-LARGE\n");
    for index in 0..1100 {
        repo.write(
            &format!("pkg/m{index:04}.txt"),
            &format!("module {index}\n"),
        );
    }
    git(&repo.root, &["add", "-f", "--", "."]);
    git(
        &repo.root,
        &["commit", "-q", "-m", "1100 modules and npmrc"],
    );
    for index in 0..1100 {
        repo.write(
            &format!("gen/g{index:04}.txt"),
            &format!("generated {index}\n"),
        );
    }
    fs::copy(repo.root.join(".npmrc"), repo.root.join("config.txt")).unwrap();
    let mut copy = fs::OpenOptions::new()
        .append(true)
        .open(repo.root.join("config.txt"))
        .unwrap();
    std::io::Write::write_all(&mut copy, b"timeout=30\n").unwrap();
    git(&repo.root, &["add", "--", "gen", "config.txt"]);
    repo
}

/// Review of #312: git_diff with --staged hides an inexact copy of a secret even when
/// the index is large and git's copy search is cut short by its rename limit.
#[test]
fn large_index_inexact_secret_copy_is_hidden_by_git_diff() {
    let repo = large_index_with_an_inexact_secret_copy_staged();
    let scoped = repo.run("git_diff", json!({"staged": true, "path": "config.txt"}));
    assert!(
        !scoped.contains("SECRET-LARGE"),
        "staged git_diff: {scoped}"
    );
    assert_eq!(scoped, "(no differences)\n");
}

/// Review of #312: git_commit refuses an inexact copy of a secret staged beside a large
/// index, both committing the whole index and naming the copy's path.
#[test]
fn large_index_inexact_secret_copy_is_refused_by_git_commit() {
    let repo = large_index_with_an_inexact_secret_copy_staged();
    let whole = repo
        .card("git_commit", json!({"message": "copy"}))
        .unwrap_err();
    assert!(whole.contains(".npmrc"), "{whole}");
    let named = repo
        .card(
            "git_commit",
            json!({"message": "copy", "files": ["config.txt"]}),
        )
        .unwrap_err();
    assert!(named.contains(".npmrc"), "{named}");
}

impl Repo {
    /// Approve and apply a git_commit card, as the owner's y press does.
    fn commit_apply(&self, arguments: serde_json::Value) -> Result<String, String> {
        let card = self.card("git_commit", arguments)?;
        approval::apply(&self.root, &card)
    }

    /// Whether `path` exists in HEAD.
    fn head_has(&self, path: &str) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["cat-file", "-e", &format!("HEAD:{path}")])
            .output()
            .unwrap()
            .status
            .success()
    }

    /// Paths staged in the index against HEAD (empty when nothing is staged).
    fn staged(&self) -> String {
        git(&self.root, &["diff", "--cached", "--name-only"])
    }
}

/// Review round 1 of #312: a git_commit that names a plain path holding a copy of a
/// secret is refused at apply, after its `git add`, and the staging is undone.
#[test]
fn apply_refuses_an_untracked_exact_copy_and_restores_the_index() {
    let repo = Repo::plain();
    repo.write(".npmrc", "_authToken=SECRET-APPLY-EXACT\n");
    git(&repo.root, &["add", "-f", ".npmrc"]);
    git(&repo.root, &["commit", "-q", "-m", "npmrc"]);
    fs::copy(repo.root.join(".npmrc"), repo.root.join("config.txt")).unwrap();
    let error = repo
        .commit_apply(json!({"message": "copy", "files": ["config.txt"]}))
        .unwrap_err();
    assert!(error.contains(".npmrc"), "{error}");
    assert!(!repo.head_has("config.txt"), "HEAD holds the secret copy");
    assert_eq!(repo.staged(), "", "index not restored");
}

/// Review round 1 of #312: the same at apply for an edited (inexact) copy.
#[test]
fn apply_refuses_an_untracked_inexact_copy_and_restores_the_index() {
    let repo = Repo::plain();
    repo.write(".npmrc", "_authToken=SECRET-APPLY-INEXACT\n");
    git(&repo.root, &["add", "-f", ".npmrc"]);
    git(&repo.root, &["commit", "-q", "-m", "npmrc"]);
    fs::copy(repo.root.join(".npmrc"), repo.root.join("config.txt")).unwrap();
    let mut copy = fs::OpenOptions::new()
        .append(true)
        .open(repo.root.join("config.txt"))
        .unwrap();
    std::io::Write::write_all(&mut copy, b"timeout=30\n").unwrap();
    let error = repo
        .commit_apply(json!({"message": "copy", "files": ["config.txt"]}))
        .unwrap_err();
    assert!(error.contains(".npmrc"), "{error}");
    assert!(!repo.head_has("config.txt"), "HEAD holds the edited copy");
    assert_eq!(repo.staged(), "", "index not restored");
}

/// Review round 1 of #312: an unstaged `mv .npmrc docs/notes.txt`, committed by naming
/// the directory, is refused at apply and does not reach HEAD.
#[test]
fn apply_refuses_an_unstaged_rename_into_a_directory_and_restores_the_index() {
    let repo = Repo::plain();
    repo.write(".npmrc", "_authToken=SECRET-APPLY-RENAME\n");
    git(&repo.root, &["add", "-f", ".npmrc"]);
    git(&repo.root, &["commit", "-q", "-m", "npmrc"]);
    fs::create_dir_all(repo.root.join("docs")).unwrap();
    fs::rename(repo.root.join(".npmrc"), repo.root.join("docs/notes.txt")).unwrap();
    let error = repo
        .commit_apply(json!({"message": "move", "files": ["docs"]}))
        .unwrap_err();
    assert!(error.contains(".npmrc"), "{error}");
    assert!(
        !repo.head_has("docs/notes.txt"),
        "HEAD holds the moved secret"
    );
    assert_eq!(repo.staged(), "", "index not restored");
}

/// Review round 1 of #312: with `core.autocrlf=true`, git warns on stderr that LF will
/// be replaced by CRLF. That is not a copy or rename warning, so the unstaged diff is
/// still shown (no false refusal).
#[test]
fn git_diff_with_a_line_ending_warning_still_returns_the_diff() {
    let repo = Repo::plain();
    git(&repo.root, &["config", "core.autocrlf", "true"]);
    repo.write("notes.txt", "one\ntwo\n");
    git(&repo.root, &["add", "notes.txt"]);
    git(&repo.root, &["commit", "-q", "-m", "notes"]);
    repo.write("notes.txt", "one\ntwo\nthree\n");
    let output = repo.run("git_diff", json!({}));
    assert!(!output.contains("refused"), "{output}");
    assert!(output.contains("+three"), "{output}");
}
