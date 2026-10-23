//! Issue #233: chat tool registry and read-only Coding Workspace tools.
use alfredo_tui::{
    chat_tools::{self, Tier, MAX_OUTPUT},
    model::ToolCall,
};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static ID: AtomicU64 = AtomicU64::new(0);

struct Workspace {
    base: PathBuf,
    root: PathBuf,
}
impl Workspace {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "alfredo-chat-tools-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.join("repo");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("README.md"), "# Demo\nA tiny demo project.\n").unwrap();
        fs::write(
            root.join("src/main.rs"),
            "fn main() {\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\nprintln\n").unwrap();
        fs::write(base.join("secret.txt"), "TOP SECRET println\n").unwrap();
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
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn registry_lists_read_tier_tools_as_ollama_function_definitions() {
    let names: Vec<_> = chat_tools::TOOLS.iter().map(|tool| tool.name).collect();
    assert_eq!(
        names,
        [
            "read_file",
            "list_files",
            "grep",
            "git_status",
            "git_branch",
            "git_log",
            "git_diff",
            "write_file",
            "edit_file",
            "git_commit",
            "git_switch",
            "git_merge",
            "git_push",
            "run_command",
            "propose_plan"
        ]
    );
    assert!(chat_tools::TOOLS[..7]
        .iter()
        .all(|tool| tool.tier == Tier::Read));
    let definitions = chat_tools::definitions();
    assert_eq!(definitions.len(), chat_tools::TOOLS.len());
    for definition in &definitions {
        assert_eq!(definition["type"], "function");
        assert!(definition["function"]["name"].is_string());
        assert!(!definition["function"]["description"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(definition["function"]["parameters"]["type"], "object");
    }
    assert_eq!(
        definitions[0]["function"]["parameters"]["required"],
        json!(["path"])
    );
}

#[test]
fn unknown_tools_and_bad_arguments_are_errors_for_the_model() {
    let workspace = Workspace::new();
    assert!(workspace
        .run("delete_everything", json!({"path": "x"}))
        .starts_with("error: unknown tool"));
    assert!(workspace.run("read_file", json!({})).starts_with("error: "));
    assert!(workspace
        .run("read_file", json!({"path": 3}))
        .starts_with("error: "));
}

#[test]
fn read_file_returns_numbered_workspace_text() {
    let workspace = Workspace::new();
    let output = workspace.run("read_file", json!({"path": "README.md"}));
    assert!(output.contains("1\t# Demo"), "{output}");
    assert!(output.contains("2\tA tiny demo project."), "{output}");
    let from = workspace.run("read_file", json!({"path": "src/main.rs", "start_line": 2}));
    assert!(!from.contains("fn main"), "{from}");
    assert!(from.contains("2\t    println!"), "{from}");
}

#[test]
fn paths_outside_the_workspace_are_refused() {
    let workspace = Workspace::new();
    let absolute = workspace.base.join("secret.txt");
    for path in [
        "../secret.txt",
        "src/../../secret.txt",
        absolute.to_str().unwrap(),
        "/etc/passwd",
        ".git/HEAD",
    ] {
        let output = workspace.run("read_file", json!({ "path": path }));
        assert!(output.starts_with("error: "), "{path}: {output}");
        assert!(!output.contains("TOP SECRET"), "{path}");
    }
    assert!(workspace
        .run("list_files", json!({"path": ".."}))
        .starts_with("error: "));
    assert!(workspace
        .run("grep", json!({"pattern": "SECRET", "path": "../"}))
        .starts_with("error: "));
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_escape_the_workspace() {
    let workspace = Workspace::new();
    std::os::unix::fs::symlink(
        workspace.base.join("secret.txt"),
        workspace.root.join("leak"),
    )
    .unwrap();
    std::os::unix::fs::symlink(&workspace.base, workspace.root.join("up")).unwrap();
    for path in ["leak", "up/secret.txt"] {
        let output = workspace.run("read_file", json!({ "path": path }));
        assert!(output.starts_with("error: "), "{path}: {output}");
    }
    let listed = workspace.run("list_files", json!({}));
    assert!(!listed.contains("secret.txt"), "{listed}");
    let found = workspace.run("grep", json!({"pattern": "SECRET"}));
    assert!(!found.contains("TOP SECRET"), "{found}");
    let into = workspace.run("list_files", json!({"path": "up"}));
    assert!(into.starts_with("error: "), "{into}");
}

#[test]
fn read_file_cuts_a_single_over_long_line_at_a_char_boundary() {
    let workspace = Workspace::new();
    // 3-byte chars: 20,001 bytes, so the budget cut can fall inside a character.
    let line = "€".repeat(6667);
    fs::write(workspace.root.join("min.js"), &line).unwrap();
    let output = workspace.run("read_file", json!({"path": "min.js"}));
    assert!(output.len() <= MAX_OUTPUT, "{}", output.len());
    assert!(output.starts_with("1\t€€€"), "{}", &output[..20]);
    assert!(
        output.contains("[line 1 is 20001 bytes; showed the first "),
        "{output}"
    );
    assert!(
        !output.contains("start_line"),
        "last line has no continuation"
    );
    let shown = output.split('\n').next().unwrap().len();
    assert!(shown > 15 * 1024, "{shown}");
    let again = workspace.run("read_file", json!({"path": "min.js", "start_line": 1}));
    assert_eq!(again, output);
}

#[test]
fn read_file_continuation_advances_past_a_long_line_after_short_ones() {
    let workspace = Workspace::new();
    let long = "x".repeat(20_000);
    fs::write(
        workspace.root.join("mixed.txt"),
        format!("a\nb\n{long}\nlast\n"),
    )
    .unwrap();
    let first = workspace.run("read_file", json!({"path": "mixed.txt"}));
    assert!(first.starts_with("1\ta\n2\tb\n"), "{first}");
    assert!(!first.contains("xxx"), "long line is deferred");
    assert!(first.contains("start_line=3"), "{first}");
    let second = workspace.run("read_file", json!({"path": "mixed.txt", "start_line": 3}));
    assert!(second.starts_with("3\txxx"), "{}", &second[..20]);
    assert!(
        second.contains("[line 3 is 20000 bytes; showed the first "),
        "{second}"
    );
    assert!(second.contains("start_line=4"), "{second}");
    let third = workspace.run("read_file", json!({"path": "mixed.txt", "start_line": 4}));
    assert_eq!(third, "4\tlast\n");
}

#[test]
fn secrets_and_generated_trees_are_never_read_listed_or_searched() {
    let workspace = Workspace::new();
    fs::write(workspace.root.join(".env.local"), "TOKEN=SECRET_ENV\n").unwrap();
    fs::write(workspace.root.join("server.key"), "SECRET_KEY\n").unwrap();
    fs::create_dir_all(workspace.root.join("vendor")).unwrap();
    fs::write(workspace.root.join("vendor/lib.rs"), "SECRET_VENDOR\n").unwrap();
    for path in [".env.local", "server.key", "vendor/lib.rs"] {
        let output = workspace.run("read_file", json!({ "path": path }));
        assert!(output.starts_with("error: "), "{path}: {output}");
    }
    let listed = workspace.run("list_files", json!({}));
    assert!(
        !listed.contains(".env") && !listed.contains("server.key") && !listed.contains("vendor"),
        "{listed}"
    );
    let found = workspace.run("grep", json!({"pattern": "SECRET_"}));
    assert_eq!(found.trim(), "no matches");
}

#[test]
fn outputs_are_size_capped_with_a_marker() {
    let workspace = Workspace::new();
    let line = "y".repeat(99) + "\n";
    fs::write(workspace.root.join("big.txt"), line.repeat(4000)).unwrap();
    let output = workspace.run("read_file", json!({"path": "big.txt"}));
    assert!(output.len() <= MAX_OUTPUT + 200, "{}", output.len());
    assert!(output.contains("[truncated"), "marker missing");
    assert!(output.contains("start_line"), "says how to read on");
    for index in 0..800 {
        fs::write(workspace.root.join(format!("src/f{index}.txt")), "needle\n").unwrap();
    }
    let listed = workspace.run("list_files", json!({}));
    assert!(listed.len() <= MAX_OUTPUT + 200);
    assert!(
        listed.contains("[truncated"),
        "{}",
        &listed[listed.len() - 100..]
    );
    let found = workspace.run("grep", json!({"pattern": "needle"}));
    assert!(found.len() <= MAX_OUTPUT + 200);
    assert!(found.contains("[truncated"));
}

#[test]
fn binary_files_are_not_dumped() {
    let workspace = Workspace::new();
    fs::write(workspace.root.join("blob.bin"), [0u8, 159, 146, 150, 0, 1]).unwrap();
    let output = workspace.run("read_file", json!({"path": "blob.bin"}));
    assert!(output.starts_with("error: "), "{output}");
}

#[test]
fn list_files_walks_the_tree_without_git_metadata() {
    let workspace = Workspace::new();
    let listed = workspace.run("list_files", json!({}));
    assert!(listed.lines().any(|line| line == "README.md"), "{listed}");
    assert!(listed.lines().any(|line| line == "src/main.rs"), "{listed}");
    assert!(!listed.contains(".git"), "{listed}");
    let sub = workspace.run("list_files", json!({"path": "src"}));
    assert_eq!(sub.trim(), "src/main.rs");
}

#[test]
fn grep_reports_path_line_and_text_and_skips_git() {
    let workspace = Workspace::new();
    let found = workspace.run("grep", json!({"pattern": "println"}));
    assert_eq!(found.trim(), "src/main.rs:2:    println!(\"hi\");");
    let none = workspace.run("grep", json!({"pattern": "absent-text"}));
    assert_eq!(none.trim(), "no matches");
    let ignored_case = workspace.run("grep", json!({"pattern": "DEMO", "ignore_case": true}));
    assert!(
        ignored_case.contains("README.md:1:# Demo"),
        "{ignored_case}"
    );
}

// Issue #343: discovery honours git's ignore rules.

/// A Workspace whose `.git` is a real repository (the plain one has a fake `.git`).
fn git_workspace() -> Workspace {
    let workspace = Workspace::new();
    fs::remove_dir_all(workspace.root.join(".git")).unwrap();
    git(&workspace, &["init", "-q"]);
    workspace
}

fn git(workspace: &Workspace, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&workspace.root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn put(workspace: &Workspace, path: &str, body: &str) {
    let full = workspace.root.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, body).unwrap();
}

fn lines(output: &str) -> Vec<&str> {
    output.lines().collect()
}

/// The repro of issue #343: a Python repo whose ignored `.venv` would fill the caps.
fn python_repo() -> Workspace {
    let workspace = git_workspace();
    put(
        &workspace,
        ".gitignore",
        ".venv/\n__pycache__/\nbuild/\n*.log\n",
    );
    for n in 0..600 {
        put(
            &workspace,
            &format!(".venv/lib/python3.12/site-packages/pkg{n}/mod.py"),
            "import os\ndef handle_request_x():\n",
        );
    }
    for n in 0..300 {
        put(
            &workspace,
            &format!("build/lib/m{n}.py"),
            "def handle_request_b():\n",
        );
    }
    put(
        &workspace,
        "src/app.py",
        "def handle_request():\n    pass\n",
    );
    put(&workspace, "tests/test_app.py", "import app\n");
    put(&workspace, "x.log", "needle log\n");
    workspace
}

#[test]
fn list_files_and_grep_skip_git_ignored_paths_so_project_files_come_first() {
    let workspace = python_repo();
    let listed = workspace.run("list_files", json!({}));
    for wanted in [".gitignore", "src/app.py", "tests/test_app.py"] {
        assert!(
            lines(&listed).contains(&wanted),
            "{wanted} missing: {listed}"
        );
    }
    for hidden in [".venv", "build/", "x.log"] {
        assert!(!listed.contains(hidden), "{hidden} listed: {listed}");
    }
    assert!(!listed.contains("[truncated"), "{listed}");
    let found = workspace.run("grep", json!({"pattern": "def handle_request"}));
    assert_eq!(found.trim(), "src/app.py:1:def handle_request():");
    let log = workspace.run("grep", json!({"pattern": "needle"}));
    assert_eq!(log.trim(), "no matches");
}

#[test]
fn tracked_files_stay_visible_even_when_they_match_an_ignore_pattern() {
    let workspace = python_repo();
    put(&workspace, "kept.log", "needle kept\n");
    git(&workspace, &["add", "-f", "kept.log"]);
    let listed = workspace.run("list_files", json!({}));
    assert!(lines(&listed).contains(&"kept.log"), "{listed}");
    assert!(!lines(&listed).contains(&"x.log"), "{listed}");
    let found = workspace.run("grep", json!({"pattern": "needle"}));
    assert_eq!(found.trim(), "kept.log:1:needle kept");
}

#[test]
fn nested_gitignore_and_info_exclude_are_honoured() {
    let workspace = git_workspace();
    put(&workspace, "pkg/.gitignore", "generated.txt\n");
    put(&workspace, "pkg/generated.txt", "needle gen\n");
    put(&workspace, "pkg/real.txt", "needle real\n");
    put(&workspace, "scratch.txt", "needle scratch\n");
    fs::write(workspace.root.join(".git/info/exclude"), "scratch.txt\n").unwrap();
    let listed = workspace.run("list_files", json!({}));
    assert!(lines(&listed).contains(&"pkg/real.txt"), "{listed}");
    assert!(!listed.contains("generated.txt"), "{listed}");
    assert!(!listed.contains("scratch.txt"), "{listed}");
    let found = workspace.run("grep", json!({"pattern": "needle"}));
    assert_eq!(found.trim(), "pkg/real.txt:1:needle real");
}

#[test]
fn virtualenv_and_cache_directories_are_skipped_without_a_gitignore() {
    for in_git in [true, false] {
        let workspace = if in_git {
            git_workspace()
        } else {
            Workspace::new()
        };
        for dir in [".venv", "venv", "pkg/__pycache__", "lib/site-packages"] {
            put(&workspace, &format!("{dir}/mod.py"), "needle env\n");
        }
        put(&workspace, "build/gen.py", "needle build\n");
        let listed = workspace.run("list_files", json!({}));
        assert!(
            lines(&listed).contains(&"build/gen.py"),
            "{in_git}: {listed}"
        );
        for hidden in ["venv", "__pycache__", "site-packages"] {
            assert!(!listed.contains(hidden), "{in_git} {hidden}: {listed}");
        }
        let found = workspace.run("grep", json!({"pattern": "needle"}));
        assert_eq!(found.trim(), "build/gen.py:1:needle build", "{in_git}");
    }
}

#[test]
fn an_explicit_path_inside_an_ignored_directory_still_lists_with_a_note() {
    let workspace = python_repo();
    let listed = workspace.run("list_files", json!({"path": ".venv"}));
    assert!(
        listed.contains(".venv/lib/python3.12/site-packages/pkg0/mod.py"),
        "{listed}"
    );
    assert!(listed.to_lowercase().contains("ignored"), "{listed}");
    let built = workspace.run("list_files", json!({"path": "build"}));
    assert!(built.contains("build/lib/m0.py"), "{built}");
    assert!(built.to_lowercase().contains("ignored"), "{built}");
    let found = workspace.run(
        "grep",
        json!({"pattern": "handle_request_b", "path": "build"}),
    );
    assert!(
        found.contains("build/lib/m0.py:1:def handle_request_b():"),
        "{found}"
    );
    // A path that is not ignored carries no note.
    let src = workspace.run("list_files", json!({"path": "src"}));
    assert_eq!(src.trim(), "src/app.py\nsrc/main.rs");
}

#[test]
fn read_file_on_an_ignored_non_secret_file_is_unchanged() {
    let workspace = python_repo();
    let out = workspace.run("read_file", json!({"path": "x.log"}));
    assert_eq!(out.trim(), "1\tneedle log");
}

#[test]
fn a_gitignore_never_re_includes_secrets_or_generated_trees() {
    let workspace = git_workspace();
    put(
        &workspace,
        ".gitignore",
        "!.env\n!*.pem\n!vendor/\n!target/\n",
    );
    put(&workspace, ".env", "SECRET_ENV\n");
    put(&workspace, "id.pem", "SECRET_PEM\n");
    put(&workspace, "vendor/lib.rs", "SECRET_VENDOR\n");
    put(&workspace, "target/out.txt", "SECRET_TARGET\n");
    put(&workspace, "ok.txt", "fine\n");
    git(
        &workspace,
        &["add", "-f", ".env", "id.pem", "vendor/lib.rs"],
    );
    let listed = workspace.run("list_files", json!({}));
    assert!(lines(&listed).contains(&"ok.txt"), "{listed}");
    for hidden in [".env", ".pem", "vendor", "target", ".git/"] {
        assert!(!listed.contains(hidden), "{hidden}: {listed}");
    }
    assert_eq!(
        workspace.run("grep", json!({"pattern": "SECRET_"})).trim(),
        "no matches"
    );
    for path in [".env", "id.pem", "vendor/lib.rs", ".git/HEAD"] {
        let out = workspace.run("read_file", json!({"path": path}));
        assert!(out.starts_with("error: "), "{path}: {out}");
    }
}

/// Overwrite `.git/index` with version-2 entries named `names` (git does not validate the
/// names it reads), each a stage-0 regular file; the checksum comes from `sha1sum`.
fn craft_index(workspace: &Workspace, names: &[&str]) {
    use std::io::Write;
    let mut names: Vec<&str> = names.to_vec();
    names.sort_unstable();
    let mut body = b"DIRC".to_vec();
    body.extend(2u32.to_be_bytes());
    body.extend((names.len() as u32).to_be_bytes());
    for name in names {
        let mut entry = Vec::new();
        for field in [0u32, 0, 0, 0, 0, 0, 0o100644, 0, 0, 3] {
            entry.extend(field.to_be_bytes());
        }
        // The blob id of "hi\n"; discovery never reads objects.
        entry.extend([
            0x45, 0xb9, 0x83, 0xbe, 0x36, 0xb7, 0x3c, 0x07, 0x88, 0xdc, 0x9c, 0xbc, 0xb7, 0x6c,
            0xbb, 0x80, 0xfc, 0x7b, 0xb0, 0x57,
        ]);
        entry.extend((name.len() as u16).to_be_bytes());
        entry.extend(name.as_bytes());
        let pad = 8 - entry.len() % 8;
        entry.extend(std::iter::repeat_n(0, pad));
        body.extend(entry);
    }
    let mut child = std::process::Command::new("sha1sum")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&body).unwrap();
    let out = child.wait_with_output().unwrap().stdout;
    let hex = String::from_utf8(out).unwrap();
    for pair in hex.as_bytes()[..40].chunks(2) {
        body.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap());
    }
    fs::write(workspace.root.join(".git/index"), body).unwrap();
}

#[cfg(unix)]
#[test]
fn a_crafted_index_entry_cannot_list_or_search_a_file_outside_the_workspace() {
    let workspace = git_workspace();
    put(&workspace, "ok.txt", "needle ok\n");
    let outside = workspace.base.join("outside-343");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "needle OUTSIDE\n").unwrap();
    let absolute = outside.join("secret.txt").to_string_lossy().into_owned();
    // The same file reached by `..` from the workspace root.
    let climbing = format!(
        "../{}/outside-343/secret.txt",
        workspace.base.file_name().unwrap().to_string_lossy()
    );
    craft_index(
        &workspace,
        &["ok.txt", &absolute, &climbing, "a/../ok.txt", "./ok.txt"],
    );
    let listed = workspace.run("list_files", json!({}));
    let found = workspace.run("grep", json!({"pattern": "needle"}));
    for out in [&listed, &found] {
        assert!(!out.contains("OUTSIDE"), "{out}");
        assert!(!out.contains("secret.txt"), "{out}");
        assert!(!out.contains("outside-343"), "{out}");
    }
    assert!(lines(&listed).contains(&"ok.txt"), "{listed}");
    assert!(found.contains("ok.txt:1:needle ok"), "{found}");
}

#[test]
fn directories_named_like_secret_files_are_not_listed_or_searched() {
    let workspace = git_workspace();
    put(&workspace, "ok.txt", "fine\n");
    for dir in ["tracked.key", "tracked.pem", "loose.key", "loose.pem"] {
        put(&workspace, &format!("{dir}/inner.txt"), "SECRET_DIR\n");
        put(&workspace, &format!("sub/{dir}/inner.txt"), "SECRET_DIR\n");
    }
    git(
        &workspace,
        &[
            "add",
            "tracked.key/inner.txt",
            "tracked.pem/inner.txt",
            "sub/tracked.key/inner.txt",
            "sub/tracked.pem/inner.txt",
        ],
    );
    let listed = workspace.run("list_files", json!({}));
    assert!(lines(&listed).contains(&"ok.txt"), "{listed}");
    assert!(!listed.contains("inner.txt"), "{listed}");
    assert_eq!(
        workspace
            .run("grep", json!({"pattern": "SECRET_DIR"}))
            .trim(),
        "no matches"
    );
}

#[cfg(unix)]
#[test]
fn a_tracked_path_behind_a_swapped_in_symlink_is_not_listed_or_searched() {
    let workspace = git_workspace();
    put(&workspace, "link/inner.txt", "needle inside\n");
    git(&workspace, &["add", "link/inner.txt"]);
    // The directory is replaced by a symlink to a folder outside the workspace.
    let outside = workspace.base.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("inner.txt"), "needle OUTSIDE\n").unwrap();
    fs::remove_dir_all(workspace.root.join("link")).unwrap();
    std::os::unix::fs::symlink(&outside, workspace.root.join("link")).unwrap();
    put(&workspace, "ok.txt", "needle ok\n");
    let listed = workspace.run("list_files", json!({}));
    assert!(!listed.contains("link"), "{listed}");
    let found = workspace.run("grep", json!({"pattern": "needle"}));
    assert_eq!(found.trim(), "ok.txt:1:needle ok");
}

#[test]
fn when_git_cannot_list_the_plain_walk_is_the_fallback() {
    // Fake `.git`: git fails, so ignore files are not applied but the fixed
    // exclusions still are.
    let workspace = Workspace::new();
    put(&workspace, ".gitignore", "*.log\n");
    put(&workspace, "x.log", "needle log\n");
    put(&workspace, ".venv/a.py", "needle venv\n");
    let listed = workspace.run("list_files", json!({}));
    assert!(lines(&listed).contains(&"x.log"), "{listed}");
    assert!(!listed.contains(".venv"), "{listed}");
    // Repository filter drivers make the git listing refuse; same fallback.
    let workspace = git_workspace();
    put(&workspace, ".gitignore", "*.log\n");
    put(&workspace, "x.log", "needle log\n");
    git(&workspace, &["config", "filter.x.clean", "cat"]);
    let listed = workspace.run("list_files", json!({}));
    assert!(lines(&listed).contains(&"x.log"), "{listed}");
}

#[test]
fn a_repository_config_cannot_make_discovery_run_programs() {
    // Repo-local fsmonitor and hooks must not run for the git listing; a .gitignore
    // negation cannot re-include an excluded path.
    let workspace = git_workspace();
    let ran = workspace.root.parent().unwrap().join(format!(
        "ran-343-{}-{}",
        std::process::id(),
        workspace.root.file_name().unwrap().to_string_lossy()
    ));
    let _ = fs::remove_file(&ran);
    let script = workspace.root.join("fsmon.sh");
    fs::write(&script, format!("#!/bin/sh\ntouch {}\n", ran.display())).unwrap();
    git(
        &workspace,
        &["config", "core.fsmonitor", script.to_str().unwrap()],
    );
    git(
        &workspace,
        &["config", "core.hooksPath", workspace.root.to_str().unwrap()],
    );
    put(&workspace, "ok.txt", "needle ok\n");
    put(&workspace, ".env", "needle secret\n");
    put(&workspace, ".gitignore", "!.env\n");
    let listed = workspace.run("list_files", json!({}));
    let found = workspace.run("grep", json!({"pattern": "needle"}));
    assert!(lines(&listed).contains(&"ok.txt"), "{listed}");
    assert!(!listed.contains(".env"), "{listed}");
    assert_eq!(found.trim(), "ok.txt:1:needle ok");
    assert!(!ran.exists(), "repository fsmonitor ran");
    let _ = fs::remove_file(&ran);
}
