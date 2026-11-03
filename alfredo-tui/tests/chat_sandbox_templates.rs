//! Issue #598: the sandbox binds the three env template names read-only instead of masking
//! them. These scan-level tests sit here, not in `secret_masks.rs`, so the mechanical review
//! runs them at the PR head (it only runs the tests under `tests/`).
use alfredo_tui::chat_tools::secret_masks::{scan, TEMPLATE_LIMIT};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static SCRATCH: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-masks-{tag}-{}-{}",
        std::process::id(),
        SCRATCH.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn env_templates_are_bound_read_only_and_are_not_masks() {
    let root = scratch("templates");
    fs::create_dir_all(root.join("sub/.env.example")).unwrap();
    fs::write(root.join("sub/.env.example/inner"), "x").unwrap();
    for name in [".env.example", ".env.sample", ".env.template"] {
        fs::write(root.join(name), "A=1").unwrap();
    }
    fs::write(root.join(".env.example.local"), "x").unwrap();
    fs::write(root.join(".env"), "x").unwrap();
    let masks = scan(&root).unwrap();
    let mut templates = masks.templates.clone();
    templates.sort();
    assert_eq!(
        templates,
        vec![
            text(&root.join(".env.example")),
            text(&root.join(".env.sample")),
            text(&root.join(".env.template")),
        ]
    );
    let mut files = masks.files.clone();
    files.sort();
    assert_eq!(
        files,
        vec![
            text(&root.join(".env")),
            text(&root.join(".env.example.local"))
        ]
    );
    assert_eq!(masks.dirs, vec![text(&root.join("sub/.env.example"))]);
    assert_eq!(masks.len(), 3);
    let mounts = masks.mounts().unwrap();
    let own = text(&root.join(".env.sample"));
    assert!(mounts.pairs.contains(&(own.clone(), own)));
    assert_eq!(mounts.pairs.len(), 6);
    let first_mask = mounts.pairs.iter().position(|(s, d)| s != d).unwrap();
    assert!(
        mounts.pairs[..first_mask].iter().all(|(s, d)| s == d) && first_mask == 3,
        "template binds come before every mask: {:?}",
        mounts.pairs
    );
    assert!(mounts.pairs[first_mask..].iter().all(|(s, d)| s != d));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_template_that_is_not_a_regular_file_is_masked_or_refused() {
    let root = scratch("template-kinds");
    let fifo = root.join(".env.example");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    let masks = scan(&root).unwrap();
    assert!(masks.templates.is_empty());
    assert_eq!(masks.files, vec![text(&fifo)]);
    fs::remove_file(&fifo).unwrap();
    std::os::unix::fs::symlink(".env", root.join(".env.example")).unwrap();
    let error = scan(&root).unwrap_err();
    assert!(error.contains(".env.example is a symbolic link"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn templates_past_the_bind_budget_are_masked_and_then_count() {
    let root = scratch("template-budget");
    for index in 0..TEMPLATE_LIMIT + 2 {
        fs::create_dir_all(root.join(format!("d{index}"))).unwrap();
        fs::write(root.join(format!("d{index}/.env.example")), "A=1").unwrap();
    }
    let masks = scan(&root).unwrap();
    assert_eq!(masks.templates.len(), TEMPLATE_LIMIT);
    assert_eq!(masks.files.len(), 2);
    assert_eq!(masks.len(), 2);
    let _ = fs::remove_dir_all(root);
}
