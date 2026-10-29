//! Issue #234: read-only git tools for the Chat Agent, on temp git repositories.
use alfredo_tui::{
    chat_tools::{self, Tier, MAX_OUTPUT},
    model::ToolCall,
};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

static ID: AtomicU64 = AtomicU64::new(0);

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            // Issue #448: no automatic maintenance. `git commit` runs `maintenance run --auto`,
            // which may prune `.git/worktrees/*` entries that still lack `gitdir`/`index`.
            "-c",
            "gc.auto=0",
            "-c",
            "gc.autoDetach=false",
            "-c",
            "maintenance.auto=false",
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

/// Issue #448: install a hand-built linked-worktree entry (`HEAD`, `commondir`, `gitdir`) as
/// `<git_dir>/worktrees/<name>` in one rename. An entry directory without a `gitdir` file and
/// without an `index` is "prunable" at once, whatever the expiry, so any `git worktree prune`,
/// `gc` or auto-maintenance run on the repository would delete it half-built.
fn install_worktree_entry(git_dir: &Path, name: &str, head: &str, gitdir: &str) -> PathBuf {
    let staging = git_dir.join(format!("staging-{name}"));
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("HEAD"), head).unwrap();
    fs::write(staging.join("commondir"), "../..\n").unwrap();
    fs::write(staging.join("gitdir"), gitdir).unwrap();
    fs::create_dir_all(git_dir.join("worktrees")).unwrap();
    let entry = git_dir.join("worktrees").join(name);
    fs::rename(&staging, &entry).unwrap();
    entry
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

/// Issue #520: with `eol=crlf` every changed LF file makes git print a warning, and 300 of
/// them are more than 4 KiB of stderr. That used to kill git (SIGPIPE) and fail the diff.
#[test]
fn git_diff_of_an_eol_crlf_repository_with_many_warnings_succeeds() {
    let repo = Repo::new();
    fs::write(repo.root.join(".gitattributes"), "* text eol=crlf\n").unwrap();
    for index in 0..300 {
        fs::write(
            repo.root.join(format!("file_with_a_long_name_{index}.txt")),
            "x\n",
        )
        .unwrap();
    }
    git(&repo.root, &["add", "-A"]);
    git(&repo.root, &["commit", "-q", "-m", "lf files"]);
    for index in 0..300 {
        let path = repo.root.join(format!("file_with_a_long_name_{index}.txt"));
        fs::write(path, "x\ny\n").unwrap();
    }
    let output = repo.run("git_diff", json!({}));
    assert!(!output.starts_with("error: "), "{output}");
    assert!(output.contains("+y"), "{output}");
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

#[test]
fn repository_filters_never_run() {
    let repo = Repo::new();
    let marker = repo.base.join("filter-ran");
    let command = format!("touch {}; cat", marker.display());
    git(&repo.root, &["config", "filter.f.clean", &command]);
    fs::write(repo.root.join(".gitattributes"), "* filter=f\n").unwrap();
    fs::write(repo.root.join("README.md"), "# Demo\ndirty\n").unwrap();
    let _ = fs::remove_file(&marker);
    for (tool, arguments) in [("git_status", json!({})), ("git_diff", json!({}))] {
        let output = repo.run(tool, arguments);
        assert!(output.starts_with("error: "), "{tool}: {output}");
    }
    assert!(!marker.exists(), "repository filter ran");
}

/// Appends a `[pad]` section of more than 1 MiB and then a `[filter "f"]` clean driver
/// running `command`: the #308 attack, where the padding cuts a `git config` listing
/// before the driver.
fn append_padded_filter(root: &Path, command: &str) {
    let mut config = fs::OpenOptions::new()
        .append(true)
        .open(root.join(".git/config"))
        .unwrap();
    let padding = "v".repeat(1_200_000);
    write!(
        config,
        "[pad]\n\tk = {padding}\n[filter \"f\"]\n\tclean = {command}\n"
    )
    .unwrap();
}

#[test]
fn padded_repository_config_never_hides_a_filter() {
    let repo = Repo::new();
    let marker = repo.base.join("padded-filter-ran");
    append_padded_filter(&repo.root, &format!("touch {}; cat", marker.display()));
    fs::write(repo.root.join(".gitattributes"), "* filter=f\n").unwrap();
    fs::write(repo.root.join("README.md"), "# Demo\ndirty\n").unwrap();
    let _ = fs::remove_file(&marker);
    for (tool, arguments) in [("git_status", json!({})), ("git_diff", json!({}))] {
        let output = repo.run(tool, arguments);
        assert!(output.starts_with("error: "), "{tool}: {output}");
        assert!(output.contains("filter drivers"), "{tool}: {output}");
    }
    assert!(!marker.exists(), "repository filter ran");
}

#[test]
fn filter_and_include_keys_in_other_spellings_or_scopes_are_refused() {
    for snippet in [
        "[Filter \"f\"]\n\tClean = cat\n",
        "[include]\n\tpath = /nonexistent/x\n",
        "[includeIf \"gitdir:/nonexistent/\"]\n\tpath = /nonexistent/x\n",
    ] {
        let repo = Repo::new();
        let mut config = fs::OpenOptions::new()
            .append(true)
            .open(repo.root.join(".git/config"))
            .unwrap();
        write!(config, "{snippet}").unwrap();
        drop(config);
        let output = repo.run("git_status", json!({}));
        assert!(output.contains("filter drivers"), "{snippet}: {output}");
    }
    let repo = Repo::new();
    git(&repo.root, &["config", "extensions.worktreeConfig", "true"]);
    git(
        &repo.root,
        &["config", "--worktree", "filter.w.clean", "cat"],
    );
    let output = repo.run("git_status", json!({}));
    assert!(output.contains("filter drivers"), "worktree: {output}");
}

/// Issue #496: a repository with a committed submodule `sub` whose per-submodule
/// `submodule.sub.ignore=none` (set in the superproject's `.git/config`) overrides the
/// global `diff.ignoreSubmodules=all`, so git would run `status` inside `sub` and read the
/// submodule's own config (`.git/modules/sub/config`), which no gate checks. Returns the
/// submodule's own config path; `README.md` of the superproject is left clean.
fn repo_with_ignore_none_submodule(repo: &Repo) -> PathBuf {
    let source = repo.base.join("sub-source");
    fs::create_dir_all(&source).unwrap();
    git(&source, &["init", "-q", "-b", "main"]);
    fs::write(source.join("lib.txt"), "lib\n").unwrap();
    git(&source, &["add", "."]);
    git(&source, &["commit", "-q", "-m", "sub initial"]);
    let url = source.display().to_string();
    git(
        &repo.root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &url,
            "sub",
        ],
    );
    git(&repo.root, &["commit", "-q", "-m", "add submodule"]);
    git(&repo.root, &["config", "submodule.sub.ignore", "none"]);
    // A modified, tracked file in the submodule that a `* filter=x` attribute applies to.
    fs::write(repo.root.join("sub/.gitattributes"), "* filter=x\n").unwrap();
    // Same size, different mtime: git must read the content (and so run the clean filter) to
    // know whether the file changed.
    let changed = repo.root.join("sub/lib.txt");
    fs::write(&changed, "bil\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&changed)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
        .unwrap();
    repo.root.join(".git/modules/sub/config")
}

fn append(path: &Path, text: &str) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    write!(file, "{text}").unwrap();
}

/// Every working-tree-reading form of the two tools that run git over the working tree.
fn submodule_exposed_calls() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("git_status", json!({})),
        ("git_diff", json!({})),
        ("git_diff", json!({"path": "sub"})),
        ("git_diff", json!({"base": "HEAD"})),
        ("git_diff", json!({"staged": true})),
        ("git_diff", json!({"staged": true, "base": "HEAD"})),
        ("git_diff", json!({"base": "HEAD", "target": "HEAD"})),
    ]
}

#[test]
fn a_submodule_filter_never_runs_even_with_ignore_none() {
    let repo = Repo::new();
    let config = repo_with_ignore_none_submodule(&repo);
    let marker = repo.base.join("submodule-filter-ran");
    append(
        &config,
        &format!(
            "[filter \"x\"]\n\tclean = touch {}; cat\n",
            marker.display()
        ),
    );
    for (tool, arguments) in submodule_exposed_calls() {
        let _ = fs::remove_file(&marker);
        let output = repo.run(tool, arguments.clone());
        assert!(
            !marker.exists(),
            "{tool} {arguments}: submodule filter ran: {output}"
        );
    }
}

#[test]
fn a_submodule_config_fifo_include_never_stalls_status_or_diff() {
    let repo = Repo::new();
    let config = repo_with_ignore_none_submodule(&repo);
    let fifo = repo.base.join("sub-include-fifo");
    mkfifo(&fifo);
    append(
        &config,
        &format!("[include]\n\tpath = {}\n", fifo.display()),
    );
    for (tool, arguments) in submodule_exposed_calls() {
        let (sender, receiver) = mpsc::channel();
        let (root, name, args) = (repo.root.clone(), tool.to_string(), arguments.clone());
        std::thread::spawn(move || {
            let output = chat_tools::dispatch(
                &root,
                &ToolCall {
                    name,
                    arguments: args,
                },
            );
            let _ = sender.send(output);
        });
        receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap_or_else(|_| {
                panic!("{tool} {arguments} stalled on the submodule's FIFO include")
            });
    }
}

#[test]
fn an_ordinary_submodule_still_gives_status_and_diff() {
    let repo = Repo::new();
    repo_with_ignore_none_submodule(&repo);
    git(&repo.root, &["config", "--unset", "submodule.sub.ignore"]);
    fs::remove_file(repo.root.join("sub/.gitattributes")).unwrap();
    fs::write(repo.root.join("README.md"), "# Demo\nchanged\n").unwrap();
    let status = repo.run("git_status", json!({}));
    assert!(!status.starts_with("error: "), "{status}");
    assert!(status.contains("M README.md"), "{status}");
    let diff = repo.run("git_diff", json!({}));
    assert!(!diff.starts_with("error: "), "{diff}");
    assert!(diff.contains("+changed"), "{diff}");
    let ignored = repo.run("git_diff", json!({"base": "HEAD"}));
    assert!(ignored.contains("+changed"), "{ignored}");
}

#[test]
fn padding_without_a_filter_does_not_block_git_status() {
    let repo = Repo::new();
    append_padded_filter(&repo.root, "cat");
    let config = fs::read_to_string(repo.root.join(".git/config")).unwrap();
    fs::write(
        repo.root.join(".git/config"),
        config.replace("[filter \"f\"]", "[other \"f\"]"),
    )
    .unwrap();
    fs::write(repo.root.join("README.md"), "changed\n").unwrap();
    let output = repo.run("git_status", json!({}));
    assert!(!output.starts_with("error: "), "{output}");
    assert!(output.contains("M README.md"), "{output}");
}

#[test]
fn non_repository_inside_a_parent_repository_is_an_error() {
    let repo = Repo::new();
    let child = repo.root.join("plain");
    fs::create_dir_all(&child).unwrap();
    for tool in ["git_status", "git_branch", "git_log"] {
        let output = chat_tools::dispatch(
            &child,
            &ToolCall {
                name: tool.into(),
                arguments: json!({}),
            },
        );
        assert!(output.starts_with("error: "), "{tool}: {output}");
    }
}

/// Issue #359: a workspace whose `.git` leads to another repository (a gitfile, a `commondir`
/// file behind a gitfile, or a `commondir` in a plain `.git` directory) shows none of it.
#[test]
fn a_git_pointer_to_another_repository_is_never_read() {
    let other = Repo::new();
    fs::write(other.root.join("creds.txt"), "AWS_SECRET=hunter2\n").unwrap();
    git(&other.root, &["add", "creds.txt"]);
    git(&other.root, &["commit", "-q", "-m", "add creds"]);
    let other_git = other.root.join(".git");
    let branch = git(&other.root, &["symbolic-ref", "--short", "HEAD"]);
    let head = format!("ref: refs/heads/{}\n", branch.trim());
    let ws = other.base.join("ws");
    for name in [
        "gitfile",
        "commondir behind a gitfile",
        "commondir in a .git directory",
    ] {
        let _ = fs::remove_dir_all(&ws);
        fs::create_dir_all(&ws).unwrap();
        match name {
            "gitfile" => fs::write(
                ws.join(".git"),
                format!("gitdir: {}\n", other_git.display()),
            )
            .unwrap(),
            "commondir behind a gitfile" => {
                fs::write(ws.join(".git"), "gitdir: ./fake\n").unwrap();
                fs::create_dir_all(ws.join("fake")).unwrap();
                fs::write(ws.join("fake/HEAD"), &head).unwrap();
                fs::write(
                    ws.join("fake/commondir"),
                    format!("{}\n", other_git.display()),
                )
                .unwrap();
            }
            _ => {
                fs::create_dir_all(ws.join(".git")).unwrap();
                fs::write(ws.join(".git/HEAD"), &head).unwrap();
                fs::write(
                    ws.join(".git/commondir"),
                    format!("{}\n", other_git.display()),
                )
                .unwrap();
            }
        }
        for (tool, arguments) in [
            ("git_status", json!({})),
            ("git_branch", json!({})),
            ("git_log", json!({"max_count": 5})),
            ("git_diff", json!({})),
            ("git_diff", json!({"staged": true})),
            ("git_diff", json!({"base": "HEAD~1", "target": "HEAD"})),
        ] {
            let output = chat_tools::dispatch(
                &ws,
                &ToolCall {
                    name: tool.into(),
                    arguments,
                },
            );
            assert!(!output.contains("hunter2"), "{name} {tool}: {output}");
            assert!(!output.contains("add creds"), "{name} {tool}: {output}");
            assert!(output.starts_with("error:"), "{name} {tool}: {output}");
        }
    }
}

/// Issue #359: a worktree entry of another repository whose `gitdir` back-reference does not
/// point at this workspace is not this workspace's linked worktree, whatever its HEAD says.
#[test]
fn a_worktree_entry_of_another_repository_that_is_not_ours_is_never_read() {
    let other = Repo::new();
    fs::write(other.root.join("creds.txt"), "AWS_SECRET=hunter2\n").unwrap();
    git(&other.root, &["add", "creds.txt"]);
    git(&other.root, &["commit", "-q", "-m", "add creds"]);
    let branch = git(&other.root, &["symbolic-ref", "--short", "HEAD"]);
    let entry = install_worktree_entry(
        &other.root.join(".git"),
        "evil",
        &format!("ref: refs/heads/{}\n", branch.trim()),
        &format!("{}/elsewhere/.git\n", other.base.display()),
    );
    let ws = other.base.join("ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join(".git"), format!("gitdir: {}\n", entry.display())).unwrap();
    for tool in ["git_status", "git_log"] {
        let output = chat_tools::dispatch(
            &ws,
            &ToolCall {
                name: tool.into(),
                arguments: json!({}),
            },
        );
        assert!(!output.contains("hunter2"), "{tool}: {output}");
        assert!(output.starts_with("error:"), "{tool}: {output}");
    }
}

/// Issue #359: a linked worktree of the workspace's own repository is still read.
#[test]
fn a_linked_worktree_of_the_workspace_repository_reads_its_own_history() {
    let repo = Repo::new();
    let wt = repo.base.join("wt");
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt-branch",
            wt.to_str().unwrap(),
        ],
    );
    let log = chat_tools::dispatch(
        &wt,
        &ToolCall {
            name: "git_log".into(),
            arguments: json!({}),
        },
    );
    assert!(log.contains("initial commit"), "{log}");
    let branch = chat_tools::dispatch(
        &wt,
        &ToolCall {
            name: "git_branch".into(),
            arguments: json!({}),
        },
    );
    assert!(branch.contains("current: wt-branch"), "{branch}");
}

/// Issue #359: a `.git` that is a symlink to another repository's git directory is not the
/// workspace's own repository, whatever it resolves to.
#[test]
fn a_symlinked_git_directory_to_another_repository_is_never_read() {
    let other = Repo::new();
    fs::write(other.root.join("creds.txt"), "AWS_SECRET=hunter2\n").unwrap();
    git(&other.root, &["add", "creds.txt"]);
    git(&other.root, &["commit", "-q", "-m", "add creds"]);
    let ws = other.base.join("ws");
    fs::create_dir_all(&ws).unwrap();
    std::os::unix::fs::symlink(other.root.join(".git"), ws.join(".git")).unwrap();
    for (tool, arguments) in [
        ("git_status", json!({})),
        ("git_log", json!({})),
        ("git_diff", json!({"staged": true})),
    ] {
        let output = chat_tools::dispatch(
            &ws,
            &ToolCall {
                name: tool.into(),
                arguments,
            },
        );
        assert!(!output.contains("hunter2"), "{tool}: {output}");
        assert!(output.starts_with("error:"), "{tool}: {output}");
    }
}

/// Issue #359 (review): a plain `.git` whose object store lists another repository's objects
/// in `objects/info/alternates` shows that repository's history with no gitfile at all.
#[test]
fn an_alternates_file_into_another_repository_is_never_read() {
    let other = Repo::new();
    fs::write(other.root.join("creds.txt"), "AWS_SECRET=hunter2\n").unwrap();
    git(&other.root, &["add", "creds.txt"]);
    git(&other.root, &["commit", "-q", "-m", "add creds"]);
    let sha = git(&other.root, &["rev-parse", "HEAD"]).trim().to_string();
    git(&other.base, &["init", "-q", "-b", "main", "ws"]);
    let ws = other.base.join("ws");
    fs::write(
        ws.join(".git/objects/info/alternates"),
        format!("{}\n", other.root.join(".git/objects").display()),
    )
    .unwrap();
    fs::write(ws.join(".git/refs/heads/main"), format!("{sha}\n")).unwrap();
    for (tool, arguments) in [
        ("git_status", json!({})),
        ("git_log", json!({})),
        ("git_diff", json!({"base": "HEAD~1", "target": "HEAD"})),
    ] {
        let output = chat_tools::dispatch(
            &ws,
            &ToolCall {
                name: tool.into(),
                arguments,
            },
        );
        assert!(!output.contains("hunter2"), "{tool}: {output}");
        assert!(!output.contains("add creds"), "{tool}: {output}");
        assert!(output.starts_with("error:"), "{tool}: {output}");
    }
}

/// Issue #359 (review 2): an object store that is a symlink into another repository, or a
/// pack directory that is one, shows that repository's history with no alternates at all.
#[test]
fn a_linked_object_store_or_pack_directory_into_another_repository_is_never_read() {
    for variant in ["objects symlink", "pack symlink"] {
        let other = Repo::new();
        fs::write(other.root.join("creds.txt"), "AWS_SECRET=hunter2\n").unwrap();
        git(&other.root, &["add", "creds.txt"]);
        git(&other.root, &["commit", "-q", "-m", "add creds"]);
        git(&other.root, &["repack", "-adq"]);
        let sha = git(&other.root, &["rev-parse", "HEAD"]).trim().to_string();
        let name = if variant == "pack symlink" {
            "ws-pack"
        } else {
            "ws-objects"
        };
        git(&other.base, &["init", "-q", "-b", "main", name]);
        let ws = other.base.join(name);
        let store = ws.join(".git/objects");
        if variant == "pack symlink" {
            fs::remove_dir_all(store.join("pack")).unwrap();
            std::os::unix::fs::symlink(other.root.join(".git/objects/pack"), store.join("pack"))
                .unwrap();
        } else {
            fs::remove_dir_all(&store).unwrap();
            std::os::unix::fs::symlink(other.root.join(".git/objects"), &store).unwrap();
        }
        fs::write(ws.join(".git/refs/heads/main"), format!("{sha}\n")).unwrap();
        let output = chat_tools::dispatch(
            &ws,
            &ToolCall {
                name: "git_log".into(),
                arguments: json!({}),
            },
        );
        assert!(!output.contains("add creds"), "{variant}: {output}");
        assert!(output.starts_with("error:"), "{variant}: {output}");
    }
}

/// Runs one tool call on its own thread and waits at most `limit`. A call that does not
/// return (a read that blocks on a FIFO) fails the test instead of hanging it (issue #359,
/// review 3). The stuck thread is left behind; the test process exits without it.
fn dispatch_within(root: &Path, tool: &str, arguments: serde_json::Value) -> String {
    let (sender, receiver) = mpsc::channel();
    let (root, name) = (root.to_path_buf(), tool.to_string());
    std::thread::spawn(move || {
        let output = chat_tools::dispatch(&root, &ToolCall { name, arguments });
        let _ = sender.send(output);
    });
    receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| panic!("{tool} did not return within 10 s: a FIFO blocked it"))
}

fn mkfifo(path: &Path) {
    assert!(Command::new("mkfifo").arg(path).status().unwrap().success());
}

/// Issue #359 (review 3): a FIFO where the object store's `info/alternates` is read would
/// block the read forever. The tool refuses at once, with an error, instead.
#[test]
fn an_alternates_fifo_refuses_promptly_and_never_hangs() {
    let repo = Repo::new();
    let alternates = repo.root.join(".git/objects/info/alternates");
    fs::create_dir_all(alternates.parent().unwrap()).unwrap();
    mkfifo(&alternates);
    for (tool, arguments) in [
        ("git_status", json!({})),
        ("git_log", json!({})),
        ("git_diff", json!({"base": "HEAD~1", "target": "HEAD"})),
    ] {
        let output = dispatch_within(&repo.root, tool, arguments);
        assert!(output.starts_with("error:"), "{tool}: {output}");
    }
}

/// Issue #458: git itself reads `HEAD`, `config`, `commondir` and (newer git, at start-up)
/// `objects/info/alternates` before any tool check could run, and a FIFO there blocks that
/// read until the call's 15 s limit. The files are type-checked first, so each refuses
/// at once (well inside the 10 s wait), with an error that names the file.
#[test]
fn a_fifo_for_a_file_git_reads_at_start_up_refuses_before_git_runs() {
    for file in ["HEAD", "config", "commondir", "objects/info/alternates"] {
        let repo = Repo::new();
        let path = repo.root.join(".git").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let _ = fs::remove_file(&path);
        mkfifo(&path);
        let name = file.rsplit('/').next().unwrap();
        for (tool, arguments) in [("git_status", json!({})), ("git_log", json!({}))] {
            let started = std::time::Instant::now();
            let output = dispatch_within(&repo.root, tool, arguments);
            assert!(output.starts_with("error:"), "{file} {tool}: {output}");
            assert!(
                output.contains(name) && !output.contains("took longer"),
                "{file} {tool} must refuse the FIFO, not wait for git: {output}"
            );
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{file} {tool} took {:?}",
                started.elapsed()
            );
        }
    }
}

/// Issue #458: the same holds for a store reached through `info/alternates` (git opens each
/// store's own alternates), and for a linked worktree's entry files.
#[test]
fn a_fifo_in_a_chained_store_or_a_worktree_entry_refuses_before_git_runs() {
    let repo = Repo::new();
    let second = repo.base.join("second-store");
    fs::create_dir_all(second.join("info")).unwrap();
    mkfifo(&second.join("info/alternates"));
    let alternates = repo.root.join(".git/objects/info/alternates");
    fs::create_dir_all(alternates.parent().unwrap()).unwrap();
    fs::write(&alternates, format!("{}\n", second.display())).unwrap();
    let output = dispatch_within(&repo.root, "git_log", json!({}));
    assert!(
        output.starts_with("error:") && !output.contains("took longer"),
        "{output}"
    );

    let repo = Repo::new();
    let wt = repo.base.join("wt");
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt-branch",
            wt.to_str().unwrap(),
        ],
    );
    for file in ["HEAD", "commondir"] {
        let path = repo.root.join(".git/worktrees/wt").join(file);
        let original = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        mkfifo(&path);
        let started = std::time::Instant::now();
        let output = dispatch_within(&wt, "git_log", json!({}));
        assert!(
            output.starts_with("error:") && !output.contains("took longer"),
            "{file}: {output}"
        );
        assert!(started.elapsed() < Duration::from_secs(5), "{file}");
        fs::remove_file(&path).unwrap();
        fs::write(&path, original).unwrap();
    }
}

/// Issue #359 (review 3): a linked worktree whose `gitdir` back-reference is a FIFO would
/// block the read of that back-reference forever. The tool refuses at once instead.
#[test]
fn a_worktree_back_reference_fifo_refuses_promptly_and_never_hangs() {
    let repo = Repo::new();
    let wt = repo.base.join("wt");
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt-branch",
            wt.to_str().unwrap(),
        ],
    );
    let back = repo.root.join(".git/worktrees/wt/gitdir");
    // Swap the FIFO in by rename: removing `gitdir` first would leave a prunable entry.
    let fifo = back.with_file_name("gitdir.fifo");
    mkfifo(&fifo);
    fs::rename(&fifo, &back).unwrap();
    for (tool, arguments) in [("git_status", json!({})), ("git_log", json!({}))] {
        let output = dispatch_within(&wt, tool, arguments);
        assert!(output.starts_with("error:"), "{tool}: {output}");
    }
}

/// Issue #461: like `dispatch_within`, but the answer must arrive in 3 s. A refusal made
/// before git runs is immediate; a git process stuck on a FIFO only gives up at 15 s.
fn dispatch_promptly(root: &Path, tool: &str) -> String {
    dispatch_promptly_with(root, tool, json!({}))
}

/// [`dispatch_promptly`] with call arguments.
fn dispatch_promptly_with(root: &Path, tool: &str, arguments: serde_json::Value) -> String {
    let (sender, receiver) = mpsc::channel();
    let (root, name) = (root.to_path_buf(), tool.to_string());
    std::thread::spawn(move || {
        let output = chat_tools::dispatch(&root, &ToolCall { name, arguments });
        let _ = sender.send(output);
    });
    receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap_or_else(|_| panic!("{tool} was not refused within 3 s: git blocked on a FIFO"))
}

const POINTS_OUTSIDE: &str = "points outside the Coding Workspace";

/// A workspace laid out as a bare repository (`objects/`, `refs/`, a FIFO `HEAD`) at its root.
fn bare_layout_with_fifo_head(dir: &Path) {
    fs::create_dir_all(dir.join("objects")).unwrap();
    fs::create_dir_all(dir.join("refs")).unwrap();
    mkfifo(&dir.join("HEAD"));
}

/// Issue #461 (1): with `.git` or `.git/HEAD` missing, git treats the workspace root as a
/// bare repository and blocks on its FIFO `HEAD`. A missing `.git` or `HEAD` is refused
/// before git runs, also without any bare layout.
#[test]
fn a_missing_dot_git_or_head_is_refused_before_git_runs() {
    let repo = Repo::new();
    fs::remove_dir_all(repo.root.join(".git")).unwrap();
    bare_layout_with_fifo_head(&repo.root);
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(
        output.starts_with("error:") && output.contains(POINTS_OUTSIDE),
        "{output}"
    );

    let repo = Repo::new();
    fs::remove_file(repo.root.join(".git/HEAD")).unwrap();
    bare_layout_with_fifo_head(&repo.root);
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(
        output.starts_with("error:") && output.contains(POINTS_OUTSIDE),
        "{output}"
    );

    // A `.git` that is not a repository (no `refs/`) makes git look at the root as bare, too.
    let repo = Repo::new();
    fs::remove_dir_all(repo.root.join(".git/refs")).unwrap();
    bare_layout_with_fifo_head(&repo.root);
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(
        output.starts_with("error:") && output.contains(POINTS_OUTSIDE),
        "{output}"
    );

    // A `.git` git rejects for its HEAD leaves the same fallback.
    let repo = Repo::new();
    fs::write(repo.root.join(".git/HEAD"), "garbage\n").unwrap();
    bare_layout_with_fifo_head(&repo.root);
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(
        output.starts_with("error:") && output.contains("HEAD that is not an ordinary file"),
        "{output}"
    );

    let plain = repo.base.join("plain");
    fs::create_dir_all(&plain).unwrap();
    let output = dispatch_promptly(&plain, "git_status");
    assert!(
        output.starts_with("error:") && output.contains(POINTS_OUTSIDE),
        "{output}"
    );
}

/// Issue #461 (2): a gitfile needs git's exact `gitdir: ` prefix, with only a trailing
/// CR/LF dropped. `gitdir:  real` (two spaces) is git's ` real`, not `real`.
#[test]
fn a_gitfile_that_git_reads_differently_is_refused_before_git_runs() {
    // git reads ` real` and `real ` as they are: the FIFO `HEAD` there is what is refused.
    for (text, name, refusal) in [
        (
            "gitdir:  real\n",
            " real",
            "has a HEAD that is not an ordinary file",
        ),
        (
            "gitdir: real \n",
            "real ",
            "has a HEAD that is not an ordinary file",
        ),
    ] {
        let repo = Repo::new();
        let root = repo.base.join("ws");
        fs::create_dir_all(&root).unwrap();
        // A clean `real` for a trimming reader, and a FIFO-HEAD repository where git looks.
        fs::rename(repo.root.join(".git"), root.join("real")).unwrap();
        bare_layout_with_fifo_head(&root.join(name));
        fs::write(root.join(".git"), text).unwrap();
        let output = dispatch_promptly(&root, "git_status");
        assert!(
            output.starts_with("error:") && output.contains(refusal),
            "{text:?}: {output}"
        );
    }
}

/// Issue #461 (2): `gitdir:real` (no space) is no gitfile to git, so it is refused.
#[test]
fn a_gitfile_without_the_exact_prefix_is_refused() {
    let repo = Repo::new();
    let root = repo.base.join("ws");
    fs::create_dir_all(&root).unwrap();
    fs::rename(repo.root.join(".git"), root.join("real")).unwrap();
    fs::write(root.join(".git"), "gitdir:real\n").unwrap();
    let output = dispatch_promptly(&root, "git_status");
    assert!(
        output.starts_with("error:") && output.contains(POINTS_OUTSIDE),
        "{output}"
    );
}

/// Issue #461 (3): an alternates line git reads differently (quoted, or padded with
/// whitespace) is refused, not skipped.
#[test]
fn a_quoted_or_padded_alternates_line_is_refused_before_git_runs() {
    for pattern in ["\"{}\"", " {}", "{} ", "{}\r"] {
        let repo = Repo::new();
        let store = repo.root.join(".git/second");
        fs::create_dir_all(store.join("info")).unwrap();
        mkfifo(&store.join("info/alternates"));
        let alternates = repo.root.join(".git/objects/info/alternates");
        fs::create_dir_all(alternates.parent().unwrap()).unwrap();
        let line = pattern.replace("{}", store.to_str().unwrap());
        fs::write(&alternates, format!("{line}\n")).unwrap();
        let output = dispatch_promptly(&repo.root, "git_status");
        assert!(
            output.starts_with("error:") && output.contains("borrows objects"),
            "{pattern:?}: {output}"
        );
    }
}

/// Issue #461 (4): a FIFO, socket or device in any file git opens is refused before git
/// runs, naming the entry.
#[test]
fn a_special_file_anywhere_in_the_repository_is_refused_before_git_runs() {
    for file in [
        "index",
        "packed-refs",
        "shallow",
        "config.worktree",
        "info/grafts",
        "info/exclude",
        "refs/heads/branch",
        "refs/tags/deep/tag",
        "objects/pack/pack-1.pack",
        "objects/ab/cdef",
    ] {
        let repo = Repo::new();
        let path = repo.root.join(".git").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let _ = fs::remove_file(&path);
        mkfifo(&path);
        let output = dispatch_promptly(&repo.root, "git_status");
        let name = file.rsplit('/').next().unwrap();
        assert!(
            output.starts_with("error:") && output.contains(name) && output.contains("FIFO"),
            "{file}: {output}"
        );
    }
    let repo = Repo::new();
    let path = repo.root.join(".git/index");
    fs::remove_file(&path).unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(
        output.starts_with("error:") && output.contains("index"),
        "{output}"
    );
}

/// Issue #461 (4): a linked worktree's own `config.worktree` and `index` count too.
#[test]
fn a_special_file_in_a_linked_worktrees_entry_is_refused_before_git_runs() {
    for file in ["index", "config.worktree"] {
        let repo = Repo::new();
        let wt = repo.base.join("wt");
        git(
            &repo.root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "wt-branch",
                wt.to_str().unwrap(),
            ],
        );
        let path = repo.root.join(".git/worktrees/wt").join(file);
        let _ = fs::remove_file(&path);
        mkfifo(&path);
        let output = dispatch_promptly(&wt, "git_status");
        assert!(
            output.starts_with("error:") && output.contains(file),
            "{file}: {output}"
        );
    }
}

/// Issue #461 (review): git's own fsmonitor daemon leaves a socket `fsmonitor--daemon.ipc` in
/// the git directory; Alfredo's git runs with `core.fsmonitor=false`, so a valid repository
/// with that socket is read. Only that exact socket is exempt: a FIFO of that name, or a
/// socket anywhere else, is refused.
#[test]
fn the_fsmonitor_daemon_socket_is_accepted_but_no_other_special_file() {
    let repo = Repo::new();
    let socket = repo.root.join(".git/fsmonitor--daemon.ipc");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    for tool in ["git_status", "git_log"] {
        let output = dispatch_promptly(&repo.root, tool);
        assert!(!output.starts_with("error:"), "{tool}: {output}");
    }
    let wt = repo.base.join("wt");
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt-branch",
            wt.to_str().unwrap(),
        ],
    );
    let entry_socket = repo.root.join(".git/worktrees/wt/fsmonitor--daemon.ipc");
    let _entry_listener = std::os::unix::net::UnixListener::bind(&entry_socket).unwrap();
    let output = dispatch_promptly(&wt, "git_status");
    assert!(!output.starts_with("error:"), "{output}");

    let repo = Repo::new();
    mkfifo(&repo.root.join(".git/fsmonitor--daemon.ipc"));
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(
        output.starts_with("error:") && output.contains("fsmonitor--daemon.ipc"),
        "{output}"
    );

    let repo = Repo::new();
    fs::create_dir_all(repo.root.join(".git/info")).unwrap();
    let _elsewhere =
        std::os::unix::net::UnixListener::bind(repo.root.join(".git/info/fsmonitor--daemon.ipc"))
            .unwrap();
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(
        output.starts_with("error:") && output.contains("socket"),
        "{output}"
    );
}

/// Issue #461 (review): when git falls back to the workspace root as a bare repository, a
/// FIFO root `commondir` or `config` blocks it just like `HEAD`.
#[test]
fn a_bare_root_fallback_with_a_fifo_commondir_or_config_is_refused() {
    for file in ["commondir", "config"] {
        let repo = Repo::new();
        fs::write(repo.root.join(".git/HEAD"), "garbage\n").unwrap();
        fs::create_dir_all(repo.root.join("objects")).unwrap();
        fs::create_dir_all(repo.root.join("refs")).unwrap();
        fs::write(repo.root.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        mkfifo(&repo.root.join(file));
        let output = dispatch_promptly(&repo.root, "git_status");
        assert!(
            output.starts_with("error:") && output.contains(file),
            "{file}: {output}"
        );
    }
}

/// Issue #461 (review 3): a directory named `config`, `HEAD` or `commondir` at the workspace
/// root (common in real projects) cannot block git and must not refuse a valid repository.
#[test]
fn root_directories_named_like_git_files_do_not_refuse_a_valid_repository() {
    let repo = Repo::new();
    for name in ["config", "HEAD", "commondir"] {
        fs::create_dir_all(repo.root.join(name)).unwrap();
        fs::write(repo.root.join(name).join("settings.txt"), "x\n").unwrap();
    }
    for tool in ["git_status", "git_log"] {
        let output = dispatch_promptly(&repo.root, tool);
        assert!(!output.starts_with("error:"), "{tool}: {output}");
    }
}

/// Issue #465 (1): a FIFO in a git-dir subdirectory git reads (`rebase-merge/head-name`,
/// `sequencer/todo`, a submodule's `modules/*/HEAD`) is refused before git runs, not after
/// git has stalled to its 15 s limit.
#[test]
fn a_fifo_in_a_git_dir_subdirectory_is_refused_before_git_runs() {
    for file in [
        "rebase-merge/head-name",
        "sequencer/todo",
        "rebase-apply/next",
        "modules/sub/HEAD",
    ] {
        let repo = Repo::new();
        let path = repo.root.join(".git").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        mkfifo(&path);
        let name = path.file_name().unwrap().to_str().unwrap();
        for tool in ["git_status", "git_log"] {
            let output = dispatch_promptly(&repo.root, tool);
            assert!(
                output.starts_with("error:") && output.contains(name),
                "{file} {tool}: {output}"
            );
        }
    }
}

/// Issue #465 (1): the same holds from a linked worktree, whose entry holds its own
/// `rebase-merge/`.
#[test]
fn a_fifo_in_a_linked_worktrees_git_dir_subdirectory_is_refused() {
    let repo = Repo::new();
    let wt = repo.base.join("wt");
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt-branch",
            wt.to_str().unwrap(),
        ],
    );
    let path = repo.root.join(".git/worktrees/wt/rebase-merge/head-name");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    mkfifo(&path);
    let output = dispatch_promptly(&wt, "git_status");
    assert!(
        output.starts_with("error:") && output.contains("head-name"),
        "{output}"
    );
}

/// Issue #465 (1): ordinary files in those subdirectories, and git's fsmonitor socket at the
/// top of the git directory, still read; a symlinked subdirectory is refused, not followed.
#[test]
fn ordinary_git_dir_subdirectories_are_accepted_and_linked_ones_refused() {
    let repo = Repo::new();
    fs::create_dir_all(repo.root.join(".git/rebase-merge")).unwrap();
    fs::write(
        repo.root.join(".git/rebase-merge/head-name"),
        "refs/heads/main\n",
    )
    .unwrap();
    fs::create_dir_all(repo.root.join(".git/sequencer")).unwrap();
    fs::write(repo.root.join(".git/sequencer/todo"), "pick abc\n").unwrap();
    let _listener =
        std::os::unix::net::UnixListener::bind(repo.root.join(".git/fsmonitor--daemon.ipc"))
            .unwrap();
    for tool in ["git_status", "git_log"] {
        let output = dispatch_promptly(&repo.root, tool);
        assert!(!output.starts_with("error:"), "{tool}: {output}");
    }
    let repo = Repo::new();
    let elsewhere = repo.base.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    mkfifo(&elsewhere.join("todo"));
    std::os::unix::fs::symlink(&elsewhere, repo.root.join(".git/sequencer")).unwrap();
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(output.starts_with("error:"), "{output}");
}

/// Issue #465 (2): `git_log` refuses a config with `include.path` or `includeIf`, as
/// `git_status` does, so an include target that is a FIFO never stalls git.
#[test]
fn git_log_refuses_a_config_include_before_git_runs() {
    for section in [
        "[include]\n\tpath = {}\n",
        "[includeIf \"gitdir:/\"]\n\tpath = {}\n",
    ] {
        let repo = Repo::new();
        let fifo = repo.base.join("included.fifo");
        mkfifo(&fifo);
        let mut config = fs::read_to_string(repo.root.join(".git/config")).unwrap();
        config.push_str(&section.replace("{}", fifo.to_str().unwrap()));
        fs::write(repo.root.join(".git/config"), config).unwrap();
        for tool in ["git_log", "git_status"] {
            let output = dispatch_promptly(&repo.root, tool);
            assert!(
                output.starts_with("error:") && output.contains("include"),
                "{tool}: {output}"
            );
        }
    }
}

/// Issue #492 (1): an include refuses every chat git read tool, so the refusal of
/// `git_status` or a working-tree `git_diff` must not claim `git_log` or `git_diff` with a
/// base and target still work.
#[test]
fn an_include_refusal_does_not_say_other_git_tools_still_work() {
    let repo = Repo::new();
    append(
        &repo.root.join(".git/config"),
        "[include]\n\tpath = /nonexistent/x\n",
    );
    fs::write(repo.root.join("README.md"), "# Demo\ndirty\n").unwrap();
    for (tool, arguments) in [("git_status", json!({})), ("git_diff", json!({}))] {
        let output = repo.run(tool, arguments);
        assert!(output.starts_with("error:"), "{tool}: {output}");
        assert!(!output.contains("still work"), "{tool}: {output}");
        assert!(
            output.contains("an include refuses every chat git read tool"),
            "{tool}: {output}"
        );
        assert!(output.contains("run_command"), "{tool}: {output}");
    }
    let output = repo.run("git_log", json!({}));
    assert!(output.starts_with("error:"), "git_log: {output}");
}

/// Issue #492 (2): with only a filter driver (no include) `git_status` is refused and says
/// `git_log` and `git_diff` with a base and target work, which they do.
#[test]
fn a_filter_refusal_says_log_and_base_target_diff_still_work() {
    let repo = Repo::new();
    repo.go_branch();
    append(
        &repo.root.join(".git/config"),
        "[filter \"f\"]\n\tclean = cat\n",
    );
    let output = repo.run("git_status", json!({}));
    assert!(output.starts_with("error:"), "{output}");
    assert!(
        output.contains("git_log and git_diff with base and target still work"),
        "{output}"
    );
    assert!(!output.contains("an include refuses"), "{output}");
    let output = repo.run("git_log", json!({}));
    assert!(!output.starts_with("error:"), "{output}");
    let output = repo.run(
        "git_diff",
        json!({"base": "main", "target": "alfredo/go-7"}),
    );
    assert!(!output.starts_with("error:"), "{output}");
}

/// Issue #492 (3): a symbolic link in the git directory is named by its git-directory-relative
/// path, with what to do about it, by every chat git read tool; no absolute host path.
#[test]
fn a_symlink_in_the_git_directory_is_named_with_its_relative_path() {
    let repo = Repo::new();
    let exclude = repo.root.join(".git/info/exclude");
    let _ = fs::remove_file(&exclude);
    fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", &exclude).unwrap();
    for tool in ["git_status", "git_log", "git_diff", "git_branch"] {
        let output = dispatch_promptly(&repo.root, tool);
        assert!(
            output.starts_with("error:") && output.contains("`info/exclude`"),
            "{tool}: {output}"
        );
        assert!(output.contains("is a symbolic link"), "{tool}: {output}");
        assert!(
            output.contains("Remove or replace it") && output.contains("run_command"),
            "{tool}: {output}"
        );
        let root = repo.root.to_str().unwrap();
        assert!(!output.contains(root), "{tool}: absolute path: {output}");
    }
}

/// Issue #492 (4): a FIFO or socket deep in the git directory (an LFS agent socket) is named
/// by its relative path in a grammatical sentence, not "has a agent.sock".
#[test]
fn a_socket_or_fifo_in_the_git_directory_is_named_in_a_grammatical_sentence() {
    for use_socket in [true, false] {
        let repo = Repo::new();
        let path = repo.root.join(".git/lfs/tmp/agent.sock");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let _listener = if use_socket {
            Some(std::os::unix::net::UnixListener::bind(&path).unwrap())
        } else {
            mkfifo(&path);
            None
        };
        for tool in ["git_status", "git_log"] {
            let output = dispatch_promptly(&repo.root, tool);
            assert!(
                output.starts_with("error:")
                    && output.contains("`lfs/tmp/agent.sock` in the repository's git directory is a FIFO, socket, device or link"),
                "{tool}: {output}"
            );
            assert!(!output.contains("has a agent.sock"), "{tool}: {output}");
            assert!(output.contains("run_command"), "{tool}: {output}");
            let root = repo.root.to_str().unwrap();
            assert!(!output.contains(root), "{tool}: absolute path: {output}");
        }
    }
}

/// Issue #492 (5): a control or format character in the offending entry's name shows as
/// `<U+XXXX>`, never raw (as #313/#361).
#[test]
fn a_control_character_in_a_refused_entry_name_is_escaped() {
    for (relative, link, shown) in [
        (
            "lfs/bad\u{1b}name\u{202e}txt",
            false,
            "lfs/bad<U+001B>name<U+202E>txt",
        ),
        ("info/new\nline", true, "info/new<U+000A>line"),
    ] {
        let repo = Repo::new();
        let path = repo.root.join(".git").join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if link {
            std::os::unix::fs::symlink("/etc/hostname", &path).unwrap();
        } else {
            mkfifo(&path);
        }
        let output = dispatch_promptly(&repo.root, "git_log");
        assert!(output.contains(shown), "{output}");
        assert!(!output.contains(['\u{1b}', '\u{202e}', '\n']), "{output:?}");
    }
}

/// Issue #497: a partial clone (`extensions.partialClone`, a promisor remote whose
/// `uploadpack` is a program the repository chose) with `object` missing from the store.
/// Returns the repo and the marker file the program would create.
fn partial_clone_repo(missing: &str) -> (Repo, PathBuf) {
    let repo = Repo::new();
    repo.go_branch();
    let object = |spec: &str| git(&repo.root, &["rev-parse", spec]).trim().to_string();
    let id = match missing {
        "commit" => object("alfredo/go-7"),
        "blob" => object("alfredo/go-7:README.md"),
        _ => object("main:README.md"),
    };
    let loose = repo.root.join(".git/objects").join(&id[..2]).join(&id[2..]);
    fs::remove_file(&loose).unwrap();
    fs::write(repo.root.join("README.md"), "# Demo\nedited\n").unwrap();
    let marker = repo.base.join("X");
    let script = repo.base.join("uploadpack.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let url = repo.root.display().to_string();
    for (key, value) in [
        ("core.repositoryformatversion", "1"),
        ("extensions.partialclone", "origin"),
        ("remote.origin.url", url.as_str()),
        ("remote.origin.promisor", "true"),
        ("remote.origin.uploadpack", script.to_str().unwrap()),
    ] {
        git(&repo.root, &["config", key, value]);
    }
    (repo, marker)
}

/// Issue #497: no read-tier git tool runs the repository's `remote.origin.uploadpack`
/// program through a partial-clone lazy fetch, and each answers with an error naming the
/// cause within 3 s.
#[test]
fn no_read_tier_tool_runs_a_partial_clone_uploadpack_program() {
    let calls = [
        ("git_status", json!({}), "blob-main"),
        ("git_diff", json!({}), "blob-main"),
        (
            "git_diff",
            json!({"base": "main", "target": "alfredo/go-7"}),
            "blob",
        ),
        ("git_diff", json!({"base": "main"}), "blob"),
        ("git_log", json!({"ref": "alfredo/go-7"}), "commit"),
        ("git_branch", json!({}), "commit"),
        ("list_files", json!({}), "blob-main"),
        ("grep", json!({"pattern": "Demo"}), "blob-main"),
    ];
    for (tool, arguments, missing) in calls {
        let (repo, marker) = partial_clone_repo(missing);
        let output = dispatch_promptly_with(&repo.root, tool, arguments.clone());
        assert!(
            !marker.exists(),
            "{tool} {arguments} ran the repository's uploadpack program: {output}"
        );
        if tool.starts_with("git_") {
            assert!(
                output.starts_with("error:") && output.contains("uploadpack"),
                "{tool} {arguments}: {output}"
            );
        }
    }
}

/// Issue #465 (mutation review): the include refusal says so, and a config git cannot read
/// at all fails closed with its own message rather than the include one.
#[test]
fn the_include_refusal_and_the_unreadable_config_refusal_are_told_apart() {
    let repo = Repo::new();
    let mut config = fs::read_to_string(repo.root.join(".git/config")).unwrap();
    config.push_str("[include]\n\tpath = /nonexistent/x\n");
    fs::write(repo.root.join(".git/config"), &config).unwrap();
    let output = dispatch_promptly(&repo.root, "git_log");
    assert!(
        output.starts_with("error:")
            && output.contains("includes another file")
            && !output.contains("cannot be read or checked"),
        "{output}"
    );

    for broken in [
        "[include\n\tpath = /nonexistent/x\n",
        "[include]\n\tpath = /x\n[broken\n",
    ] {
        let repo = Repo::new();
        let mut config = fs::read_to_string(repo.root.join(".git/config")).unwrap();
        config.push_str(broken);
        fs::write(repo.root.join(".git/config"), config).unwrap();
        let output = dispatch_promptly(&repo.root, "git_log");
        assert!(
            output.starts_with("error:")
                && output.contains("cannot be read or checked for includes")
                && !output.contains("includes another file"),
            "{broken:?}: {output}"
        );
    }
}

/// Frees git children blocked on `fifo` and kills every git working under `dir`, until none
/// has shown up for a few checks in a row: git opens the FIFO again after its first read
/// ends, so one wake-up is not enough. Without it a red run leaves `git` blocked for good.
fn release_git_blocked_on(fifo: &Path, dir: &Path) {
    let mut quiet = 0;
    while quiet < 3 {
        let _ = fs::OpenOptions::new().read(true).write(true).open(fifo);
        let killed = Command::new("pkill")
            .args(["-KILL", "-f", "--"])
            .arg(format!("-C {}", dir.display()))
            .status()
            .is_ok_and(|status| status.success());
        quiet = if killed { 0 } else { quiet + 1 };
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Releases the git children a failed run leaves blocked on its FIFO, when dropped (also
/// while a failed assertion unwinds).
struct Unblock {
    fifo: PathBuf,
    base: PathBuf,
}
impl Drop for Unblock {
    fn drop(&mut self) {
        release_git_blocked_on(&self.fifo, &self.base);
    }
}

/// Appends `section` to the repository config, with `{}` replaced by the path of a fresh FIFO
/// `base_relative` to the repository's parent directory.
fn config_with_fifo(repo: &Repo, base_relative: &str, section: &str) -> Unblock {
    let fifo = repo.base.join(base_relative);
    mkfifo(&fifo);
    let mut config = fs::read_to_string(repo.root.join(".git/config")).unwrap();
    config.push_str(&section.replace("{}", fifo.to_str().unwrap()));
    fs::write(repo.root.join(".git/config"), config).unwrap();
    Unblock {
        fifo,
        base: repo.root.clone(),
    }
}

/// Issue #474 (1): `git_branch` refuses a config with `include.path` or `includeIf` before
/// git runs, as `git_status` and `git_log` do, so a FIFO include target never stalls it.
#[test]
fn git_branch_refuses_a_config_include_before_git_runs() {
    for section in [
        "[include]\n\tpath = {}\n",
        "[includeIf \"gitdir:/\"]\n\tpath = {}\n",
    ] {
        let repo = Repo::new();
        let _release = config_with_fifo(&repo, "included.fifo", section);
        let output = dispatch_promptly(&repo.root, "git_branch");
        assert!(
            output.starts_with("error:") && output.contains("includes another file"),
            "{output}"
        );
    }
}

/// Issue #474 (2): repository config naming a file git opens on its own (`core.excludesFile`,
/// `core.attributesFile`, `mailmap.file`) as a FIFO, in the worktree or outside `.git`, never
/// stalls a read-tier call: it is refused or completes within 3 s.
#[test]
fn a_fifo_named_by_excludes_attributes_or_mailmap_config_never_stalls() {
    let cases = [
        (
            "core.excludesFile",
            "[core]\n\texcludesFile = {}\n",
            "git_status",
        ),
        (
            "core.attributesFile",
            "[core]\n\tattributesFile = {}\n",
            "git_status",
        ),
        (
            "mailmap.file",
            "[mailmap]\n\tfile = {}\n[log]\n\tmailmap = true\n",
            "git_log",
        ),
    ];
    for (key, section, tool) in cases {
        for path in ["repo/x.fifo", "x.fifo"] {
            let repo = Repo::new();
            let _release = config_with_fifo(&repo, path, section);
            let output = dispatch_promptly(&repo.root, tool);
            assert!(
                !output.contains("time limit") && !output.contains("took longer"),
                "{key} ({path}): {output}"
            );
        }
    }
}

/// Issue #474 (review): `diff.orderFile` is read by git on its own whenever a diff has
/// changes, so a FIFO there stalls `git_status` and both `git_diff` forms unless overridden.
#[test]
fn a_fifo_diff_order_file_never_stalls_status_or_diff() {
    let repo = Repo::new();
    repo.go_branch();
    fs::write(repo.root.join("README.md"), "# Demo\nchanged\n").unwrap();
    git(&repo.root, &["add", "README.md"]);
    fs::write(repo.root.join("README.md"), "# Demo\nchanged again\n").unwrap();
    let _release = config_with_fifo(&repo, "order.fifo", "[diff]\n\torderFile = {}\n");
    for (tool, arguments) in [
        ("git_status", json!({})),
        ("git_diff", json!({})),
        ("git_diff", json!({ "staged": true })),
        (
            "git_diff",
            json!({ "base": "main", "target": "alfredo/go-7" }),
        ),
    ] {
        let output = dispatch_promptly_with(&repo.root, tool, arguments.clone());
        assert!(
            !output.contains("time limit") && !output.contains("took longer"),
            "{tool} {arguments}: {output}"
        );
    }
}

/// Issue #474 (review): `log.mailmap=false` is what keeps a committed `.mailmap` out of
/// `git_log`, also when the repository config turns `log.mailmap` on.
#[test]
fn a_committed_mailmap_is_not_applied_to_git_log() {
    let repo = Repo::new();
    fs::write(repo.root.join(".mailmap"), "Mapped <t@e> T <t@e>\n").unwrap();
    git(&repo.root, &["add", ".mailmap"]);
    git(&repo.root, &["commit", "-q", "-m", "add mailmap"]);
    let mut config = fs::read_to_string(repo.root.join(".git/config")).unwrap();
    config.push_str("[log]\n\tmailmap = true\n");
    fs::write(repo.root.join(".git/config"), config).unwrap();
    let output = dispatch_promptly(&repo.root, "git_log");
    assert!(
        output.contains("add mailmap") && output.contains(" T: ") && !output.contains("Mapped"),
        "{output}"
    );
}

/// Issue #474 (review): a FIFO `.mailmap` in the worktree is read by `git log` when the
/// repository config sets `log.mailmap`; `log.mailmap=false` keeps it from stalling the call.
#[test]
fn a_fifo_mailmap_in_the_worktree_never_stalls_git_log() {
    let repo = Repo::new();
    let fifo = repo.root.join(".mailmap");
    mkfifo(&fifo);
    let _release = Unblock {
        fifo,
        base: repo.root.clone(),
    };
    let mut config = fs::read_to_string(repo.root.join(".git/config")).unwrap();
    config.push_str("[log]\n\tmailmap = true\n");
    fs::write(repo.root.join(".git/config"), config).unwrap();
    let output = dispatch_promptly(&repo.root, "git_log");
    assert!(output.contains("initial commit"), "{output}");
}

/// Issue #474 (3): a regular `core.excludesFile` in the repository config does not refuse
/// `git_status`; git does not read it (the tools override the key), which the listing shows.
#[test]
fn a_regular_excludes_file_does_not_refuse_git_status() {
    let repo = Repo::new();
    let excludes = repo.base.join("excludes");
    fs::write(&excludes, "ignored.txt\n").unwrap();
    let mut config = fs::read_to_string(repo.root.join(".git/config")).unwrap();
    config.push_str(&format!(
        "[core]\n\texcludesFile = {}\n",
        excludes.display()
    ));
    fs::write(repo.root.join(".git/config"), config).unwrap();
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(
        output.contains("clean") && !output.starts_with("error:"),
        "{output}"
    );
    // Documented behaviour change: the file named by the repository config is not read, so
    // what it would hide is listed.
    fs::write(repo.root.join("ignored.txt"), "x\n").unwrap();
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(output.contains("?? ignored.txt"), "{output}");
}

/// Issue #465 (review): a symlinked hook (`ln -s ../../hook.sh .git/hooks/pre-commit`, common
/// in setup scripts) is never opened by these calls (hooks are off), so it must not refuse a
/// valid repository, also from a linked worktree.
#[test]
fn a_symlinked_hook_does_not_refuse_a_valid_repository() {
    let repo = Repo::new();
    fs::write(repo.root.join("hook.sh"), "#!/bin/sh\n").unwrap();
    let hooks = repo.root.join(".git/hooks");
    fs::create_dir_all(&hooks).unwrap();
    std::os::unix::fs::symlink("../../hook.sh", hooks.join("pre-commit")).unwrap();
    let wt = repo.base.join("wt");
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt-branch",
            wt.to_str().unwrap(),
        ],
    );
    for (root, tool) in [
        (&repo.root, "git_status"),
        (&repo.root, "git_log"),
        (&wt, "git_status"),
    ] {
        let output = dispatch_promptly(root, tool);
        assert!(!output.starts_with("error:"), "{tool}: {output}");
    }
}

/// Issue #465 (review): git's fsmonitor socket in a linked worktree's entry stays allowed
/// while the common directory (and its `worktrees/*` entries) are walked.
#[test]
fn the_fsmonitor_socket_in_a_worktree_entry_is_accepted_from_the_main_workspace() {
    let repo = Repo::new();
    let wt = repo.base.join("wt");
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt-branch",
            wt.to_str().unwrap(),
        ],
    );
    let _listener = std::os::unix::net::UnixListener::bind(
        repo.root.join(".git/worktrees/wt/fsmonitor--daemon.ipc"),
    )
    .unwrap();
    for root in [&repo.root, &wt] {
        let output = dispatch_promptly(root, "git_status");
        assert!(!output.starts_with("error:"), "{output}");
    }
}

/// Issue #461 (6): an oversize gitfile or alternates file says it is too large, not that it
/// is a FIFO, device or link.
#[test]
fn an_oversize_control_file_is_refused_as_too_large() {
    let repo = Repo::new();
    let alternates = repo.root.join(".git/objects/info/alternates");
    fs::create_dir_all(alternates.parent().unwrap()).unwrap();
    fs::write(&alternates, "# padding\n".repeat(8_000)).unwrap();
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(
        output.starts_with("error:") && output.contains("larger than") && !output.contains("FIFO"),
        "{output}"
    );

    let repo = Repo::new();
    let root = repo.base.join("ws");
    fs::create_dir_all(&root).unwrap();
    fs::rename(repo.root.join(".git"), root.join("real")).unwrap();
    fs::write(
        root.join(".git"),
        format!("gitdir: real\n{}", "#".repeat(70_000)),
    )
    .unwrap();
    let output = dispatch_promptly(&root, "git_status");
    assert!(
        output.starts_with("error:") && output.contains("larger than") && !output.contains("FIFO"),
        "{output}"
    );
}

/// Issue #461 (5): a chain of out-of-repository stores that list each other is refused at the
/// first one, long before the clock.
#[test]
fn a_chain_of_outside_stores_is_refused_at_once() {
    let repo = Repo::new();
    let stores: Vec<PathBuf> = (0..300).map(|n| repo.base.join(format!("s{n}"))).collect();
    let listing: String = stores
        .iter()
        .map(|s| format!("{}\n", s.display()))
        .collect();
    for store in &stores {
        fs::create_dir_all(store.join("info")).unwrap();
        fs::write(store.join("info/alternates"), &listing).unwrap();
    }
    let alternates = repo.root.join(".git/objects/info/alternates");
    fs::create_dir_all(alternates.parent().unwrap()).unwrap();
    fs::write(&alternates, &listing).unwrap();
    let started = std::time::Instant::now();
    let output = dispatch_promptly(&repo.root, "git_status");
    assert!(output.contains("borrows objects"), "{output}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

/// Issue #359 (review 3): a reflog under `logs/` that is a symlink into another repository
/// is refused like a link in `refs/` or `info/`, however deep it sits.
#[test]
fn a_reflog_symlink_into_another_repository_is_never_read() {
    let other = Repo::new();
    fs::write(other.root.join("creds.txt"), "AWS_SECRET=hunter2\n").unwrap();
    git(&other.root, &["add", "creds.txt"]);
    git(&other.root, &["commit", "-q", "-m", "add creds"]);
    let repo = Repo::new();
    let heads = repo.root.join(".git/logs/refs/heads");
    fs::create_dir_all(&heads).unwrap();
    // A fresh repository already has its own reflog for main: replace it with the link.
    fs::remove_file(heads.join("main")).unwrap();
    std::os::unix::fs::symlink(other.root.join(".git/logs/HEAD"), heads.join("main")).unwrap();
    for (tool, arguments) in [("git_status", json!({})), ("git_log", json!({}))] {
        let output = dispatch_within(&repo.root, tool, arguments);
        assert!(!output.contains("hunter2"), "{tool}: {output}");
        assert!(output.starts_with("error:"), "{tool}: {output}");
    }
}

/// Issue #359 (review 2): an alternates entry naming the repository's own object store is
/// not another repository: the history still reads.
#[test]
fn an_alternates_entry_naming_the_own_object_store_still_reads() {
    let repo = Repo::new();
    let own = repo.root.join(".git/objects");
    fs::write(own.join("info/alternates"), format!("{}\n", own.display())).unwrap();
    let log = chat_tools::dispatch(
        &repo.root,
        &ToolCall {
            name: "git_log".into(),
            arguments: json!({}),
        },
    );
    assert!(log.contains("initial commit"), "{log}");
}

/// Issue #359 (review 2): many alternates stores sharing each other must be checked once each,
/// not once per path through them: a shared-store DAG (18 wide, 5 deep) stays within budget.
#[test]
fn a_dag_of_shared_object_stores_is_checked_once_per_store() {
    let repo = Repo::new();
    let root = repo.root.join(".git/stores");
    let width = 18;
    let depth = 5;
    let dir = |layer: usize, index: usize| root.join(format!("l{layer}/d{index}/objects"));
    for layer in 0..depth {
        for index in 0..width {
            fs::create_dir_all(dir(layer, index).join("info")).unwrap();
        }
    }
    for layer in 0..depth - 1 {
        for index in 0..width {
            let next: String = (0..width)
                .map(|next| format!("{}\n", dir(layer + 1, next).display()))
                .collect();
            fs::write(dir(layer, index).join("info/alternates"), next).unwrap();
        }
    }
    let first: String = (0..width)
        .map(|index| format!("{}\n", dir(0, index).display()))
        .collect();
    fs::write(repo.root.join(".git/objects/info/alternates"), first).unwrap();
    let started = std::time::Instant::now();
    let log = chat_tools::dispatch(
        &repo.root,
        &ToolCall {
            name: "git_log".into(),
            arguments: json!({}),
        },
    );
    assert!(log.contains("initial commit"), "{log}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

/// Issue #359: a linked worktree whose back-reference is relative and has CRLF line ends is
/// still the workspace's own linked worktree.
#[test]
fn a_linked_worktree_with_a_relative_back_reference_still_reads() {
    let repo = Repo::new();
    let wt = repo.base.join("wt");
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt-branch",
            wt.to_str().unwrap(),
        ],
    );
    // The entry sits at <repo>/.git/worktrees/wt; from there the workspace is ../../../../wt.
    // Rename over `gitdir`: truncating it in place would leave a prunable entry.
    let gitdir = repo.root.join(".git/worktrees/wt/gitdir");
    let staged = gitdir.with_file_name("gitdir.new");
    fs::write(&staged, "../../../../wt/.git\r\n").unwrap();
    fs::rename(&staged, &gitdir).unwrap();
    let log = chat_tools::dispatch(
        &wt,
        &ToolCall {
            name: "git_log".into(),
            arguments: json!({}),
        },
    );
    assert!(log.contains("initial commit"), "{log}");
}

/// Issue #359: a linked-worktree layout whose main repository sits inside the workspace is a
/// repository the chat can write to, not the owner's, and is refused.
#[test]
fn a_linked_worktree_whose_main_repository_is_inside_the_workspace_is_refused() {
    let repo = Repo::new();
    let ws = repo.base.join("ws");
    let main = ws.join("main");
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    fs::write(main.join("creds.txt"), "AWS_SECRET=hunter2\n").unwrap();
    git(&main, &["add", "creds.txt"]);
    git(&main, &["commit", "-q", "-m", "add creds"]);
    let branch = git(&main, &["symbolic-ref", "--short", "HEAD"]);
    let entry = install_worktree_entry(
        &main.join(".git"),
        "x",
        &format!("ref: refs/heads/{}\n", branch.trim()),
        &format!("{}\n", ws.join(".git").display()),
    );
    fs::write(ws.join(".git"), format!("gitdir: {}\n", entry.display())).unwrap();
    for tool in ["git_status", "git_log"] {
        let output = chat_tools::dispatch(
            &ws,
            &ToolCall {
                name: tool.into(),
                arguments: json!({}),
            },
        );
        assert!(!output.contains("hunter2"), "{tool}: {output}");
        assert!(output.starts_with("error:"), "{tool}: {output}");
    }
}

#[test]
fn renamed_secret_content_is_not_shown() {
    let repo = Repo::new();
    fs::write(repo.root.join(".env"), "TOKEN=hunter2\n").unwrap();
    git(&repo.root, &["add", ".env"]);
    git(&repo.root, &["commit", "-q", "-m", "env"]);
    git(&repo.root, &["mv", ".env", "config.txt"]);
    let output = repo.run("git_diff", json!({"staged": true}));
    assert!(!output.contains("hunter2"), "{output}");
    git(&repo.root, &["commit", "-q", "-m", "rename"]);
    let output = repo.run("git_diff", json!({"base": "HEAD~1", "target": "HEAD"}));
    assert!(!output.contains("hunter2"), "{output}");
}

/// #309 e: core.worktree points git at files outside the workspace; the Read tier must
/// never show their content.
#[test]
fn a_worktree_outside_the_workspace_is_never_read() {
    let repo = Repo::new();
    let outside = repo.base.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("README.md"), "OUTSIDE-CONTENT\n").unwrap();
    git(
        &repo.root,
        &["config", "core.worktree", outside.to_str().unwrap()],
    );
    for (name, arguments) in [("git_status", json!({})), ("git_diff", json!({}))] {
        let output = repo.run(name, arguments);
        assert!(!output.contains("OUTSIDE-CONTENT"), "{name}: {output}");
        assert!(output.starts_with("error:"), "{name}: {output}");
    }
}
