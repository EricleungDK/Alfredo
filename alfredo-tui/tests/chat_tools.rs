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
            "edit_file"
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
