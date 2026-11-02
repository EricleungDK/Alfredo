//! Issue #486: user-defined slash commands load only from `<state dir>/commands`.
use alfredo_tui::{
    commands::Completion,
    user_commands::{directory, expand, publish, scan, MAX_BYTES},
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
    assert!(expand(&dir, "/link").is_none());
    assert!(matches!(expand(&dir, "/big"), None | Some(Err(_))));
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
