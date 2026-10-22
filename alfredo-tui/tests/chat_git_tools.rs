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
    let entry = other.root.join(".git/worktrees/evil");
    fs::create_dir_all(&entry).unwrap();
    let branch = git(&other.root, &["symbolic-ref", "--short", "HEAD"]);
    fs::write(
        entry.join("HEAD"),
        format!("ref: refs/heads/{}\n", branch.trim()),
    )
    .unwrap();
    fs::write(entry.join("commondir"), "../..\n").unwrap();
    fs::write(
        entry.join("gitdir"),
        format!("{}/elsewhere/.git\n", other.base.display()),
    )
    .unwrap();
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
    fs::write(
        repo.root.join(".git/worktrees/wt/gitdir"),
        "../../../../wt/.git\r\n",
    )
    .unwrap();
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
    let entry = main.join(".git/worktrees/x");
    fs::create_dir_all(&entry).unwrap();
    let branch = git(&main, &["symbolic-ref", "--short", "HEAD"]);
    fs::write(
        entry.join("HEAD"),
        format!("ref: refs/heads/{}\n", branch.trim()),
    )
    .unwrap();
    fs::write(entry.join("commondir"), "../..\n").unwrap();
    fs::write(
        entry.join("gitdir"),
        format!("{}\n", ws.join(".git").display()),
    )
    .unwrap();
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
