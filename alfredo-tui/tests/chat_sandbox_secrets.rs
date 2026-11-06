//! Issue #469: a sandboxed `run_command` must never hand a secret file to the model.
//! The chat sandbox binds the whole Coding Workspace read-write, so before this fix a
//! rule-approved prefix command (`cat README.md`) also ran `cat README.md .env` with no
//! card. Every workspace file the secret predicate matches is now masked inside the
//! sandbox (bound over with `/dev/null`), and every secret directory is masked with an
//! empty read-only directory, so the sandbox sees what the read tools see.
use alfredo_tui::{
    chat_tools::{
        self, allow,
        approval::{self, Card, CardKind},
    },
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

const SECRET: &str = "s3cr3t";

/// A fresh canonical workspace; each test writes the files it needs into it.
fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-secrets-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Run `command` through the chat tool dispatch, as an approved call would.
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
fn sandboxed_prefix_rule_never_hands_secret_files_to_the_model() {
    let root = workspace("exfil");
    write(&root, ".env", "API_TOKEN=s3cr3t\n");
    write(&root, "README.md", "hi\n");
    assert!(chat_tools::resolve(&root, ".env").is_err());
    let seed = Card {
        tool: "run_command".into(),
        arguments: json!({"command": "cat README.md", "escalate": false}),
        kind: CardKind::Shell { escalated: false },
        summary: String::new(),
        preview: String::new(),
        written: None,
        fingerprint: "/x".into(),
    };
    let rule = allow::rule_for(&seed).unwrap();
    let call = json!({"command": "cat README.md .env"});
    let card = approval::card(
        &root,
        &ToolCall {
            name: "run_command".into(),
            arguments: call.clone(),
        },
    )
    .unwrap();
    let auto = allow::matches(&rule, &card);
    let out = chat_tools::dispatch(
        &root,
        &ToolCall {
            name: "run_command".into(),
            arguments: call,
        },
    );
    assert!(
        !(auto && out.contains("s3cr3t")),
        "rule-approved command read a secret file: {out}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_rule_approved_recursive_grep_does_not_return_env_contents() {
    let root = workspace("grep");
    write(&root, ".env", &format!("API_TOKEN={SECRET}\n"));
    write(&root, "README.md", "the TOKEN_OK marker\n");
    let out = run(&root, "grep -r TOKEN . ; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(
        out.contains("TOKEN_OK"),
        "a visible file is not searched: {out}"
    );
    assert!(!out.contains(SECRET), "grep returned .env contents: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_secret_directory_is_hidden_from_the_sandbox() {
    let root = workspace("ssh");
    write(&root, ".ssh/id_rsa", &format!("PRIVATE {SECRET}\n"));
    write(&root, ".ssh/config", "Host example\n");
    let out = run(
        &root,
        "cat .ssh/id_rsa 2>/dev/null; grep -r PRIVATE . ; echo END",
    );
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a .ssh file leaked: {out}");
    // The listing alone: the command's own stderr may echo a name the model typed.
    let listing = run(&root, "ls -A .ssh; echo END");
    assert!(
        listing.contains("END"),
        "the listing did not run: {listing}"
    );
    assert!(!listing.contains("id_rsa"), "a .ssh name leaked: {listing}");
    assert!(!listing.contains("config"), "a .ssh name leaked: {listing}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_nested_secret_file_is_hidden_and_the_host_copy_is_unchanged() {
    let root = workspace("nested");
    write(&root, "sub/.env", &format!("NESTED={SECRET}\n"));
    write(&root, "sub/app.txt", "visible\n");
    let out = run(
        &root,
        "cat sub/.env; grep -r NESTED . ; echo HACKED > sub/.env 2>/dev/null; cat sub/app.txt; echo END",
    );
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a nested .env leaked: {out}");
    assert!(
        out.contains("visible"),
        "a non-secret file is unreadable: {out}"
    );
    assert_eq!(
        fs::read_to_string(root.join("sub/.env")).unwrap(),
        format!("NESTED={SECRET}\n"),
        "a masked secret file was overwritten"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_gitignored_secret_file_in_a_repository_is_hidden() {
    let root = workspace("ignored");
    let git = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(&root)
        .status();
    assert!(git.is_ok_and(|s| s.success()), "git init failed");
    write(&root, ".gitignore", ".env\n");
    write(&root, ".env", &format!("IGNORED={SECRET}\n"));
    write(&root, "src/lib.txt", "tracked\n");
    let out = run(&root, "grep -r IGNORED . ; cat src/lib.txt; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(
        out.contains("tracked"),
        "a tracked file is unreadable: {out}"
    );
    assert!(!out.contains(SECRET), "a gitignored .env leaked: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_non_secret_file_stays_readable_and_writable() {
    let root = workspace("plain");
    write(&root, "README.md", "readme text\n");
    let out = run(&root, "cat README.md; echo made > out.txt; echo END");
    assert!(out.contains("readme text"), "{out}");
    assert!(out.contains("END"), "{out}");
    assert_eq!(fs::read_to_string(root.join("out.txt")).unwrap(), "made\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_secret_named_symlink_refuses_the_run_and_runs_nothing() {
    let root = workspace("symlink-secret");
    write(&root, "README.md", "hi\n");
    std::os::unix::fs::symlink("README.md", root.join(".env")).unwrap();
    let out = run(&root, "touch ran");
    assert!(out.contains("nothing ran"), "{out}");
    assert!(!out.contains("exit code"), "the command ran: {out}");
    assert!(!root.join("ran").exists(), "the command ran: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_symlink_to_a_masked_file_reads_the_mask() {
    let root = workspace("symlink-file");
    write(&root, ".env", &format!("LINKED={SECRET}\n"));
    std::os::unix::fs::symlink(".env", root.join("link.txt")).unwrap();
    let out = run(&root, "cat link.txt; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a symlink to .env leaked: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_symlink_into_a_secret_directory_reads_the_mask() {
    let root = workspace("symlink-dir");
    write(&root, ".ssh/id_rsa", &format!("DIR {SECRET}\n"));
    std::os::unix::fs::symlink(".ssh", root.join("creds")).unwrap();
    let out = run(&root, "cat creds/id_rsa; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a symlink into .ssh leaked: {out}");
    let _ = fs::remove_dir_all(root);
}

/// Whether this process is root, which reads any directory and so cannot test an
/// unreadable one.
fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "0")
        .unwrap_or(false)
}

/// Restore the mode of `path` so the workspace can be removed after the test.
fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Issue #581: a non-secret directory the scan cannot list is bound over with the empty
/// read-only directory instead of refusing every run.
#[test]
fn an_unreadable_non_secret_directory_runs_and_looks_empty() {
    if is_root() {
        eprintln!("skipped: root reads every directory");
        return;
    }
    let root = workspace("unreadable");
    write(&root, "a.txt", "x");
    write(&root, "data/pg/x", "");
    chmod(&root.join("data/pg"), 0o000);
    let out = run(&root, "true");
    let listed = run(&root, "ls -A data/pg | wc -l; echo END");
    chmod(&root.join("data/pg"), 0o755);
    assert!(out.contains("exit code 0"), "the run was refused: {out}");
    assert!(!out.contains("nothing ran"), "{out}");
    assert!(listed.contains("0\nEND"), "data/pg is not empty: {listed}");
    let _ = fs::remove_dir_all(root);
}

/// Issue #581: more than 250,000 entries no longer refuses, and a secret in the tree stays
/// masked.
#[test]
fn a_workspace_over_the_old_entry_limit_runs_and_keeps_env_masked() {
    let root = workspace("big");
    write(&root, ".env", &format!("BIG={SECRET}\n"));
    for dir in 0..300 {
        let base = root.join(format!("bulk/d{dir}"));
        fs::create_dir_all(&base).unwrap();
        for file in 0..1000 {
            fs::File::create(base.join(format!("f{file}"))).unwrap();
        }
    }
    let out = run(&root, "cat .env; echo END");
    assert!(out.contains("END"), "the run was refused: {out}");
    assert!(!out.contains("nothing ran"), "{out}");
    assert!(!out.contains(SECRET), "the mask leaked: {out}");
    let _ = fs::remove_dir_all(root);
}

/// Issue #581: an unreadable directory takes one of the 128 masks, so too many still refuse.
#[test]
fn unreadable_directories_count_toward_the_mask_cap() {
    if is_root() {
        eprintln!("skipped: root reads every directory");
        return;
    }
    let root = workspace("unreadable-cap");
    for index in 0..129 {
        write(&root, &format!("d{index}/x"), "");
        chmod(&root.join(format!("d{index}")), 0o000);
    }
    let out = run(&root, "touch ran");
    for index in 0..129 {
        chmod(&root.join(format!("d{index}")), 0o755);
    }
    assert!(out.contains("nothing ran"), "{out}");
    assert!(!root.join("ran").exists(), "the command ran: {out}");
    assert!(
        out.contains("more than 128 secret files and directories"),
        "{out}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Issue #581: a directory that can be entered but not listed (mode 0111) is masked too, so
/// a secret file inside it by a known name is not readable.
#[test]
fn a_search_only_directory_hides_a_secret_inside_it() {
    if is_root() {
        eprintln!("skipped: root reads every directory");
        return;
    }
    let root = workspace("search-only");
    write(&root, "vol/.env", &format!("VOL={SECRET}\n"));
    chmod(&root.join("vol"), 0o111);
    let out = run(&root, "cat vol/.env; echo END");
    chmod(&root.join("vol"), 0o755);
    assert!(out.contains("END"), "the run was refused: {out}");
    assert!(
        !out.contains(SECRET),
        "a secret in an unlistable directory leaked: {out}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn too_many_secret_masks_refuse_the_run_and_run_nothing() {
    let root = workspace("cap");
    for index in 0..129 {
        write(&root, &format!("many/.env{index}"), "X\n");
    }
    let out = run(&root, "touch ran");
    assert!(out.contains("nothing ran"), "{out}");
    assert!(!out.contains("exit code"), "the command ran: {out}");
    assert!(!root.join("ran").exists(), "the command ran: {out}");
    assert!(
        out.contains("more than 128 secret files and directories"),
        "{out}"
    );
    assert!(
        out.contains("remove or rename them, or move them out of the Coding Workspace"),
        "{out}"
    );
    let _ = fs::remove_dir_all(root);
}

/// The note line of a result, if any.
fn note_line(out: &str) -> Option<&str> {
    out.lines()
        .find(|line| line.contains("hidden in this sandbox"))
}

#[test]
fn a_masked_npmrc_read_explains_the_hidden_file_with_a_count_only() {
    let root = workspace("note");
    write(&root, ".npmrc", "registry=x\n");
    let out = run(&root, "cat .npmrc");
    assert!(out.contains("Permission denied"), "{out}");
    assert!(out.contains("hidden in this sandbox"), "{out}");
    let note = note_line(&out).unwrap();
    assert_eq!(
        note,
        "note: 1 secret file(s)/directory(ies) are hidden in this sandbox and are not readable: a hidden file fails with Permission denied and a hidden directory looks empty (its contents report No such file or directory); they need the owner, do not retry on the host to read them"
    );
    assert!(!note.contains("npmrc"), "the note names the file: {note}");
    assert!(!out.contains("registry=x"), "contents leaked: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_hidden_note_counts_files_and_directories_and_shows_on_a_successful_run() {
    let root = workspace("note-count");
    write(&root, ".env", "A=1\n");
    write(&root, ".ssh/id_rsa", "k\n");
    let out = run(&root, "echo END");
    assert!(out.contains("exit code 0"), "{out}");
    assert!(
        note_line(&out)
            .is_some_and(|n| n.starts_with("note: 2 secret file(s)/directory(ies) are hidden")),
        "{out}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn no_hidden_note_when_there_are_no_secret_files() {
    let root = workspace("note-none");
    write(&root, "README.md", "hi\n");
    let out = run(&root, "echo Permission denied >&2; exit 1");
    assert!(out.contains("exit code 1"), "{out}");
    assert!(out.contains("Permission denied"), "{out}");
    assert!(!out.contains("hidden in this sandbox"), "{out}");
    let _ = fs::remove_dir_all(root);
}

const TEMPLATES: [&str; 3] = [".env.example", ".env.sample", ".env.template"];

#[test]
fn sandboxed_env_templates_are_readable() {
    let root = workspace("tpl-read");
    for name in TEMPLATES {
        write(&root, name, &format!("ROOT_{name}=1\n"));
        write(
            &root,
            &format!("sub/deeper/{name}"),
            &format!("DEEP_{name}=1\n"),
        );
    }
    write(&root, ".env", &format!("S={SECRET}\n"));
    let mut command = String::new();
    for name in TEMPLATES {
        command.push_str(&format!("cat {name} sub/deeper/{name}; "));
    }
    let out = run(&root, &command);
    assert!(out.contains("exit code 0"), "{out}");
    for name in TEMPLATES {
        assert!(out.contains(&format!("ROOT_{name}=1")), "{out}");
        assert!(out.contains(&format!("DEEP_{name}=1")), "{out}");
    }
    assert!(!out.contains(SECRET), "{out}");
    let copy = run(&root, "cp .env.example copy.txt && cat copy.txt");
    assert!(copy.contains("exit code 0"), "{copy}");
    assert!(copy.contains("ROOT_.env.example=1"), "{copy}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sandboxed_env_templates_are_read_only() {
    let root = workspace("tpl-ro");
    for name in TEMPLATES {
        write(&root, name, "A=1\n");
        write(&root, &format!("sub/{name}"), "A=1\n");
    }
    // Readable but not writable: a template that is merely hidden would also fail the
    // writes below, so the read comes first.
    let seen = run(&root, "cat .env.example sub/.env.template");
    assert!(seen.contains("exit code 0"), "{seen}");
    for name in TEMPLATES {
        for path in [name.to_string(), format!("sub/{name}")] {
            for command in [
                format!("echo x >> {path}"),
                format!("echo x > {path}"),
                format!("rm {path}"),
                format!("mv {path} moved.txt"),
            ] {
                let out = run(&root, &command);
                assert!(!out.contains("exit code 0"), "{command}: {out}");
                assert!(out.contains("exit code"), "{command}: {out}");
                assert_eq!(
                    fs::read_to_string(root.join(&path)).unwrap(),
                    "A=1\n",
                    "{command}"
                );
                assert!(!root.join("moved.txt").exists(), "{command}");
            }
        }
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn non_template_env_files_stay_masked() {
    let root = workspace("tpl-masked");
    write(&root, ".env.example", "TEMPLATE=1\n");
    write(&root, ".env", &format!("A={SECRET}\n"));
    write(&root, ".env.example.local", &format!("B={SECRET}\n"));
    write(&root, ".envrc", &format!("C={SECRET}\n"));
    write(&root, "sub/.env.sample/inner.txt", &format!("E={SECRET}\n"));
    let out = run(
        &root,
        "cat .env.example; cat .env; cat .env.example.local; cat .envrc; cat sub/.env.sample/inner.txt; echo END",
    );
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a secret leaked: {out}");
    assert!(out.contains("Permission denied"), "{out}");
    assert!(
        out.contains("TEMPLATE=1"),
        "the template next to them is readable: {out}"
    );
    let _ = fs::remove_dir_all(root);

    // A directory named like a template exempts nothing inside it.
    let root = workspace("tpl-dir");
    write(&root, ".env.example/inner.txt", &format!("D={SECRET}\n"));
    let out = run(
        &root,
        "cat .env.example/inner.txt; ls .env.example; echo END",
    );
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a secret leaked: {out}");
    assert!(out.contains("No such file or directory"), "{out}");
    let _ = fs::remove_dir_all(root);

    // A template that is a symbolic link is still refused, never bound or followed.
    let root = workspace("tpl-link");
    write(&root, ".env", &format!("A={SECRET}\n"));
    std::os::unix::fs::symlink(".env", root.join(".env.example")).unwrap();
    let out = run(&root, "cat .env.example");
    assert!(out.contains("nothing ran"), "{out}");
    assert!(!out.contains(SECRET), "{out}");
    let _ = fs::remove_dir_all(root);

    let root = workspace("tpl-link-out");
    let outside = workspace("tpl-link-target");
    write(&outside, "secret.txt", &format!("F={SECRET}\n"));
    std::os::unix::fs::symlink(outside.join("secret.txt"), root.join(".env.template")).unwrap();
    let out = run(&root, "cat .env.template");
    assert!(out.contains("nothing ran"), "{out}");
    assert!(!out.contains(SECRET), "{out}");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn templates_do_not_count_toward_the_mask_limit() {
    let root = workspace("tpl-cap");
    for index in 0..128 {
        write(&root, &format!("many/d{index}/.env.example"), "X=1\n");
    }
    write(&root, ".env", &format!("S={SECRET}\n"));
    // Which templates are bound depends on directory order, so count them instead of
    // naming one: the run is not refused and at least the bind budget (120) is readable.
    let out = run(
        &root,
        "cat .env; n=0; for f in many/*/.env.example; do grep -q X=1 \"$f\" 2>/dev/null && n=$((n+1)); done; echo READABLE=$n",
    );
    assert!(!out.contains("nothing ran"), "{out}");
    assert!(!out.contains("more than 128"), "{out}");
    assert!(out.contains("exit code"), "{out}");
    assert!(!out.contains(SECRET), "{out}");
    let readable: usize = out
        .lines()
        .find_map(|line| line.strip_prefix("READABLE="))
        .unwrap_or_else(|| panic!("no count: {out}"))
        .parse()
        .unwrap();
    assert!(readable >= 120, "only {readable} templates readable: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn hidden_note_and_escalate_hint_do_not_contradict() {
    let root = workspace("tpl-hints");
    write(&root, ".npmrc", "registry=x\n");
    write(&root, ".ssh/id_rsa", "k\n");
    let out = run(&root, "cat .npmrc .ssh/id_rsa");
    assert!(out.contains("exit code 1"), "{out}");
    let note = note_line(&out).unwrap();
    assert!(note.contains("2 secret file(s)/directory(ies)"), "{note}");
    assert!(note.contains("No such file or directory"), "{note}");
    assert!(note.contains("looks empty"), "{note}");
    assert!(
        !note.contains("npmrc") && !note.contains("id_rsa"),
        "{note}"
    );
    let escalates = out.contains("escalate: true");
    let forbids = out.contains("do not retry on the host");
    assert!(!(escalates && forbids), "contradictory hints: {out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_unmasked_runs_result_text_is_unchanged() {
    let root = workspace("note-bytes");
    write(&root, "README.md", "hi\n");
    let ok = run(&root, "echo hi");
    assert!(
        ok.starts_with("exit code 0 \u{b7} sandboxed \u{b7} cwd "),
        "{ok}"
    );
    assert!(ok.ends_with("\nstdout:\nhi\nstderr:\n"), "{ok}");
    assert_eq!(ok.lines().count(), 4, "{ok}");
    let failed = run(&root, "exit 3");
    let lines: Vec<&str> = failed.lines().collect();
    assert_eq!(lines.len(), 5, "{failed}");
    assert!(
        lines[0].starts_with("exit code 3 \u{b7} sandboxed"),
        "{failed}"
    );
    assert!(
        lines[1].starts_with("note: ran sandboxed (network off;"),
        "{failed}"
    );
    assert!(
        lines[1].ends_with("the owner must approve running it on the host."),
        "{failed}"
    );
    assert_eq!(&lines[2..], ["stdout:", "", "stderr:"], "{failed}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn secret_masks_up_to_the_cap_still_run() {
    let root = workspace("cap-ok");
    for index in 0..128 {
        write(&root, &format!("many/.env{index}"), "X\n");
    }
    let out = run(&root, "echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(out.contains("exit code 0"), "{out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_nested_git_directory_is_hidden_from_the_sandbox() {
    let root = workspace("nested-git");
    write(
        &root,
        "notgit/.git/config",
        &format!("[remote]\nurl = https://u:{SECRET}@host\n"),
    );
    write(
        &root,
        "notgit/.git/secret.env",
        &format!("NESTED_GIT={SECRET}\n"),
    );
    let out = run(
        &root,
        "cat notgit/.git/config notgit/.git/secret.env; grep -r NESTED_GIT . ; echo END",
    );
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(
        !out.contains(SECRET),
        "a nested .git directory leaked: {out}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_nested_git_file_is_masked() {
    let root = workspace("nested-git-file");
    write(
        &root,
        "ptr/.git",
        &format!("gitdir: ../elsewhere {SECRET}\n"),
    );
    let out = run(&root, "cat ptr/.git; echo END");
    assert!(out.contains("END"), "the command did not run: {out}");
    assert!(!out.contains(SECRET), "a nested .git file leaked: {out}");
    let _ = fs::remove_dir_all(root);
}
