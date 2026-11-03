//! Issue #581: the pre-run secret scan binds an unreadable non-secret directory instead of
//! refusing, still refuses an unreadable workspace root, and names the top-level directory
//! with the most entries when the entry limit is passed.
use alfredo_tui::chat_tools::secret_masks::{scan, scan_with};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static ID: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-scan-{tag}-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn is_root() -> bool {
    let probe = scratch("probe");
    let dir = probe.join("d");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read_dir(&dir).is_ok();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    let _ = fs::remove_dir_all(probe);
    readable
}

fn files(root: &Path, dir: &str, count: usize) {
    fs::create_dir_all(root.join(dir)).unwrap();
    for index in 0..count {
        fs::write(root.join(dir).join(format!("f{index}")), "").unwrap();
    }
}

/// A tree whose entries are all counted exactly once: 40 root files, then `big` and `small`.
fn tree(tag: &str, big: usize, small: usize) -> (PathBuf, usize) {
    let root = scratch(tag);
    files(&root, ".", 40);
    files(&root, "big", big);
    files(&root, "small", small);
    (root, 40 + 2 + big + small)
}

#[test]
fn an_unreadable_nested_directory_is_masked_not_refused() {
    if is_root() {
        eprintln!("skipped: root reads every directory");
        return;
    }
    let root = scratch("nested");
    files(&root, "data/pg", 1);
    let pg = root.join("data/pg");
    fs::set_permissions(&pg, fs::Permissions::from_mode(0o000)).unwrap();
    let result = scan(&root);
    fs::set_permissions(&pg, fs::Permissions::from_mode(0o755)).unwrap();
    let masks = result.unwrap();
    assert_eq!(masks.dirs, vec![pg.to_string_lossy().into_owned()]);
    assert!(masks.files.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_unreadable_workspace_root_still_refuses() {
    if is_root() {
        eprintln!("skipped: root reads every directory");
        return;
    }
    let root = scratch("root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();
    let result = scan(&root);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    let error = result.unwrap_err();
    assert!(
        error.contains("the Coding Workspace could not be listed"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_root_that_cannot_be_listed_for_another_reason_still_refuses() {
    let root = scratch("missing");
    let error = scan(&root.join("absent")).unwrap_err();
    assert!(
        error.contains("the Coding Workspace could not be listed"),
        "{error}"
    );
    let file = root.join("plain");
    fs::write(&file, "").unwrap();
    assert!(scan(&file).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_entry_limit_refusal_names_the_top_level_directory_with_the_most_entries() {
    let (root, total) = tree("biggest", 9, 3);
    assert!(scan_with(&root, total).is_ok());
    let error = scan_with(&root, total - 1).unwrap_err();
    assert!(
        error.contains(&format!("more than {} entries to scan", total - 1)),
        "{error}"
    );
    assert!(error.contains("is big (9 entries)"), "{error}");
    assert!(!error.contains("escalate"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_named_directory_follows_the_counts_not_the_names() {
    let (root, total) = tree("flipped", 2, 7);
    let error = scan_with(&root, total - 1).unwrap_err();
    assert!(error.contains("is small (7 entries)"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn equal_top_level_directories_name_the_first_by_name() {
    let (root, total) = tree("tie", 4, 4);
    let error = scan_with(&root, total - 1).unwrap_err();
    assert!(error.contains("is big (4 entries)"), "{error}");
    let _ = fs::remove_dir_all(root);
}
