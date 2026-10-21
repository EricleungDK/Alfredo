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
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

static ID: AtomicU64 = AtomicU64::new(0);

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
        Self { base, root }
    }

    /// A local bare remote named origin with main pushed.
    fn with_remote(self) -> Self {
        let bare = self.base.join("remote.git");
        git(
            &self.base,
            &["init", "-q", "--bare", "-b", "main", "remote.git"],
        );
        git(
            &self.root,
            &["remote", "add", "origin", bare.to_str().unwrap()],
        );
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
        &["config", "remote.origin.pushurl", other.to_str().unwrap()],
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
fn large_listings_are_marked_and_fingerprinted_in_full() {
    let repo = Repo::new();
    let long = "x".repeat(60);
    for index in 0..1500 {
        fs::write(repo.root.join(format!("{long}{index:05}.txt")), "u\n").unwrap();
    }
    git(&repo.root, &["add", "."]);
    let card = repo.card("git_commit", json!({"message": "many"})).unwrap();
    assert!(card.preview.contains("more"), "{}", card.preview);
    assert!(card.summary.contains("1500 files"), "{}", card.summary);
    // A change past the first 32 KiB of status still counts.
    fs::write(repo.root.join("zzzz-late.txt"), "late\n").unwrap();
    let error = approval::apply(&repo.root, &card).unwrap_err();
    assert!(error.contains("changed"), "{error}");
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
