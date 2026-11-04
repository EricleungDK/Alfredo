//! Issue #486: user-defined slash commands load only from `<state dir>/commands`.
use alfredo_tui::{
    commands::Completion,
    model::{Session, Status, MAX_DRAFT},
    user_commands::{
        claims, directory, expand, is_blocked, listed, publish, reload, resolve, scan, send,
        sent_notice, stage, summarise_first, Resolved, MAX_BYTES,
    },
};
use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

/// `publish` sets process-wide type-ahead state; tests that touch it run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "alfredo-user-commands-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn commands_in(root: &Path) -> std::path::PathBuf {
    let dir = directory(root);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn commands_live_under_the_state_directory_only() {
    assert_eq!(directory(Path::new("/state")), Path::new("/state/commands"));
}

#[test]
fn a_name_clashing_with_a_built_in_is_refused_and_named() {
    let root = Temp::new();
    let dir = commands_in(root.path());
    for name in ["go", "help", "ship"] {
        fs::write(dir.join(format!("{name}.md")), "body").unwrap();
    }
    let found = scan(&dir);
    assert_eq!(found.names, ["/ship"]);
    let notice = found.notice().expect("a refusal notice");
    assert!(
        notice.contains("/go") && notice.contains("/help"),
        "{notice}"
    );
    assert!(notice.contains("built-in"), "{notice}");
    assert!(!notice.contains("/ship"), "{notice}");
    assert!(
        expand(&dir, "/go now").is_none(),
        "built-ins are never expanded"
    );
}

#[test]
fn symlinked_and_oversized_files_are_refused() {
    let root = Temp::new();
    let dir = commands_in(root.path());
    let secret = root.path().join("secret.txt");
    fs::write(&secret, "SECRET").unwrap();
    symlink(&secret, dir.join("link.md")).unwrap();
    fs::write(dir.join("big.md"), vec![b'a'; MAX_BYTES as usize + 1]).unwrap();
    fs::write(dir.join("edge.md"), vec![b'a'; MAX_BYTES as usize]).unwrap();
    fs::write(dir.join("bad name.md"), "x").unwrap();
    fs::write(dir.join("binary.md"), [0xff, 0xfe]).unwrap();
    fs::write(dir.join("empty.md"), "  \n").unwrap();
    let found = scan(&dir);
    assert_eq!(found.names, ["/edge"], "exactly 64 KiB is accepted");
    let notice = found.notice().unwrap();
    for refused in ["/link", "/big", "/binary", "/empty", "bad name"] {
        assert!(notice.contains(refused), "{refused} in {notice}");
    }
    assert!(!notice.contains("SECRET"), "{notice}");
    assert!(matches!(expand(&dir, "/link"), Some(Err(_))));
    assert!(matches!(expand(&dir, "/big"), Some(Err(_))));
}

#[test]
fn a_file_swapped_for_a_symlink_after_the_scan_is_refused_when_run() {
    let root = Temp::new();
    let dir = commands_in(root.path());
    fs::write(dir.join("swap.md"), "fine").unwrap();
    assert_eq!(scan(&dir).names, ["/swap"]);
    fs::remove_file(dir.join("swap.md")).unwrap();
    let secret = root.path().join("secret.txt");
    fs::write(&secret, "SECRET").unwrap();
    symlink(&secret, dir.join("swap.md")).unwrap();
    let error = expand(&dir, "/swap").unwrap().unwrap_err();
    assert!(
        error.contains("/swap") && error.contains("symbolic link"),
        "{error}"
    );
    assert!(!error.contains("SECRET"));
}

#[test]
fn a_symlinked_commands_directory_is_not_followed() {
    let root = Temp::new();
    let real = root.path().join("elsewhere");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("x.md"), "body").unwrap();
    let dir = directory(root.path());
    symlink(&real, &dir).unwrap();
    let found = scan(&dir);
    assert!(found.names.is_empty());
    assert!(found.notice().unwrap().contains("symbolic link"));
    // Enter does not need the listing: a linked directory must not leak its files either.
    assert!(
        expand(&dir, "/x hi").is_none(),
        "nothing is read through a linked directory"
    );
}

#[test]
fn a_commands_directory_linked_into_a_repository_sends_nothing() {
    let root = Temp::new();
    let repo = root.path().join("repo/.alfredo/commands");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("leak.md"), "PLANTED $ARGUMENTS").unwrap();
    let dir = directory(&root.path().join("state"));
    fs::create_dir_all(dir.parent().unwrap()).unwrap();
    symlink(&repo, &dir).unwrap();
    assert!(scan(&dir).names.is_empty());
    assert!(expand(&dir, "/leak hi").is_none());
}

fn staged(typed: &str) -> Session {
    let mut session = Session::new("fixture".into());
    session.insert(typed);
    session
}

#[test]
fn stage_puts_the_expanded_prompt_in_the_draft() {
    let mut session = staged("/fix it");
    stage(&mut session, "/fix", "/fix it", "Fix it now", false).unwrap();
    assert_eq!(session.draft, "Fix it now");
    let sent = send(&mut session, "/fix it").unwrap();
    assert_eq!(sent.last().unwrap().role, "user");
    assert_eq!(sent.last().unwrap().content, "Fix it now");
    assert!(session.draft.is_empty());
}

#[test]
fn stage_refuses_while_a_command_is_pending_and_keeps_the_typed_command() {
    let mut session = staged("/fix it");
    let error = stage(&mut session, "/fix", "/fix it", "Fix it now", true).unwrap_err();
    assert!(error.contains("pending command"), "{error}");
    assert_eq!(session.draft, "/fix it");
}

#[test]
fn stage_refuses_while_a_reply_runs_and_keeps_the_typed_command() {
    for status in [Status::Connecting, Status::Streaming] {
        let mut session = staged("/fix it");
        session.status = status;
        let error = stage(&mut session, "/fix", "/fix it", "Fix it now", false).unwrap_err();
        assert!(error.contains("reply to end"), "{error}");
        assert_eq!(session.draft, "/fix it");
    }
    let mut session = staged("/fix it");
    session.status = Status::Complete;
    assert!(stage(&mut session, "/fix", "/fix it", "Fix it now", false).is_ok());
}

#[test]
fn stage_refuses_a_prompt_over_the_draft_limit_and_restores_the_typed_command() {
    let mut session = staged("/fix it");
    let over = "a".repeat(MAX_DRAFT + 1);
    let error = stage(&mut session, "/fix", "/fix it", &over, false).unwrap_err();
    assert!(
        error.contains("/fix") && error.contains("16 KiB"),
        "{error}"
    );
    assert_eq!(session.draft, "/fix it");
    assert!(session.replace_pending(), "shown as a rejected draft");
    let exact = "a".repeat(MAX_DRAFT);
    let mut session = staged("/fix it");
    stage(&mut session, "/fix", "/fix it", &exact, false).unwrap();
    assert_eq!(session.draft.len(), MAX_DRAFT);
}

#[test]
fn a_send_the_session_refuses_brings_the_typed_command_back() {
    let mut session = Session::new("fixture".into());
    assert_eq!(send(&mut session, "/fix it").unwrap_err(), "Enter a prompt");
    assert_eq!(session.draft, "/fix it");
    assert!(session.replace_pending());
}

#[test]
fn arguments_replace_every_dollar_arguments_once() {
    let root = Temp::new();
    let dir = commands_in(root.path());
    fs::write(
        dir.join("fix.md"),
        "Fix $ARGUMENTS now.\nThen test $ARGUMENTS.\n",
    )
    .unwrap();
    assert_eq!(
        expand(&dir, "/fix  the  parser ").unwrap().unwrap(),
        "Fix the  parser now.\nThen test the  parser."
    );
    assert_eq!(
        expand(&dir, "/fix $ARGUMENTS").unwrap().unwrap(),
        "Fix $ARGUMENTS now.\nThen test $ARGUMENTS."
    );
    assert_eq!(
        expand(&dir, "/fix").unwrap().unwrap(),
        "Fix  now.\nThen test ."
    );
}

#[test]
fn arguments_are_appended_when_the_body_has_no_placeholder() {
    let root = Temp::new();
    let dir = commands_in(root.path());
    fs::write(dir.join("plain.md"), "Review the diff.\n").unwrap();
    assert_eq!(
        expand(&dir, "/plain src/main.rs").unwrap().unwrap(),
        "Review the diff.\n\nARGUMENTS: src/main.rs"
    );
    assert_eq!(expand(&dir, "/plain").unwrap().unwrap(), "Review the diff.");
}

#[test]
fn only_exact_command_words_expand() {
    let root = Temp::new();
    let dir = commands_in(root.path());
    fs::write(dir.join("fix.md"), "body").unwrap();
    assert!(expand(&dir, "/fixer x").is_none());
    assert!(expand(&dir, "/fi").is_none());
    assert!(expand(&dir, "/../fix").is_none());
    assert!(expand(&dir, "fix").is_none());
    assert!(expand(&dir, "/missing").is_none());
    assert!(expand(&dir, "/fix").is_some());
}

#[test]
fn f1_and_the_type_ahead_list_user_commands_marked_as_such() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = Temp::new();
    let dir = commands_in(root.path());
    fs::write(dir.join("shipit.md"), "body").unwrap();
    publish(&scan(&dir));
    let ahead = Completion::typeahead("/shi").unwrap();
    let choice = ahead.choices.iter().find(|c| c.name == "/shipit").unwrap();
    assert!(
        choice.description.starts_with("user command"),
        "{}",
        choice.description
    );
    assert_eq!(ahead.draft(), "/shipit ");
    let all = Completion::all();
    let listed = all.choices.iter().find(|c| c.name == "/shipit").unwrap();
    assert_eq!(listed.group, Some("User commands"));
    assert!(listed.description.starts_with("user command"));
    assert_eq!(
        all.choices.iter().filter(|c| c.name == "/shipit").count(),
        1
    );
    let builtin = all.choices.iter().find(|c| c.name == "/go").unwrap();
    assert!(!builtin.description.starts_with("user command"));
    publish(&scan(&dir.join("gone")));
    assert!(
        Completion::typeahead("/shi").is_none(),
        "a removed command is unlisted"
    );
}

#[test]
fn the_size_cap_is_exactly_65536_bytes() {
    assert_eq!(MAX_BYTES, 65_536);
    let root = Temp::new();
    let dir = commands_in(root.path());
    fs::write(dir.join("full.md"), vec![b'a'; 65_536]).unwrap();
    fs::write(dir.join("over.md"), vec![b'a'; 65_537]).unwrap();
    let found = scan(&dir);
    assert_eq!(found.names, ["/full"]);
    assert_eq!(
        found.refused,
        [("/over".to_string(), "over 64 KiB".to_string())]
    );
    assert_eq!(expand(&dir, "/full").unwrap().unwrap().len(), 65_536);
    assert_eq!(
        expand(&dir, "/over").unwrap().unwrap_err(),
        "/over refused: over 64 KiB"
    );
}

#[test]
fn reload_publishes_the_listing_and_returns_the_refusal_notice() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = Temp::new();
    let dir = commands_in(root.path());
    fs::write(dir.join("ok.md"), "hello").unwrap();
    assert_eq!(reload(&dir), None);
    assert_eq!(listed(), ["/ok"]);
    fs::write(dir.join("help.md"), "x").unwrap();
    fs::write(dir.join("empty.md"), "").unwrap();
    let notice = reload(&dir).unwrap();
    assert_eq!(
        notice,
        "Refused user commands: /empty (empty), /help (clashes with a built-in command)"
    );
    assert_eq!(listed(), ["/ok"]);
    publish(&scan(&root.path().join("missing")));
}

#[test]
fn claims_needs_no_agent_view_and_an_existing_command_file() {
    let root = Temp::new();
    let dir = commands_in(root.path());
    fs::write(dir.join("fix.md"), "Fix $ARGUMENTS").unwrap();
    assert!(claims(false, &dir, "/fix it"));
    assert!(!claims(true, &dir, "/fix it"), "an open agent view wins");
    assert!(!claims(false, &dir, "/nothere"));
    assert!(!claims(true, &dir, "/nothere"));
    assert!(!claims(false, &dir, "plain text"));
    assert!(!claims(false, &dir, "/model"), "built-ins are not claimed");
    // A refused file is still claimed so the owner sees the reason.
    fs::write(dir.join("big.md"), vec![b'a'; 65_537]).unwrap();
    assert!(claims(false, &dir, "/big"));
}

#[test]
fn resolve_returns_the_prompt_the_refusal_or_no_longer_available() {
    let root = Temp::new();
    let dir = commands_in(root.path());
    fs::write(dir.join("fix.md"), "Fix $ARGUMENTS").unwrap();
    fs::write(dir.join("big.md"), vec![b'a'; 65_537]).unwrap();
    assert_eq!(
        resolve(&dir, "  /fix the bug \n"),
        Resolved {
            typed: "/fix the bug".into(),
            name: "/fix".into(),
            prompt: Ok("Fix the bug".into()),
        }
    );
    assert_eq!(
        resolve(&dir, "/big").prompt,
        Err("/big refused: over 64 KiB".into())
    );
    // Deleted between the claim and the resolve.
    assert_eq!(
        resolve(&dir, "/gone x"),
        Resolved {
            typed: "/gone x".into(),
            name: "/gone".into(),
            prompt: Err("/gone is no longer available".into()),
        }
    );
}

#[test]
fn a_pending_command_or_active_scope_blocks_a_user_command() {
    assert!(!is_blocked(false, false));
    assert!(is_blocked(true, false));
    assert!(is_blocked(false, true));
    assert!(is_blocked(true, true));
}

#[test]
fn summarising_runs_only_when_due_and_the_send_waits_only_if_it_started() {
    let mut calls = 0;
    assert!(!summarise_first(false, || {
        calls += 1;
        true
    }));
    assert_eq!(calls, 0, "not due: nothing is spawned");
    assert!(summarise_first(true, || {
        calls += 1;
        true
    }));
    assert_eq!(calls, 1);
    assert!(!summarise_first(true, || {
        calls += 1;
        false
    }));
    assert_eq!(calls, 2, "a failed spawn sends straight away");
}

#[test]
fn the_sent_notice_names_the_command_or_the_refusal() {
    assert_eq!(
        sent_notice("/fix", &Ok(Vec::new())),
        "/fix sent as your message"
    );
    assert_eq!(
        sent_notice("/fix", &Err("Nothing to send")),
        "Nothing to send"
    );
}
