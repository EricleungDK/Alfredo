//! Issue #237: git write tools (commit, switch, merge) and push for the Chat Agent,
//! each behind owner Approval, run on host against temp repos with a local bare remote.
mod ollama_fixture;

use alfredo_tui::{
    chat_tools::{
        self,
        approval::{self, Card, CardKind, Decision, Gate},
        Tier, TOOLS,
    },
    model::{App, Event, Session, ToolCall, Update},
    provider::Ollama,
    ui,
};
use ollama_fixture::{done, Reply};
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

static ID: AtomicU64 = AtomicU64::new(0);

/// Held by every test that sets GIT_SSH: the process environment is shared.
static SSH: Mutex<()> = Mutex::new(());

/// A GIT_SSH stand-in that runs the remote command locally, so a push reaches a bare
/// repository through an `ssh://localhost` URL (a local-path remote is refused, #309).
fn ssh_stand_in(base: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = base.join("ssh-stand-in");
    fs::write(
        &script,
        "#!/bin/sh\nfor last; do :; done\nexec sh -c \"$(printf '%s' \"$last\" | sed 's/^git-/git /')\"\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn ssh_url(path: &Path) -> String {
    format!("ssh://localhost{}", path.display())
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
    /// Set once a remote is configured: GIT_SSH is the stand-in for this test.
    ssh: Option<MutexGuard<'static, ()>>,
}

impl Repo {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "alfredo-git-write-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("repo");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Owner"]);
        git(&root, &["config", "user.email", "owner@example.com"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        fs::write(root.join("README.md"), "# Demo\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "initial commit"]);
        Self {
            base,
            root,
            ssh: None,
        }
    }

    /// A local bare remote named origin with main pushed.
    fn with_remote(mut self) -> Self {
        self.ssh = Some(SSH.lock().unwrap_or_else(|error| error.into_inner()));
        std::env::set_var("GIT_SSH", ssh_stand_in(&self.base));
        let bare = self.remote();
        git(
            &self.base,
            &["init", "-q", "--bare", "-b", "main", "remote.git"],
        );
        git(&self.root, &["remote", "add", "origin", &ssh_url(&bare)]);
        git(&self.root, &["push", "-q", "-u", "origin", "main"]);
        self
    }

    fn remote(&self) -> PathBuf {
        self.base.join("remote.git")
    }

    fn go_branch(&self) {
        git(&self.root, &["checkout", "-q", "-b", "alfredo/go-7"]);
        fs::write(self.root.join("feature.txt"), "go work\n").unwrap();
        git(&self.root, &["add", "feature.txt"]);
        git(&self.root, &["commit", "-q", "-m", "go: add feature"]);
        git(&self.root, &["checkout", "-q", "main"]);
    }

    fn head(&self) -> String {
        git(&self.root, &["rev-parse", "HEAD"]).trim().to_string()
    }

    fn branch(&self) -> String {
        git(&self.root, &["branch", "--show-current"])
            .trim()
            .to_string()
    }

    fn card(&self, name: &str, arguments: serde_json::Value) -> Result<Card, String> {
        approval::card(&self.root, &call(name, arguments))
    }

    /// Preview then apply, as an approved card.
    fn approve(&self, name: &str, arguments: serde_json::Value) -> Result<String, String> {
        approval::apply(&self.root, &self.card(name, arguments)?)
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        name: name.into(),
        arguments,
    }
}

#[test]
fn git_write_tools_are_gated_and_push_has_its_own_tier() {
    for name in ["git_commit", "git_switch", "git_merge"] {
        let tool = TOOLS.iter().find(|tool| tool.name == name).unwrap();
        assert_eq!(tool.tier, Tier::Mutating, "{name}");
    }
    let push = TOOLS.iter().find(|tool| tool.name == "git_push").unwrap();
    assert_eq!(push.tier, Tier::Push);
    assert!(Tier::Mutating.allowlistable());
    assert!(!Tier::Push.allowlistable());
    assert!(!Tier::Read.allowlistable());
}

/// Issue #359: a gitfile or a `commondir` file to another repository must not redirect an
/// approved commit into it: the write is refused with the pointer reason, and the other
/// repository's HEAD and files stay as they were.
#[test]
fn a_gitfile_to_another_repository_never_receives_a_commit() {
    for variant in ["gitfile", "commondir behind a gitfile"] {
        let other = Repo::new();
        let other_head = other.head();
        let ws = other.base.join("ws");
        fs::create_dir_all(&ws).unwrap();
        let other_git = other.root.join(".git");
        if variant == "gitfile" {
            fs::write(
                ws.join(".git"),
                format!("gitdir: {}\n", other_git.display()),
            )
            .unwrap();
        } else {
            fs::write(ws.join(".git"), "gitdir: ./fake\n").unwrap();
            fs::create_dir_all(ws.join("fake")).unwrap();
            fs::write(ws.join("fake/HEAD"), "ref: refs/heads/main\n").unwrap();
            fs::write(
                ws.join("fake/commondir"),
                format!("{}\n", other_git.display()),
            )
            .unwrap();
        }
        fs::write(ws.join("b.txt"), "b\n").unwrap();
        let outcome = approval::card(
            &ws,
            &call(
                "git_commit",
                json!({"message": "redirected", "files": ["b.txt"]}),
            ),
        )
        .and_then(|card| approval::apply(&ws, &card));
        let error = outcome.unwrap_err();
        assert!(
            error.contains("points outside the Coding Workspace"),
            "{variant}: {error}"
        );
        assert_eq!(other.head(), other_head, "{variant}");
        assert!(!other.root.join("b.txt").exists(), "{variant}");
    }
}

#[test]
fn commit_staged_files_shows_files_and_message_then_commits() {
    let repo = Repo::new();
    fs::write(repo.root.join("README.md"), "# Demo\nmore\n").unwrap();
    fs::write(repo.root.join("unstaged.txt"), "x\n").unwrap();
    git(&repo.root, &["add", "README.md"]);
    let card = repo
        .card("git_commit", json!({"message": "docs: extend readme"}))
        .unwrap();
    assert_eq!(card.kind, CardKind::Git);
    assert!(card.summary.contains("main"), "{}", card.summary);
    assert!(card.preview.contains("README.md"), "{}", card.preview);
    assert!(!card.preview.contains("unstaged.txt"), "{}", card.preview);
    assert!(
        card.preview.contains("docs: extend readme"),
        "{}",
        card.preview
    );
    approval::apply(&repo.root, &card).unwrap();
    assert_eq!(
        git(&repo.root, &["log", "-1", "--format=%s"]).trim(),
        "docs: extend readme"
    );
    let committed = git(&repo.root, &["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(committed.trim(), "README.md");
}

#[test]
fn commit_selected_files_includes_new_files_and_leaves_others() {
    let repo = Repo::new();
    fs::write(repo.root.join("new.txt"), "new\n").unwrap();
    fs::write(repo.root.join("README.md"), "# Demo\nchanged\n").unwrap();
    let card = repo
        .card(
            "git_commit",
            json!({"message": "add new", "files": ["new.txt"]}),
        )
        .unwrap();
    assert!(card.preview.contains("new.txt"), "{}", card.preview);
    assert!(!card.preview.contains("README.md"), "{}", card.preview);
    approval::apply(&repo.root, &card).unwrap();
    let committed = git(&repo.root, &["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(committed.trim(), "new.txt");
    let status = git(&repo.root, &["status", "--porcelain"]);
    assert!(status.contains(" M README.md"), "{status}");
}

#[test]
fn commit_of_a_directory_holding_secrets_is_refused_before_asking() {
    let repo = Repo::new();
    fs::create_dir_all(repo.root.join("cfg")).unwrap();
    fs::write(repo.root.join("cfg/app.toml"), "a = 1\n").unwrap();
    fs::write(repo.root.join("cfg/.env"), "TOKEN=x\n").unwrap();
    fs::write(repo.root.join("cfg/server.key"), "k\n").unwrap();
    let error = repo
        .card(
            "git_commit",
            json!({"message": "add cfg", "files": ["cfg"]}),
        )
        .unwrap_err();
    assert!(error.contains("cfg/.env"), "{error}");
    assert!(error.contains("by hand"), "{error}");
    assert!(git(&repo.root, &["ls-files", "cfg"]).trim().is_empty());
}

#[test]
fn commit_of_a_prestaged_secret_is_refused_before_asking() {
    let repo = Repo::new();
    fs::write(repo.root.join(".env.local"), "TOKEN=x\n").unwrap();
    git(&repo.root, &["add", ".env.local"]);
    let error = repo
        .card("git_commit", json!({"message": "oops"}))
        .unwrap_err();
    assert!(error.contains(".env.local"), "{error}");
    assert!(error.contains("by hand"), "{error}");
}

#[test]
fn a_secret_staged_after_the_preview_is_refused_at_apply() {
    let repo = Repo::new();
    fs::write(repo.root.join("a.txt"), "a\n").unwrap();
    git(&repo.root, &["add", "a.txt"]);
    let card = repo
        .card("git_commit", json!({"message": "add a"}))
        .unwrap();
    fs::write(repo.root.join("id.pem"), "k\n").unwrap();
    git(&repo.root, &["add", "id.pem"]);
    assert!(approval::apply(&repo.root, &card).is_err());
    assert!(!git(&repo.root, &["ls-tree", "-r", "HEAD"]).contains("id.pem"));
}

#[test]
fn commit_of_a_clean_directory_is_allowed_including_generated_trees() {
    let repo = Repo::new();
    fs::create_dir_all(repo.root.join("pkg/vendor/lib")).unwrap();
    fs::create_dir_all(repo.root.join("pkg/dist")).unwrap();
    fs::write(repo.root.join("pkg/main.go"), "package p\n").unwrap();
    fs::write(repo.root.join("pkg/vendor/lib/x.go"), "package x\n").unwrap();
    fs::write(repo.root.join("pkg/dist/out.js"), "1\n").unwrap();
    repo.approve(
        "git_commit",
        json!({"message": "add pkg", "files": ["pkg"]}),
    )
    .unwrap();
    let tracked = git(&repo.root, &["ls-files", "pkg"]);
    assert!(tracked.contains("pkg/vendor/lib/x.go"), "{tracked}");
    assert!(tracked.contains("pkg/dist/out.js"), "{tracked}");
}

#[test]
fn commit_with_nothing_to_commit_is_refused_before_asking() {
    let repo = Repo::new();
    let error = repo
        .card("git_commit", json!({"message": "empty"}))
        .unwrap_err();
    assert!(error.contains("nothing"), "{error}");
    let error = repo
        .card(
            "git_commit",
            json!({"message": "x", "files": ["README.md"]}),
        )
        .unwrap_err();
    assert!(error.contains("nothing"), "{error}");
    let error = repo
        .card("git_commit", json!({"message": " "}))
        .unwrap_err();
    assert!(error.contains("message"), "{error}");
}

/// Issue #269: a commit or merge file list is shown whole or the call is refused. The
/// cap is 40 listed lines: 40 passes, 41 is refused before the index, HEAD or tree move.
fn add_files(repo: &Repo, count: usize) {
    for index in 0..count {
        fs::write(repo.root.join(format!("f{index:03}.txt")), "u\n").unwrap();
    }
    git(&repo.root, &["add", "."]);
}

#[test]
fn commit_listing_of_exactly_40_files_is_shown_whole() {
    let repo = Repo::new();
    add_files(&repo, 40);
    let card = repo
        .card("git_commit", json!({"message": "forty"}))
        .unwrap();
    assert!(!card.preview.contains("more"), "{}", card.preview);
    assert!(card.preview.contains("f039.txt"), "{}", card.preview);
    approval::apply(&repo.root, &card).unwrap();
    assert!(git(&repo.root, &["status", "--porcelain"]).is_empty());
}

#[test]
fn commit_listing_of_41_files_is_refused_and_nothing_moves() {
    let repo = Repo::new();
    add_files(&repo, 41);
    let (head, index) = (repo.head(), git(&repo.root, &["ls-files", "-s"]));
    let status = git(&repo.root, &["status", "--porcelain"]);
    let error = repo
        .card("git_commit", json!({"message": "x"}))
        .unwrap_err();
    assert!(error.contains("41") && error.contains("40-line"), "{error}");
    assert!(error.contains("fewer files"), "{error}");
    // Named files hit the same cap; so does the apply path.
    let names: Vec<String> = (0..41).map(|i| format!("f{i:03}.txt")).collect();
    git(&repo.root, &["reset", "-q"]);
    let error = repo
        .card("git_commit", json!({"message": "x", "files": names}))
        .unwrap_err();
    assert!(error.contains("40-line"), "{error}");
    git(&repo.root, &["add", "."]);
    let error = repo
        .approve("git_commit", json!({"message": "x"}))
        .unwrap_err();
    assert!(error.contains("40-line"), "{error}");
    assert_eq!(repo.head(), head);
    assert_eq!(git(&repo.root, &["ls-files", "-s"]), index);
    assert_eq!(git(&repo.root, &["status", "--porcelain"]), status);
}

#[test]
fn a_secret_in_an_over_cap_commit_is_still_refused_as_a_secret() {
    let repo = Repo::new();
    add_files(&repo, 45);
    fs::write(repo.root.join(".env"), "KEY=1\n").unwrap();
    git(&repo.root, &["add", "-f", ".env"]);
    let error = repo
        .card("git_commit", json!({"message": "x"}))
        .unwrap_err();
    assert!(!error.contains("40-line"), "{error}");
}

/// A branch `wide` that adds `count` files, merged from `main`.
fn wide_branch(repo: &Repo, count: usize) {
    git(&repo.root, &["checkout", "-q", "-b", "wide"]);
    add_files(repo, count);
    git(&repo.root, &["commit", "-q", "-m", "wide"]);
    git(&repo.root, &["checkout", "-q", "main"]);
}

#[test]
fn merge_file_list_of_exactly_40_lines_is_shown_whole_and_41_is_refused() {
    // `--stat` is one line per file plus a summary line: 39 files = 40 lines.
    let repo = Repo::new();
    wide_branch(&repo, 39);
    let card = repo.card("git_merge", json!({"source": "wide"})).unwrap();
    assert!(!card.preview.contains("more"), "{}", card.preview);
    assert!(card.preview.contains("f038.txt"), "{}", card.preview);
    approval::apply(&repo.root, &card).unwrap();
    assert!(repo.root.join("f038.txt").exists());

    let repo = Repo::new();
    wide_branch(&repo, 40);
    let head = repo.head();
    let error = repo
        .card("git_merge", json!({"source": "wide"}))
        .unwrap_err();
    assert!(error.contains("41") && error.contains("40-line"), "{error}");
    let error = repo
        .approve("git_merge", json!({"source": "wide"}))
        .unwrap_err();
    assert!(error.contains("40-line"), "{error}");
    assert_eq!(repo.head(), head);
    assert!(!repo.root.join("f000.txt").exists());
    assert!(git(&repo.root, &["status", "--porcelain"]).is_empty());
    assert!(!repo.root.join(".git/MERGE_HEAD").exists());
}

#[test]
fn commit_hooks_do_not_run() {
    let repo = Repo::new();
    let hook = repo.root.join(".git/hooks/pre-commit");
    fs::write(
        &hook,
        "#!/bin/sh\ntouch \"$(dirname \"$0\")/../../HOOK_RAN\"\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(repo.root.join("a.txt"), "a\n").unwrap();
    repo.approve("git_commit", json!({"message": "a", "files": ["a.txt"]}))
        .unwrap();
    assert!(!repo.root.join("HOOK_RAN").exists());
}

#[test]
fn switch_creates_or_changes_branch_and_card_shows_from_to() {
    let repo = Repo::new();
    let card = repo
        .card("git_switch", json!({"branch": "feature/x", "create": true}))
        .unwrap();
    assert_eq!(card.kind, CardKind::Git);
    assert!(card.summary.contains("main"), "{}", card.summary);
    assert!(card.summary.contains("feature/x"), "{}", card.summary);
    approval::apply(&repo.root, &card).unwrap();
    assert_eq!(repo.branch(), "feature/x");
    repo.approve("git_switch", json!({"branch": "main"}))
        .unwrap();
    assert_eq!(repo.branch(), "main");
    let error = repo
        .card("git_switch", json!({"branch": "missing"}))
        .unwrap_err();
    assert!(error.contains("missing"), "{error}");
    let error = repo
        .card("git_switch", json!({"branch": "main", "create": true}))
        .unwrap_err();
    assert!(error.contains("exists"), "{error}");
}

#[test]
fn padded_config_filter_is_refused_before_switch() {
    let repo = Repo::new();
    repo.go_branch();
    // The attribute lives on the target branch, so the checkout of feature.txt runs the
    // smudge filter there (without the fix the marker is created).
    git(&repo.root, &["checkout", "-q", "alfredo/go-7"]);
    fs::write(repo.root.join(".gitattributes"), "*.txt filter=s\n").unwrap();
    git(&repo.root, &["add", ".gitattributes"]);
    git(&repo.root, &["commit", "-q", "-m", "attributes"]);
    git(&repo.root, &["checkout", "-q", "main"]);
    let marker = repo.base.join("smudge-ran");
    let mut config = fs::OpenOptions::new()
        .append(true)
        .open(repo.root.join(".git/config"))
        .unwrap();
    // More than 1 MiB of padding before the driver (#308): the listing must not be cut.
    let padding = "v".repeat(1_200_000);
    write!(
        config,
        "[pad]\n\tk = {padding}\n[filter \"s\"]\n\tsmudge = touch {}; cat\n",
        marker.display()
    )
    .unwrap();
    drop(config);
    let outcome = repo.approve("git_switch", json!({"branch": "alfredo/go-7"}));
    assert!(!marker.exists(), "repository filter ran: {outcome:?}");
    let error = outcome.unwrap_err();
    assert!(error.contains("filter drivers"), "{error}");
    assert_eq!(repo.branch(), "main");
}

#[test]
fn option_like_refs_paths_and_remotes_are_refused() {
    let repo = Repo::new().with_remote();
    for (name, arguments) in [
        ("git_switch", json!({"branch": "-f"})),
        (
            "git_switch",
            json!({"branch": "--orphan=x", "create": true}),
        ),
        ("git_merge", json!({"source": "--abort"})),
        ("git_merge", json!({"source": "main", "target": "-x"})),
        ("git_commit", json!({"message": "m", "files": ["-a"]})),
        ("git_commit", json!({"message": "m", "files": [":(glob)*"]})),
        ("git_commit", json!({"message": "m", "files": ["../x"]})),
        ("git_commit", json!({"message": "m", "files": [".env"]})),
        ("git_push", json!({"remote": "--mirror"})),
        ("git_push", json!({"remote": "/tmp/elsewhere.git"})),
        ("git_push", json!({"remote": "nope"})),
        ("git_push", json!({"branch": "-f"})),
    ] {
        let before = repo.head();
        assert!(
            repo.card(name, arguments.clone()).is_err(),
            "{name} {arguments}"
        );
        assert_eq!(repo.head(), before);
    }
}

#[test]
fn merge_card_shows_source_target_commits_and_files_then_merges() {
    let repo = Repo::new();
    repo.go_branch();
    let card = repo
        .card("git_merge", json!({"source": "alfredo/go-7"}))
        .unwrap();
    assert_eq!(card.kind, CardKind::Git);
    assert!(card.summary.contains("alfredo/go-7"), "{}", card.summary);
    assert!(card.summary.contains("main"), "{}", card.summary);
    assert!(card.preview.contains("go: add feature"), "{}", card.preview);
    assert!(card.preview.contains("feature.txt"), "{}", card.preview);
    approval::apply(&repo.root, &card).unwrap();
    assert!(repo.root.join("feature.txt").exists());
    assert_eq!(repo.branch(), "main");
}

#[test]
fn merge_into_another_target_is_refused_until_switched() {
    let repo = Repo::new();
    repo.go_branch();
    let error = repo
        .card(
            "git_merge",
            json!({"source": "main", "target": "alfredo/go-7"}),
        )
        .unwrap_err();
    assert!(error.contains("git_switch"), "{error}");
    let error = repo
        .card("git_merge", json!({"source": "main"}))
        .unwrap_err();
    assert!(error.contains("up to date"), "{error}");
}

#[test]
fn merge_conflicts_are_listed_and_the_merge_is_aborted() {
    let repo = Repo::new();
    git(&repo.root, &["checkout", "-q", "-b", "alfredo/go-9"]);
    fs::write(repo.root.join("README.md"), "# Go side\n").unwrap();
    git(&repo.root, &["commit", "-q", "-am", "go side"]);
    git(&repo.root, &["checkout", "-q", "main"]);
    fs::write(repo.root.join("README.md"), "# Main side\n").unwrap();
    git(&repo.root, &["commit", "-q", "-am", "main side"]);
    let before = repo.head();
    let error = repo
        .approve("git_merge", json!({"source": "alfredo/go-9"}))
        .unwrap_err();
    assert!(error.contains("conflict"), "{error}");
    assert!(error.contains("README.md"), "{error}");
    assert!(error.contains("aborted"), "{error}");
    assert_eq!(repo.head(), before);
    assert!(!repo.root.join(".git/MERGE_HEAD").exists());
    assert_eq!(git(&repo.root, &["status", "--porcelain"]), "");
    assert_eq!(
        fs::read_to_string(repo.root.join("README.md")).unwrap(),
        "# Main side\n"
    );
}

#[test]
fn a_repository_changed_after_the_preview_is_refused() {
    let repo = Repo::new();
    fs::write(repo.root.join("a.txt"), "a\n").unwrap();
    git(&repo.root, &["add", "a.txt"]);
    let card = repo.card("git_commit", json!({"message": "a"})).unwrap();
    fs::write(repo.root.join("b.txt"), "b\n").unwrap();
    git(&repo.root, &["add", "b.txt"]);
    let before = repo.head();
    let error = approval::apply(&repo.root, &card).unwrap_err();
    assert!(error.contains("changed"), "{error}");
    assert_eq!(repo.head(), before);

    let card = repo
        .card("git_switch", json!({"branch": "side", "create": true}))
        .unwrap();
    git(&repo.root, &["commit", "-q", "-m", "moved"]);
    assert!(approval::apply(&repo.root, &card).is_err());
    assert_eq!(repo.branch(), "main");
}

#[test]
fn push_card_is_distinct_and_pushes_to_the_named_remote() {
    let repo = Repo::new().with_remote();
    fs::write(repo.root.join("a.txt"), "a\n").unwrap();
    git(&repo.root, &["add", "a.txt"]);
    git(&repo.root, &["commit", "-q", "-m", "local work"]);
    let card = repo.card("git_push", json!({})).unwrap();
    assert_eq!(card.kind, CardKind::Push);
    assert!(card.summary.contains("origin"), "{}", card.summary);
    assert!(card.summary.contains("main"), "{}", card.summary);
    assert!(card.preview.contains("local work"), "{}", card.preview);
    approval::apply(&repo.root, &card).unwrap();
    assert_eq!(
        git(&repo.remote(), &["rev-parse", "refs/heads/main"]).trim(),
        repo.head()
    );
    let error = repo.card("git_push", json!({})).unwrap_err();
    assert!(error.contains("up to date"), "{error}");
}

#[test]
fn demo_merge_the_go_branch_to_main_and_push() {
    let repo = Repo::new().with_remote();
    repo.go_branch();
    repo.approve("git_merge", json!({"source": "alfredo/go-7"}))
        .unwrap();
    repo.approve("git_push", json!({"remote": "origin", "branch": "main"}))
        .unwrap();
    let remote = git(&repo.remote(), &["log", "--format=%s", "main"]);
    assert!(remote.contains("go: add feature"), "{remote}");
    assert_eq!(
        git(&repo.remote(), &["rev-parse", "refs/heads/main"]).trim(),
        repo.head()
    );
}

/// A gate approving every request and recording its cards.
fn approving_gate() -> (Gate, Arc<Mutex<Vec<Card>>>) {
    let (sender, mut receiver) = mpsc::channel::<approval::Request>(4);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let cards = seen.clone();
    tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            cards.lock().unwrap().push(request.card.clone());
            let _ = request.reply.send(Decision::Approve);
        }
    });
    (Gate::new(sender, None), seen)
}

async fn push_turn(root: &Path, gate: Option<Gate>) -> Session {
    let addr = ollama_fixture::reserve();
    let first = Mutex::new(Some(Reply::Stream(
        vec![
            format!(
                "{}\n",
                json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"git_push","arguments":{}}}]},"done":false})
            )
            .into_bytes(),
            format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )));
    let fixture = ollama_fixture::serve(addr, move |_, index| {
        if index == 0 {
            first.lock().unwrap().take().unwrap()
        } else {
            done("done")
        }
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let mut session = Session::new("fixture".into());
    session.insert("push");
    let messages = session.begin().unwrap();
    let (sender, mut receiver) = mpsc::channel::<Event>(256);
    chat_tools::run_turn(
        &provider,
        root,
        0,
        1,
        "fixture".into(),
        messages,
        sender,
        gate,
    )
    .await;
    while let Some(event) = receiver.recv().await {
        session.apply(event.attempt, event.update);
    }
    session
}

#[tokio::test]
async fn push_in_a_turn_always_asks_and_is_refused_without_a_gate() {
    let repo = Repo::new().with_remote();
    fs::write(repo.root.join("a.txt"), "a\n").unwrap();
    git(&repo.root, &["add", "a.txt"]);
    git(&repo.root, &["commit", "-q", "-m", "local work"]);
    let remote_before = git(&repo.remote(), &["rev-parse", "main"]);
    let session = push_turn(&repo.root, None).await;
    assert!(session.tool_steps(1)[0].results[0].starts_with("error: "));
    assert_eq!(git(&repo.remote(), &["rev-parse", "main"]), remote_before);

    let (gate, seen) = approving_gate();
    push_turn(&repo.root, Some(gate)).await;
    let cards = seen.lock().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].kind, CardKind::Push);
    assert_eq!(
        git(&repo.remote(), &["rev-parse", "main"]).trim(),
        repo.head()
    );
}

/// The screen text with `card` pending.
fn rendered(card: Card) -> String {
    let mut app = App::new("m".into());
    let session = &mut app.sessions[0];
    session.insert("go");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    assert!(session.await_approval(attempt, card, Instant::now()));
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

/// Draws `card` pending at `width`x`height`, waits out the arm delay and tries `y`, `a`
/// then `n`: (y, a, n, whole).
fn probe_card(
    card: Card,
    width: u16,
    height: u16,
) -> (Option<Decision>, Option<Decision>, Option<Decision>, bool) {
    let mut app = App::new("m".into());
    let session = &mut app.sessions[0];
    session.insert("go");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    assert!(session.await_approval(attempt, card, Instant::now()));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let start = Instant::now();
    for now in [start, start + approval::ARM * 2] {
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let session = &mut app.sessions[0];
        let drawn = session.approval_header_drawn();
        session.approval_visible(drawn, now);
    }
    let at = start + approval::ARM * 3;
    let session = &mut app.sessions[0];
    let whole = session.approval_whole();
    let y = session.answer_approval('y', at);
    let a = session.answer_approval('a', at);
    let n = session.answer_approval('n', at);
    (y, a, n, whole)
}

/// Issue #269: a commit message so long that the file list is pushed off screen makes
/// the card deny-only: the owner never approves a list they could not see.
#[test]
fn a_commit_card_with_a_huge_message_is_deny_only_and_commits_nothing() {
    let repo = Repo::new();
    add_files(&repo, 3);
    let head = repo.head();
    // The message cap is 4096 bytes: one long line, or 2000 one-letter lines.
    let long_line = "m".repeat(4000);
    let many_lines = "a\n".repeat(2000);
    for message in [long_line, many_lines] {
        let card = repo
            .card("git_commit", json!({"message": message}))
            .unwrap();
        for (w, h) in [(80, 24), (100, 24), (80, 40)] {
            let (y, a, n, whole) = probe_card(card.clone(), w, h);
            assert!(!whole, "{w}x{h}");
            assert_eq!((y, a), (None, None), "{w}x{h}");
            assert_eq!(n, Some(Decision::Deny), "{w}x{h}");
        }
        assert_eq!(repo.head(), head, "nothing was committed");
    }
}

/// Issue #269: a commit card that fits whole on screen still arms.
#[test]
fn a_commit_card_that_fits_still_arms() {
    let repo = Repo::new();
    add_files(&repo, 3);
    let card = repo
        .card("git_commit", json!({"message": "small"}))
        .unwrap();
    let (y, _, _, whole) = probe_card(card, 80, 24);
    assert!(whole);
    assert_eq!(y, Some(Decision::Approve));
}

/// Issue #269: `git diff --stat` shortens a long path to `.../ci.yml`, hiding which
/// directory a merged file lands in. The merge card lists the whole path, and a card
/// whose wrapped rows do not fit is deny-only.
#[test]
fn a_merge_card_shows_a_deep_long_path_whole_and_is_deny_only_when_it_cannot_fit() {
    let repo = Repo::new();
    let dir = ["configuration-directory-segment"; 12].join("/");
    let path = format!("{dir}/ci.yml");
    git(&repo.root, &["checkout", "-q", "-b", "alfredo/deep"]);
    fs::create_dir_all(repo.root.join(&dir)).unwrap();
    fs::write(repo.root.join(&path), "on: push\n").unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "-q", "-m", "deep file"]);
    git(&repo.root, &["checkout", "-q", "main"]);
    let head = repo.head();
    let card = repo
        .card("git_merge", json!({"source": "alfredo/deep"}))
        .unwrap();
    assert!(card.preview.contains(&path), "{}", card.preview);
    assert!(!card.preview.contains(".../"), "{}", card.preview);
    let (y, _, _, whole) = probe_card(card.clone(), 120, 40);
    assert!(whole);
    assert_eq!(y, Some(Decision::Approve));
    // A path wider than the terminal wraps; when the wrapped card does not fit, y is off.
    let (y, a, n, whole) = probe_card(card, 40, 24);
    assert!(!whole);
    assert_eq!((y, a), (None, None));
    assert_eq!(n, Some(Decision::Deny));
    assert_eq!(repo.head(), head, "nothing was merged");
}

/// Commits `files` (path, content) on a branch `name` off main and returns to main.
fn branch_with_paths(repo: &Repo, name: &str, files: &[String]) {
    git(&repo.root, &["checkout", "-q", "-b", name]);
    for file in files {
        let target = repo.root.join(file);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, "x\n").unwrap();
    }
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "-q", "-m", "paths"]);
    git(&repo.root, &["checkout", "-q", "main"]);
}

/// Issue #269 (round 3): a file list git prints past the read limit was cut silently, so
/// the payload files after the long lines never reached the card.
#[test]
fn a_merge_file_list_past_the_read_limit_never_hides_the_files_after_it() {
    let repo = Repo::new();
    let part = format!("a{}", "\u{301}".repeat(124));
    let deep = vec![part; 14].join("/");
    let mut files: Vec<String> = (0..12).map(|i| format!("{i:02}{deep}/f")).collect();
    files.push("zz/.github/workflows/ci.yml".into());
    files.push("zz-payload.sh".into());
    branch_with_paths(&repo, "alfredo/long", &files);
    let head = repo.head();
    match repo.card("git_merge", json!({"source": "alfredo/long"})) {
        Err(_) => {}
        Ok(card) => {
            assert!(card.preview.contains("zz-payload.sh"), "payload hidden");
            assert!(card.preview.contains("zz/.github/workflows/ci.yml"));
            let (y, a, _, whole) = probe_card(card, 80, 24);
            assert!(!whole);
            assert_eq!((y, a), (None, None));
        }
    }
    assert_eq!(repo.head(), head);
}

/// Issue #269 (round 3): git also shortens a name to `.../` when control bytes widen it.
#[test]
fn a_merge_path_full_of_control_bytes_keeps_its_directory_on_the_card() {
    let repo = Repo::new();
    let part = "\u{1}".repeat(250);
    let path = format!(".github/workflows/{part}/{part}/{part}/{part}");
    branch_with_paths(&repo, "alfredo/ctl", &[path]);
    let head = repo.head();
    {
        let card = repo
            .card("git_merge", json!({"source": "alfredo/ctl"}))
            .unwrap();
        assert!(
            card.preview.contains(".github/workflows/"),
            "{}",
            card.preview
        );
        assert!(!card.preview.contains(".../"), "{}", card.preview);
        assert!(
            !card.preview.contains('\u{1}'),
            "raw control byte on the card"
        );
    }
    assert_eq!(repo.head(), head);
}

#[test]
fn git_and_push_cards_render_their_effect_and_push_is_distinct() {
    let repo = Repo::new().with_remote();
    repo.go_branch();
    let text = rendered(
        repo.card("git_merge", json!({"source": "alfredo/go-7"}))
            .unwrap(),
    );
    assert!(text.contains("Approve git_merge"), "{text}");
    assert!(text.contains("go: add feature"), "{text}");
    assert!(!text.contains("PUSH"), "{text}");
    let text = rendered(
        repo.card("git_push", json!({"branch": "alfredo/go-7"}))
            .unwrap(),
    );
    assert!(text.contains("PUSH git_push"), "{text}");
    assert!(text.contains("never always-allowed"), "{text}");
    assert!(!text.contains("Approve git_push"), "{text}");
}

#[test]
fn push_card_shows_the_push_url() {
    let repo = Repo::new().with_remote();
    let other = repo.base.join("pushed.git");
    git(&repo.base, &["init", "-q", "--bare", "pushed.git"]);
    git(
        &repo.root,
        &["config", "remote.origin.pushurl", &ssh_url(&other)],
    );
    fs::write(repo.root.join("a.txt"), "a\n").unwrap();
    git(&repo.root, &["add", "a.txt"]);
    git(&repo.root, &["commit", "-q", "-m", "local work"]);
    let card = repo.card("git_push", json!({})).unwrap();
    assert!(card.preview.contains("pushed.git"), "{}", card.preview);
}

/// Issue #267: ssh (and some credential helpers) open /dev/tty for host-key and passphrase
/// prompts, so the push child must not share the TUI's controlling terminal.
#[cfg(unix)]
#[test]
fn push_child_has_no_controlling_terminal_and_names_the_ssh_prompt() {
    use std::os::unix::fs::PermissionsExt;
    let _ssh = SSH.lock().unwrap_or_else(|error| error.into_inner());
    let repo = Repo::new();
    git(
        &repo.root,
        &[
            "remote",
            "add",
            "origin",
            "ssh://git@fake-host.invalid/o/r.git",
        ],
    );
    let marker = repo.base.join("fakessh-session");
    let script = repo.base.join("fakessh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ncut -d' ' -f6 /proc/$$/stat > '{}'\n\
             if (exec 3<>/dev/tty) 2>/dev/null; then\n\
             echo 'FAKESSH-PROMPT: continue connecting (yes/no)?' > /dev/tty\n\
             read answer < /dev/tty\nfi\n\
             echo 'Host key verification failed.' >&2\nexit 255\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(repo.root.join("a.txt"), "a\n").unwrap();
    git(&repo.root, &["add", "a.txt"]);
    git(&repo.root, &["commit", "-q", "-m", "local work"]);
    let card = repo.card("git_push", json!({})).unwrap();
    std::env::set_var("GIT_SSH", &script);
    let started = Instant::now();
    let result = approval::apply(&repo.root, &card);
    std::env::remove_var("GIT_SSH");
    assert!(started.elapsed() < Duration::from_secs(20), "push hung");
    let session: i32 = fs::read_to_string(&marker).unwrap().trim().parse().unwrap();
    // SAFETY: getsid(0) only reads the caller's session id.
    let ours = unsafe { libc::getsid(0) };
    assert_ne!(session, ours, "the ssh child shares the TUI's session");
    let error = result.unwrap_err();
    assert!(error.contains("ssh needed a terminal prompt"), "{error}");
    assert!(error.contains("ssh-agent"), "{error}");
}

#[test]
fn push_is_refused_when_repository_config_can_run_commands() {
    for (key, value) in [
        ("core.sshCommand", "touch MARK"),
        ("core.askPass", "/bin/true"),
        ("credential.helper", "!touch MARK"),
        ("credential.https://x.helper", "store"),
        ("remote.origin.receivepack", "touch MARK"),
        ("remote.origin.uploadpack", "touch MARK"),
        ("http.proxy", "http://x"),
        ("https.proxy", "http://x"),
        ("url./tmp/x.insteadOf", "/tmp/y"),
        ("url./tmp/x.pushInsteadOf", "/tmp/y"),
        ("protocol.ext.allow", "always"),
        ("protocol.allow", "always"),
        ("core.gitProxy", "touch MARK"),
        ("remote.origin.proxy", "http://127.0.0.1:9"),
        ("http.sslVerify", "false"),
        ("http.sslCAInfo", "/tmp/x"),
        ("remote.origin.vcs", "x"),
        ("push.followTags", "true"),
        ("push.recurseSubmodules", "on-demand"),
        ("core.hooksPath", "/tmp/hooks"),
    ] {
        let repo = Repo::new().with_remote();
        fs::write(repo.root.join("a.txt"), "a\n").unwrap();
        git(&repo.root, &["add", "a.txt"]);
        git(&repo.root, &["commit", "-q", "-m", "local work"]);
        git(&repo.root, &["config", key, value]);
        let error = repo.card("git_push", json!({})).unwrap_err();
        assert!(
            error.to_lowercase().contains(&key.to_lowercase()),
            "{key}: {error}"
        );
    }
}

/// A commit of a new file `a.txt`, ahead of the remote.
fn commit_a_file(repo: &Repo) {
    fs::write(repo.root.join("a.txt"), "a\n").unwrap();
    git(&repo.root, &["add", "a.txt"]);
    git(&repo.root, &["commit", "-q", "-m", "local work"]);
}

/// An executable `#!/bin/sh` script at `path` running `body`.
fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Sets HOME to an empty directory for one test and restores it on drop. It is process-wide
/// (the environment is shared), so only the commit key test uses it: a signing key in the
/// owner's global config would stop git from running the key command that test checks.
struct EmptyHome(Option<std::ffi::OsString>);

impl EmptyHome {
    fn new(base: &Path) -> Self {
        let home = base.join("home");
        fs::create_dir_all(&home).unwrap();
        let old = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);
        Self(old)
    }
}

impl Drop for EmptyHome {
    fn drop(&mut self) {
        match self.0.take() {
            Some(old) => std::env::set_var("HOME", old),
            None => std::env::remove_var("HOME"),
        }
    }
}

/// #309 a: a signing key command in the repository config would run on commit.
#[test]
fn a_commit_never_runs_a_repository_signing_key_command() {
    let repo = Repo::new();
    let _home = EmptyHome::new(&repo.base);
    let marker = repo.base.join("KEYCMD_MARK");
    let keycmd = repo.base.join("keycmd");
    script(&keycmd, &format!("touch '{}'; exit 1", marker.display()));
    git(&repo.root, &["config", "commit.gpgsign", "true"]);
    git(&repo.root, &["config", "gpg.format", "ssh"]);
    git(
        &repo.root,
        &[
            "config",
            "gpg.ssh.defaultKeyCommand",
            keycmd.to_str().unwrap(),
        ],
    );
    fs::write(repo.root.join("b.txt"), "b\n").unwrap();
    let before = repo.head();
    let result = repo.approve("git_commit", json!({"message": "b", "files": ["b.txt"]}));
    assert!(!marker.exists(), "the repository signing key command ran");
    assert!(result.is_err(), "{result:?}");
    assert_eq!(repo.head(), before);
}

/// #309 b: core.gitProxy runs a program before any git:// connection.
#[test]
fn a_push_never_runs_a_repository_git_proxy() {
    let repo = Repo::new();
    commit_a_file(&repo);
    let marker = repo.base.join("PROXY_MARK");
    let proxy = repo.base.join("proxy");
    script(&proxy, &format!("touch '{}'; exit 1", marker.display()));
    git(
        &repo.root,
        &["config", "core.gitProxy", proxy.to_str().unwrap()],
    );
    git(
        &repo.root,
        &["remote", "add", "origin", "git://127.0.0.1:9/x.git"],
    );
    let result = repo.approve("git_push", json!({}));
    assert!(!marker.exists(), "the repository git proxy ran");
    assert!(result.is_err(), "{result:?}");
}

/// #309 c: a local-path remote runs its own hooks (repository config does not reach them).
#[test]
fn a_push_to_a_local_path_never_runs_the_remote_hooks() {
    let repo = Repo::new();
    commit_a_file(&repo);
    git(
        &repo.base,
        &["init", "-q", "--bare", "-b", "main", "hooked.git"],
    );
    let bare = repo.base.join("hooked.git");
    let marker = repo.base.join("HOOK_MARK");
    script(
        &bare.join("hooks/pre-receive"),
        &format!("touch '{}'", marker.display()),
    );
    git(
        &repo.root,
        &["remote", "add", "origin", bare.to_str().unwrap()],
    );
    let result = repo.approve("git_push", json!({}));
    assert!(!marker.exists(), "the remote pre-receive hook ran");
    assert!(result.is_err(), "{result:?}");
    assert_eq!(git(&bare, &["for-each-ref", "--format=%(refname)"]), "");
}

/// #309 d: a repository proxy and a TLS override would send the push through a
/// man-in-the-middle while the card shows github.com.
#[test]
fn a_push_never_goes_through_a_repository_proxy_or_tls_override() {
    let repo = Repo::new();
    commit_a_file(&repo);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            let _ = sender.send(());
            drop(stream);
        }
    });
    git(
        &repo.root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/owner/repo.git",
        ],
    );
    git(
        &repo.root,
        &[
            "config",
            "remote.origin.proxy",
            &format!("http://127.0.0.1:{port}"),
        ],
    );
    git(&repo.root, &["config", "http.sslVerify", "false"]);
    let result = repo.approve("git_push", json!({}));
    let connected = receiver.recv_timeout(Duration::from_secs(2)).is_ok();
    assert!(!connected, "the push reached the repository proxy");
    assert!(result.is_err(), "{result:?}");
}

/// #309 f: push.followTags sends an annotated tag the card does not list. The repository
/// config route is refused by the allowlist; `--no-follow-tags` covers the owner's own
/// global config, which is trusted and read by the push.
#[test]
fn a_push_never_sends_tags_the_card_does_not_list() {
    let repo = Repo::new().with_remote();
    commit_a_file(&repo);
    git(&repo.root, &["tag", "-a", "v1", "-m", "release"]);
    git(&repo.root, &["config", "push.followTags", "true"]);
    let result = repo.approve("git_push", json!({}));
    assert!(
        !git(&repo.remote(), &["tag"]).contains("v1"),
        "a tag the card does not list was pushed"
    );
    assert!(result.is_err(), "{result:?}");
}

/// #309 g: the remote is repointed after the card; the apply must not push elsewhere.
#[test]
fn a_remote_repointed_after_the_card_is_refused_and_nothing_is_pushed() {
    let repo = Repo::new().with_remote();
    commit_a_file(&repo);
    let card = repo.card("git_push", json!({})).unwrap();
    let other = repo.base.join("other.git");
    git(
        &repo.base,
        &["init", "-q", "--bare", "-b", "main", "other.git"],
    );
    git(
        &repo.root,
        &["remote", "set-url", "origin", &ssh_url(&other)],
    );
    let error = approval::apply(&repo.root, &card).unwrap_err();
    assert!(error.contains("changed"), "{error}");
    assert_eq!(git(&other, &["for-each-ref", "--format=%(refname)"]), "");
    assert_eq!(
        git(&repo.remote(), &["rev-parse", "refs/heads/main"]).trim(),
        git(&repo.root, &["rev-parse", "origin/main"]).trim()
    );
}

/// Criterion 4: a repository config change between the card and the apply is refused.
#[test]
fn a_repository_config_change_after_the_card_is_refused() {
    let repo = Repo::new();
    fs::write(repo.root.join("b.txt"), "b\n").unwrap();
    let card = repo
        .card("git_commit", json!({"message": "b", "files": ["b.txt"]}))
        .unwrap();
    let before = repo.head();
    git(&repo.root, &["config", "core.autocrlf", "true"]);
    let error = approval::apply(&repo.root, &card).unwrap_err();
    assert!(error.contains("changed"), "{error}");
    assert_eq!(repo.head(), before);
}

/// Criterion 2: git_push takes only https://, ssh:// and scp-style remotes.
#[test]
fn push_remotes_must_be_https_ssh_or_scp_style() {
    let repo = Repo::new();
    commit_a_file(&repo);
    git(
        &repo.root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/owner/repo.git",
        ],
    );
    for url in [
        "https://github.com/owner/repo.git",
        "https://user@github.com/owner/repo.git",
        "ssh://git@github.com/owner/repo.git",
        "git@github.com:owner/repo.git",
        "github.com:owner/repo.git",
    ] {
        git(&repo.root, &["remote", "set-url", "origin", url]);
        repo.card("git_push", json!({}))
            .unwrap_or_else(|error| panic!("{url} should be accepted: {error}"));
    }
    for url in [
        "/tmp/elsewhere.git",
        "../elsewhere.git",
        "file:///tmp/elsewhere.git",
        "git://127.0.0.1:9/x.git",
        "http://example.com/x.git",
        "ext::sh -c touch MARK",
        "fd::3",
        "ssh://-oProxyCommand=touch/x.git",
        "https://-evil.example/x.git",
        // libcurl ends the authority at `?` or `#`: these resolve evil-probe.invalid.
        "https://evil-probe.invalid?@github.com/o/r.git",
        "https://evil-probe.invalid#@github.com/o/r.git",
        // More than one `@`, and a backslash, where the host is read differently.
        "https://a@b@github.com/x",
        "https://github.com\\@evil/x",
        "ssh://a@b@github.com/o/r.git",
        "git@github.com@evil:o/r.git",
        // scp-style: git reads the host before the first `:`, so these reach other hosts.
        "evil.invalid:x@github.com:o/r.git",
        "https:github.com/o/r.git",
        "evil:tok@github.com:o/r.git",
        "git@github.com:o/r.git:x",
        "git@github.com:o/r@x",
    ] {
        git(&repo.root, &["remote", "set-url", "origin", url]);
        let error = repo.card("git_push", json!({})).unwrap_err();
        assert!(error.contains("is not https://"), "{url}: {error}");
        assert!(!error.contains("tok@"), "a token was printed: {error}");
    }
}

/// Repository config outside the allowlist is refused by every chat git write, by name.
#[test]
fn commit_refuses_repository_config_outside_the_allowlist() {
    for (key, value) in [
        ("core.pager", "touch MARK"),
        ("core.editor", "touch MARK"),
        ("diff.evil.command", "touch MARK"),
        ("core.hooksPath", "/tmp/hooks"),
        ("commit.gpgsign", "true"),
    ] {
        let repo = Repo::new();
        git(&repo.root, &["config", key, value]);
        fs::write(repo.root.join("b.txt"), "b\n").unwrap();
        let error = repo
            .card("git_commit", json!({"message": "b", "files": ["b.txt"]}))
            .unwrap_err();
        assert!(
            error.to_lowercase().contains(&key.to_lowercase()),
            "{key}: {error}"
        );
    }
}

/// A subsection name may hold `=`: `[remote "a.url=z"] vcs = probe` must not read as the
/// allowed key `remote.a.url`, so the listing is parsed as NUL-separated records (#309).
#[test]
fn a_config_subsection_holding_an_equals_sign_is_refused_by_its_full_key() {
    let repo = Repo::new();
    let mut config = fs::OpenOptions::new()
        .append(true)
        .open(repo.root.join(".git/config"))
        .unwrap();
    write!(config, "[remote \"a.url=z\"]\n\tvcs = probe\n").unwrap();
    drop(config);
    fs::write(repo.root.join("b.txt"), "b\n").unwrap();
    let error = repo
        .card("git_commit", json!({"message": "b", "files": ["b.txt"]}))
        .unwrap_err();
    assert!(error.contains("remote.a.url=z.vcs"), "{error}");
}

/// Keys the chat writes need for ordinary repositories stay allowed.
#[test]
fn ordinary_repository_config_is_still_allowed_for_writes() {
    let repo = Repo::new();
    git(&repo.root, &["config", "core.autocrlf", "false"]);
    git(&repo.root, &["config", "branch.main.description", "x"]);
    git(&repo.root, &["config", "pull.rebase", "true"]);
    git(&repo.root, &["config", "push.default", "current"]);
    fs::write(repo.root.join("b.txt"), "b\n").unwrap();
    repo.card("git_commit", json!({"message": "b", "files": ["b.txt"]}))
        .unwrap();
}

#[test]
fn repository_merge_drivers_and_gpg_programs_are_refused() {
    let repo = Repo::new();
    git(&repo.root, &["checkout", "-q", "-b", "alfredo/go-3"]);
    fs::write(repo.root.join("README.md"), "# Go\n").unwrap();
    git(&repo.root, &["commit", "-q", "-am", "go"]);
    git(&repo.root, &["checkout", "-q", "main"]);
    fs::write(repo.root.join("README.md"), "# Main\n").unwrap();
    git(&repo.root, &["commit", "-q", "-am", "main"]);
    fs::write(repo.root.join(".gitattributes"), "README.md merge=evil\n").unwrap();
    git(&repo.root, &["add", ".gitattributes"]);
    git(&repo.root, &["commit", "-q", "-m", "attrs"]);
    git(
        &repo.root,
        &["config", "merge.evil.driver", "touch MERGE_MARK; false"],
    );
    let error = repo
        .approve("git_merge", json!({"source": "alfredo/go-3"}))
        .unwrap_err();
    assert!(error.contains("merge.evil.driver"), "{error}");
    assert!(!repo.root.join("MERGE_MARK").exists());
    git(&repo.root, &["config", "--unset", "merge.evil.driver"]);
    for key in ["gpg.program", "gpg.ssh.program"] {
        git(&repo.root, &["config", key, "/bin/true"]);
        fs::write(repo.root.join("b.txt"), "b\n").unwrap();
        let error = repo
            .card("git_commit", json!({"message": "b", "files": ["b.txt"]}))
            .unwrap_err();
        assert!(error.contains(key), "{error}");
        git(&repo.root, &["config", "--unset", key]);
    }
}

#[test]
fn large_listings_are_refused_rather_than_cut() {
    let repo = Repo::new();
    let long = "x".repeat(60);
    for index in 0..1500 {
        fs::write(repo.root.join(format!("{long}{index:05}.txt")), "u\n").unwrap();
    }
    git(&repo.root, &["add", "."]);
    let error = repo
        .card("git_commit", json!({"message": "many"}))
        .unwrap_err();
    assert!(
        error.contains("1500") && error.contains("40-line"),
        "{error}"
    );
}

/// Coverage kept from the old large-listing test (issue #269 made a long listing a
/// refusal): a change late in a huge `status` still invalidates an approved git card.
/// Passes on main and on the branch.
#[test]
fn a_change_late_in_a_huge_status_still_invalidates_the_approval() {
    let repo = Repo::new();
    let long = "x".repeat(60);
    for index in 0..1500 {
        fs::write(repo.root.join(format!("{long}{index:05}.txt")), "u\n").unwrap();
    }
    fs::write(repo.root.join("one.txt"), "one\n").unwrap();
    let card = repo
        .card(
            "git_commit",
            json!({"message": "one", "files": ["one.txt"]}),
        )
        .unwrap();
    // Past the first 32 KiB of status.
    fs::write(repo.root.join("zzzz-late.txt"), "late\n").unwrap();
    let head = repo.head();
    let error = approval::apply(&repo.root, &card).unwrap_err();
    assert!(error.contains("changed"), "{error}");
    assert_eq!(repo.head(), head);
}

#[test]
fn a_failed_commit_of_named_files_leaves_the_index_as_it_was() {
    let repo = Repo::new();
    fs::write(repo.root.join("new.txt"), "n\n").unwrap();
    let card = repo
        .card("git_commit", json!({"message": "n", "files": ["new.txt"]}))
        .unwrap();
    fs::write(repo.root.join(".git/refs/heads/main.lock"), "").unwrap();
    assert!(approval::apply(&repo.root, &card).is_err());
    fs::remove_file(repo.root.join(".git/refs/heads/main.lock")).unwrap();
    let status = git(&repo.root, &["status", "--porcelain"]);
    assert_eq!(status, "?? new.txt\n");
}

#[test]
fn merge_with_uncommitted_tracked_changes_is_refused() {
    let repo = Repo::new();
    repo.go_branch();
    fs::write(repo.root.join("README.md"), "# Dirty\n").unwrap();
    let error = repo
        .card("git_merge", json!({"source": "alfredo/go-7"}))
        .unwrap_err();
    assert!(error.contains("uncommitted"), "{error}");
    git(&repo.root, &["add", "README.md"]);
    assert!(repo
        .card("git_merge", json!({"source": "alfredo/go-7"}))
        .is_err());
}
